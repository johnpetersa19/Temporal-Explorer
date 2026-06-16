/* window.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Main application window — orchestrator module.
//!
//! | Responsibility              | Module                        |
//! |-----------------------------|-------------------------------|
//! | List-view widget building   | [`crate::views::list_view`]   |
//! | Grid-view widget building   | [`crate::views::grid_view`]   |
//! | Address-bar rebuilding      | [`crate::address_bar`]        |
//! | Commit list / search        | [`crate::commit_controller`]  |
//! | File content preview dialog | [`crate::file_preview`]       |
//! | Git history / tree reads    | [`crate::git_engine`]         |
//! | Timeline grouping logic     | [`crate::timeline_filter`]    |
//! | New-branch dialog           | [`crate::new_branch_dialog`]  |
//!
//! ## Blueprint ↔ Rust contract
//!
//! Every widget with an `id` in `window.blp` **must** appear as a
//! `#[template_child]` in `imp::TemporalExplorerWindow`.  The only
//! exception is `SearchFilterPopover`, created at runtime and stored
//! as `RefCell<Option<…>>` because its parent (`filter_button`) is
//! not known at template-inflate time.
//!
//! ### Panel widgets declared in `.blp` and wired here
//!
//! | Widget id             | Blueprint file   | Rust field             |
//! |-----------------------|------------------|------------------------|
//! | `filter_button`       | `window.blp`     | `#[template_child]`    |
//! | `commit_search_entry` | `window.blp`     | `#[template_child]`    |
//! | `timeline_stack`      | `window.blp`     | `#[template_child]`    |
//! | `right_panel_stack`   | `window.blp`     | `#[template_child]`    |
//! | `commit_info_bar`     | `window.blp`     | `#[template_child]`    |
//! | `toolbar`             | `window.blp`     | `#[template_child]`    |
//! | `new_branch_button`   | `toolbar.blp`    | via `TemporalToolbar`  |

use adw::prelude::{AdwDialogExt, AlertDialogExt};
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use gtk::prelude::*;
use glib::object::ObjectExt;
use gettextrs::gettext;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use crate::address_bar;
use crate::git_engine::{CommitInfo, DirCache, HistoryReader, SnapshotResolver, TreeNode};
use crate::commit_controller;
use crate::file_preview;
use crate::new_branch_dialog::NewBranchDialog;
use crate::search_filter_popover::{SearchFilterPopover, FilterState};
use crate::timeline_filter;
use crate::views::{list_view, grid_view};
use crate::views::list_view::{OnEnterDir, OnOpenFile};
use crate::view_controls::FileSortMode;
use crate::column_chooser::{ColumnChooser, ColumnVisibility};
use crate::batch_operations_dialog::{BatchOperationsDialog, BatchOp};
use crate::select_commits_by_pattern::{SelectCommitsByPattern, commit_matches_pattern};
use crate::merge_conflict_dialog::{MergeConflictDialog, ConflictInfo};
use crate::filter_types_dialog::FilterTypesDialog;
use crate::toolbar::TemporalToolbar;

// ── ViewMode ───────────────────────────────────────────────────────────────────

/// Whether the right panel renders files as a list or a grid.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ViewMode { #[default] List, Grid }

// ── TimelineLevel ──────────────────────────────────────────────────────────────

/// The currently visible drill-down level of the left timeline panel.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum TimelineLevel {
    #[default] Years,
    Months,
    Commits,
}

// ── DebugRepository ────────────────────────────────────────────────────────────

/// Newtype wrapper that makes `git2::Repository` implement `Debug`.
pub struct DebugRepository(pub git2::Repository);

impl std::fmt::Debug for DebugRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Repository").field(&"<git2::Repository>").finish()
    }
}

impl std::ops::Deref for DebugRepository {
    type Target = git2::Repository;
    fn deref(&self) -> &Self::Target { &self.0 }
}

// ── Private implementation ─────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/window.ui")]
    pub struct TemporalExplorerWindow {
        // ── Toolbar / title ──────────────────────────────────────────────────
        #[template_child] pub toolbar:               TemplateChild<TemporalToolbar>,
        #[template_child] pub window_title:          TemplateChild<adw::WindowTitle>,

        // ── Timeline panel ───────────────────────────────────────────────────
        #[template_child] pub timeline_stack:        TemplateChild<gtk::Stack>,
        #[template_child] pub timeline_back_button:  TemplateChild<gtk::Button>,
        #[template_child] pub timeline_header_title: TemplateChild<adw::WindowTitle>,
        #[template_child] pub year_list:             TemplateChild<gtk::ListBox>,
        #[template_child] pub month_list:            TemplateChild<gtk::ListBox>,
        #[template_child] pub commit_search_entry:   TemplateChild<gtk::SearchEntry>,

        // ── Filter button (declared in window.blp; popover wired at runtime) ─
        #[template_child] pub filter_button:         TemplateChild<gtk::ToggleButton>,

        #[template_child] pub commit_list:           TemplateChild<gtk::ListBox>,

        // SearchFilterPopover cannot be a TemplateChild — its type is not
        // registered in Blueprint at template-inflate time, so it is created
        // in setup_filter_popover() and stored here.
        pub filter_popover: RefCell<Option<SearchFilterPopover>>,

        // ── Right panel ──────────────────────────────────────────────────────
        #[template_child] pub content_toolbar_view:  TemplateChild<adw::ToolbarView>,
        #[template_child] pub right_panel_stack:     TemplateChild<gtk::Stack>,
        #[template_child] pub right_panel_content:   TemplateChild<gtk::Box>,
        #[template_child] pub empty_state:           TemplateChild<adw::StatusPage>,
        #[template_child] pub split_view:            TemplateChild<adw::OverlaySplitView>,

        // ── Commit info bar ──────────────────────────────────────────────────
        #[template_child] pub commit_info_bar:       TemplateChild<gtk::ActionBar>,
        #[template_child] pub commit_hash_label:     TemplateChild<gtk::Label>,
        #[template_child] pub commit_message_label:  TemplateChild<gtk::Label>,
        #[template_child] pub commit_date_label:     TemplateChild<gtk::Label>,

        // ── Runtime state ────────────────────────────────────────────────────
        pub all_commits:      RefCell<Vec<CommitInfo>>,
        pub repo_path:        RefCell<Option<PathBuf>>,
        pub repository:       RefCell<Option<DebugRepository>>,
        pub last_query:       RefCell<String>,
        pub current_hash:     RefCell<Option<String>>,
        pub current_dir:      RefCell<PathBuf>,
        pub history_back:     RefCell<Vec<PathBuf>>,
        pub history_forward:  RefCell<Vec<PathBuf>>,
        pub view_mode:        RefCell<ViewMode>,
        pub repo_name:        RefCell<String>,

        // ── Commit navigation history ────────────────────────────────────────
        pub commit_nav_back:    RefCell<Vec<String>>,
        pub commit_nav_forward: RefCell<Vec<String>>,

        pub sort_mode:          RefCell<FileSortMode>,
        pub column_visibility:  RefCell<ColumnVisibility>,

        pub timeline_level:   RefCell<TimelineLevel>,
        pub selected_year:    Cell<i32>,
        pub loading_commits:  Cell<bool>,
        pub dir_cache:        RefCell<DirCache>,
        pub search_debounce:  RefCell<Option<Arc<AtomicBool>>>,
        pub search_cancel:    RefCell<Option<Arc<AtomicBool>>>,

        pub filter_state:     RefCell<FilterState>,
        pub load_cancel:      RefCell<Option<Arc<AtomicBool>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TemporalExplorerWindow {
        const NAME: &'static str = "TemporalExplorerWindow";
        type Type = super::TemporalExplorerWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            TemporalToolbar::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for TemporalExplorerWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_callbacks();
            self.obj().setup_styles();
        }
    }

    impl WidgetImpl for TemporalExplorerWindow {}
    impl WindowImpl for TemporalExplorerWindow {}
    impl ApplicationWindowImpl for TemporalExplorerWindow {}
    impl AdwApplicationWindowImpl for TemporalExplorerWindow {}
}

// ── Public wrapper ─────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct TemporalExplorerWindow(ObjectSubclass<imp::TemporalExplorerWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements
            gio::ActionGroup, gio::ActionMap,
            gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::Root, gtk::ShortcutManager;
}

// ── Free helpers ───────────────────────────────────────────────────────────────

/// Remove all children from a `gtk::Box`.
fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

// ── Calendar / date matching ───────────────────────────────────────────────────

/// Returns `true` if `query` matches any date-related representation of `ts`.
///
/// Recognised formats: ISO date (`2024-03-15`), year-month (`2024-03`),
/// bare year (`2024`), full/abbreviated English month name, and the
/// locale-translated month name returned by [`timeline_filter::month_name`].
fn matches_calendar(ts: i64, q: &str) -> bool {
    let Ok(dt) = glib::DateTime::from_unix_local(ts) else { return false };

    let year  = dt.year();
    let month = dt.month() as u32;
    let day   = dt.day_of_month();

    let iso_date   = format!("{:04}-{:02}-{:02}", year, month, day);
    let year_month = format!("{:04}-{:02}", year, month);
    let year_str   = format!("{:04}", year);
    let human = dt.format("%Y-%m-%d %H:%M")
        .map(|s| s.to_string())
        .unwrap_or_default();

    let month_full = match month {
        1  => "january",   2  => "february", 3  => "march",
        4  => "april",     5  => "may",       6  => "june",
        7  => "july",      8  => "august",    9  => "september",
        10 => "october",   11 => "november",  12 => "december",
        _  => "",
    };
    let month_abbr       = &month_full[..3.min(month_full.len())];
    let month_translated = timeline_filter::month_name(month).to_lowercase();

    iso_date.contains(q)
        || year_month.contains(q)
        || year_str == q
        || human.contains(q)
        || month_full.contains(q)
        || month_abbr == q
        || month_translated.contains(q)
}

// ── Short-hash helpers ─────────────────────────────────────────────────────────

/// Returns the short (7-char) prefix of a full commit hash.
#[inline]
fn short_hash(hash: &str) -> &str {
    &hash[..7.min(hash.len())]
}

/// Returns the first 8 characters of a full commit hash (used in the info bar).
#[inline]
fn display_hash(hash: &str) -> &str {
    &hash[..8.min(hash.len())]
}

// ── TemporalExplorerWindow impl ────────────────────────────────────────────────

impl TemporalExplorerWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    // ── Callback wiring ────────────────────────────────────────────────────────

    fn setup_callbacks(&self) {
        let imp = self.imp();

        let win = self.clone();
        imp.toolbar.open_repo_button().connect_clicked(move |_| { win.open_repo_dialog(); });

        // Keep show_sidebar_button in sync with the split-view's show-sidebar property.
        imp.toolbar.show_sidebar_button().bind_property("active", &imp.split_view.get(), "show-sidebar")
            .bidirectional()
            .sync_create()
            .build();

        // new_branch_button fires win.new-branch via action-name in Blueprint;
        // start insensitive until a repository is loaded.
        imp.toolbar.new_branch_button().set_sensitive(false);

        let win_g = self.clone();
        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        gesture.connect_pressed(move |_, _n, _, _| { win_g.enter_location_mode(); });
        imp.toolbar.address_bar().add_controller(gesture);

        let win = self.clone();
        imp.toolbar.location_entry().connect_activate(move |entry| {
            win.navigate_to_typed_path(entry.text().as_str());
        });

        let win = self.clone();
        imp.toolbar.location_cancel_btn().connect_clicked(move |_| { win.leave_location_mode(); });

        let win = self.clone();
        imp.timeline_back_button.connect_clicked(move |_| { win.timeline_pop(); });

        let win = self.clone();
        imp.year_list.connect_row_activated(move |_, row| {
            let year = unsafe {
                row.data::<i32>("year")
                    .map(|p| *p.as_ref())
                    .unwrap_or(row.index())
            };
            win.on_year_selected(year);
        });

        let win = self.clone();
        imp.month_list.connect_row_activated(move |_, row| {
            if let Some(m) = unsafe { row.data::<u32>("month") } {
                win.on_month_selected(unsafe { *m.as_ref() });
            }
        });

        let win = self.clone();
        imp.commit_list.connect_row_activated(move |_, row| {
            if let Some(h) = unsafe { row.data::<String>("hash") } {
                win.on_commit_selected(unsafe { h.as_ref().clone() });
            }
        });

        let win = self.clone();
        imp.commit_search_entry.connect_search_changed(move |entry| {
            win.on_search_changed(entry.text().to_string());
        });

        self.setup_filter_popover();
        self.setup_history_controls();
        self.setup_view_controls();
        self.setup_actions();
    }

    // ── GAction registration ──────────────────────────────────────────────────

    fn setup_actions(&self) {
        let actions: &[(&str, fn(&TemporalExplorerWindow))] = &[
            ("batch-operations",   Self::show_batch_operations_dialog),
            ("select-by-pattern",  Self::show_select_by_pattern_dialog),
            ("filter-file-type",   Self::show_filter_types_dialog),
            ("show-column-chooser", Self::show_column_chooser),
            ("new-branch",         Self::show_new_branch_dialog),
        ];

        for (name, handler) in actions {
            let win = self.clone();
            let h = *handler;
            let action = gio::SimpleAction::new(name, None);
            action.connect_activate(move |_, _| h(&win));
            self.add_action(&action);
        }
    }

    // ── NewBranchDialog ───────────────────────────────────────────────────────

    fn show_new_branch_dialog(&self) {
        let dialog = NewBranchDialog::new();
        let win = self.clone();
        dialog.connect_branch_created(move |_, name| { win.create_branch(name); });
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    /// Create a local branch at HEAD of the currently loaded repository.
    ///
    /// # Borrow-checker / lifetime rationale
    ///
    /// `git2::Branch<'repo>` borrows from the `Repository` it was created on.
    /// When `repo` is a local variable owned by a closure passed to `and_then`,
    /// returning the `Branch` from that closure is illegal (E0515) because the
    /// borrow of `repo` would escape the scope in which `repo` is owned.
    ///
    /// ## Fix
    ///
    /// Two complementary techniques are used here:
    ///
    /// 1. **Resolve HEAD OID in a tightly-scoped block** — the `RefCell` borrow
    ///    on `self.imp().repository` is released before we do anything else,
    ///    so no live borrow is held across later fallible operations.
    ///
    /// 2. **Map `Branch` to `()` inside the closure** — `repo.branch(…)` returns
    ///    `Result<Branch<'_>, git2::Error>`.  Calling `.map(|_| ())` immediately
    ///    drops the `Branch` (and its borrow of `repo`) *within* the closure,
    ///    so the closure returns `Result<(), String>` — a type with no lifetime
    ///    parameters — which can be safely returned to the outer scope.
    pub fn create_branch(&self, name: &str) {
        // ── 1. Resolve repo_path and HEAD OID — borrow ends here ─────────────
        let (repo_path, head_oid) = {
            let guard = self.imp().repository.borrow();
            let Some(ref repo) = *guard else {
                self.show_error(&gettext("No repository loaded."));
                return;
            };

            let head = match repo.head() {
                Ok(h) => h,
                Err(e) => {
                    self.show_error(&format!("{}: {e}", gettext("Cannot read HEAD")));
                    return;
                }
            };

            let oid = match head.peel_to_commit() {
                Ok(c) => c.id(),
                Err(e) => {
                    self.show_error(&format!("{}: {e}", gettext("Cannot peel HEAD to commit")));
                    return;
                }
            };

            // Clone the path so the borrow on `self.imp().repository` can end.
            let path = self.imp().repo_path.borrow().clone();
            (path, oid)
        };
        // repo_guard is dropped here; no borrows of `self.imp()` are live.

        // ── 2. Resolve the repo_path ──────────────────────────────────────────
        let Some(path) = repo_path else {
            self.show_error(&gettext("No repository path stored."));
            return;
        };

        // ── 3. Open a short-lived handle and create the branch ────────────────
        //
        // A secondary `Repository` handle is opened solely for this operation.
        // Its lifetime — and therefore the lifetime of `Branch<'_>` — is
        // entirely contained within this block, satisfying the borrow checker.
        //
        // `.map(|_| ())` drops the `Branch` (and its borrow of `repo`) inside
        // the closure so `Result<(), String>` — with no lifetime parameter —
        // is what gets returned to the outer scope.  Without this `.map`, the
        // compiler would emit E0515 because the `Branch` borrow would escape
        // the closure that owns `repo`.
        let result = git2::Repository::open(&path)
            .map_err(|e| format!("{}: {e}", gettext("Cannot open repository")))
            .and_then(|repo| {
                let commit = repo
                    .find_commit(head_oid)
                    .map_err(|e| format!("{}: {e}", gettext("Cannot find HEAD commit")))?;
                repo.branch(name, &commit, false)
                    .map(|_| ())   // ← drop Branch<'_> here; borrow of `repo` ends
                    .map_err(|e| format!("{} \u{2018}{}\u{2019}: {e}", gettext("Failed to create branch"), name))
            });

        // ── 4. Report outcome ─────────────────────────────────────────────────
        match result {
            Ok(()) => {
                let toast = adw::Toast::new(&format!(
                    "{} \u{2018}{}\u{2019}",
                    gettext("Created branch"),
                    name,
                ));
                if let Some(overlay) = self.imp().content_toolbar_view
                    .parent()
                    .and_then(|w| w.downcast::<adw::ToastOverlay>().ok())
                {
                    overlay.add_toast(toast);
                }
            }
            Err(msg) => self.show_error(&msg),
        }
    }

    // ── HistoryControls wiring ────────────────────────────────────────────────
    //
    // The toolbar exposes a single HistoryControls widget that serves both
    // dir-nav (back/forward within a snapshot's directory tree) and
    // commit-nav (back/forward across previously visited commits).
    //
    // Dispatch rule (evaluated on every signal emission):
    //   • A snapshot is "open" when current_hash is Some AND current_dir
    //     is non-empty (i.e. the user has descended into at least one
    //     sub-directory).
    //   • While a snapshot is open the signals drive dir-nav so the user
    //     can step back up the directory tree without losing the commit.
    //   • At the root of a snapshot (current_dir is empty) or when no
    //     commit is selected the signals drive commit-nav.
    //
    // BORROW SAFETY: each borrow() guard is extracted into an owned local
    // (bool / clone) and dropped before any method that may re-borrow the
    // same RefCell is called.  This prevents "RefCell already mutably
    // borrowed" panics when navigate_commit_back / navigate_commit_forward
    // write to current_hash or current_dir inside jump_to_commit_hash.
    //
    // CRITICAL: do NOT keep a live `imp` binding across the dispatch call.
    // The glib closure marshaller cannot unwind, so any panic caused by a
    // RefCell double-borrow is immediately fatal (SIGABRT).  All borrow
    // guards must be dropped — and `imp` itself released — before calling
    // navigate_back / navigate_commit_back / navigate_forward /
    // navigate_commit_forward.

    fn setup_history_controls(&self) {
        let win = self.clone();
        self.imp().toolbar.history_controls().connect_local("navigate-back", false, move |_| {
            // ── Step 1: snapshot condition into owned bools ───────────────────
            // All borrow() guards are dropped at the end of this block.
            // `imp` is NOT stored in a let-binding that outlives this block,
            // because doing so would keep an implicit borrow alive across the
            // dispatch call below, which triggers the RefCell panic.
            let (has_hash, in_subdir) = {
                let imp = win.imp();
                let has_hash  = imp.current_hash.borrow().is_some();
                let in_subdir = !imp.current_dir.borrow().as_os_str().is_empty();
                (has_hash, in_subdir)
                // imp, and both borrow() guards, are dropped here
            };

            // ── Step 2: dispatch — no RefCell is borrowed at this point ───────
            if has_hash && in_subdir {
                win.navigate_back();
            } else {
                win.navigate_commit_back();
            }
            None
        });

        let win = self.clone();
        self.imp().toolbar.history_controls().connect_local("navigate-forward", false, move |_| {
            // Same pattern as "navigate-back".
            let (has_hash, in_subdir) = {
                let imp = win.imp();
                let has_hash  = imp.current_hash.borrow().is_some();
                let in_subdir = !imp.current_dir.borrow().as_os_str().is_empty();
                (has_hash, in_subdir)
                // imp, and both borrow() guards, are dropped here
            };

            if has_hash && in_subdir {
                win.navigate_forward();
            } else {
                win.navigate_commit_forward();
            }
            None
        });
    }

    /// Push `hash` onto the commit navigation back-stack and clear the forward stack.
    ///
    /// # Borrow safety
    ///
    /// `current_hash` is read into an owned `Option<String>` before
    /// `commit_nav_forward` is mutably borrowed, so the two RefCells are
    /// never borrowed simultaneously.
    fn push_commit_nav(&self, hash: &str) {
        let imp = self.imp();

        // Read current_hash into an owned value; guard is dropped immediately.
        let prev_hash = imp.current_hash.borrow().clone();

        imp.commit_nav_forward.borrow_mut().clear();
        if let Some(prev) = prev_hash {
            if prev != hash {
                imp.commit_nav_back.borrow_mut().push(prev);
            }
        }
        self.update_commit_nav_buttons();
    }

    /// Navigate back one step in the commit history.
    ///
    /// # Borrow safety
    ///
    /// All RefCell values are extracted into owned locals inside a tightly-scoped
    /// block.  That block ends — and every borrow guard is dropped — **before**
    /// `jump_to_commit_hash` is called.  `jump_to_commit_hash` mutably borrows
    /// `current_hash` and `current_dir` internally; any live `imp` binding at the
    /// call-site would alias those 