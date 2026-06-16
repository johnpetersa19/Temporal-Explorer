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
    // This keeps navigate_back and navigate_forward reachable from
    // live code (silencing the dead_code warning) without changing the
    // existing commit-nav behaviour.

    fn setup_history_controls(&self) {
        let win = self.clone();
        self.imp().toolbar.history_controls().connect_local("navigate-back", false, move |_| {
            let imp = win.imp();
            let in_snapshot = imp.current_hash.borrow().is_some()
                && !imp.current_dir.borrow().as_os_str().is_empty();
            if in_snapshot {
                win.navigate_back();
            } else {
                win.navigate_commit_back();
            }
            None
        });

        let win = self.clone();
        self.imp().toolbar.history_controls().connect_local("navigate-forward", false, move |_| {
            let imp = win.imp();
            let in_snapshot = imp.current_hash.borrow().is_some()
                && !imp.current_dir.borrow().as_os_str().is_empty();
            if in_snapshot {
                win.navigate_forward();
            } else {
                win.navigate_commit_forward();
            }
            None
        });
    }

    /// Push `hash` onto the commit navigation back-stack and clear the forward stack.
    fn push_commit_nav(&self, hash: &str) {
        let imp = self.imp();
        imp.commit_nav_forward.borrow_mut().clear();
        if let Some(prev) = imp.current_hash.borrow().clone() {
            if prev != hash {
                imp.commit_nav_back.borrow_mut().push(prev);
            }
        }
        self.update_commit_nav_buttons();
    }

    fn navigate_commit_back(&self) {
        let imp = self.imp();
        if let Some(prev) = imp.commit_nav_back.borrow_mut().pop() {
            if let Some(current) = imp.current_hash.borrow().clone() {
                imp.commit_nav_forward.borrow_mut().push(current);
            }
            self.jump_to_commit_hash(prev);
        }
    }

    fn navigate_commit_forward(&self) {
        let imp = self.imp();
        if let Some(next) = imp.commit_nav_forward.borrow_mut().pop() {
            if let Some(current) = imp.current_hash.borrow().clone() {
                imp.commit_nav_back.borrow_mut().push(current);
            }
            self.jump_to_commit_hash(next);
        }
    }

    /// Navigate directly to `hash`, resetting dir history and updating the info bar.
    fn jump_to_commit_hash(&self, hash: String) {
        let imp = self.imp();
        *imp.current_hash.borrow_mut() = Some(hash.clone());
        *imp.current_dir.borrow_mut()  = PathBuf::new();
        imp.history_back.borrow_mut().clear();
        imp.history_forward.borrow_mut().clear();

        {
            let commits = imp.all_commits.borrow();
            if let Some(commit) = commits.iter().find(|c| c.hash.starts_with(short_hash(&hash))) {
                imp.commit_hash_label.set_label(display_hash(&commit.hash));
                imp.commit_message_label.set_label(&commit.summary);
                imp.commit_date_label.set_label(&Self::format_timestamp(commit.timestamp));
                imp.commit_info_bar.set_revealed(true);
            }
        }
        self.update_commit_nav_buttons();
        self.navigate_to_dir(PathBuf::new());
    }

    fn update_commit_nav_buttons(&self) {
        let imp = self.imp();
        let can_back    = !imp.commit_nav_back.borrow().is_empty();
        let can_forward = !imp.commit_nav_forward.borrow().is_empty();
        imp.toolbar.history_controls().set_sensitivity(can_back, can_forward);
    }

    // ── ViewControls wiring ───────────────────────────────────────────────────

    fn setup_view_controls(&self) {
        let win = self.clone();
        self.imp().toolbar.view_controls().connect_local("view-mode-changed", false, move |args| {
            let is_grid = args[1].get::<bool>().unwrap_or(false);
            *win.imp().view_mode.borrow_mut() = if is_grid { ViewMode::Grid } else { ViewMode::List };
            let dir = win.imp().current_dir.borrow().clone();
            if win.imp().current_hash.borrow().is_some() {
                win.navigate_to_dir(dir);
            }
            None
        });

        let win = self.clone();
        self.imp().toolbar.view_controls().connect_local("sort-changed", false, move |args| {
            let raw = args[1].get::<u32>().unwrap_or(0);
            let mode = match raw {
                1 => FileSortMode::Status,
                2 => FileSortMode::Extension,
                _ => FileSortMode::Name,
            };
            *win.imp().sort_mode.borrow_mut() = mode;
            let dir = win.imp().current_dir.borrow().clone();
            if win.imp().current_hash.borrow().is_some() {
                win.navigate_to_dir(dir);
            }
            None
        });
    }

    // ── ColumnChooser ─────────────────────────────────────────────────────────

    pub fn show_column_chooser(&self) {
        let dialog = ColumnChooser::new();
        dialog.apply_visibility(&self.imp().column_visibility.borrow());

        let win     = self.clone();
        let dlg_ref = dialog.clone();
        dialog.connect_local("columns-changed", false, move |_| {
            *win.imp().column_visibility.borrow_mut() = dlg_ref.visibility();
            let dir = win.imp().current_dir.borrow().clone();
            if win.imp().current_hash.borrow().is_some() {
                win.navigate_to_dir(dir);
            }
            None
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── BatchOperationsDialog ─────────────────────────────────────────────────

    pub fn show_batch_operations_dialog(&self) {
        let dialog = BatchOperationsDialog::new();
        dialog.set_commits(&self.imp().all_commits.borrow());

        let win = self.clone();
        dialog.connect_operation_requested(move |dlg, op, shas| {
            match op {
                BatchOp::CherryPick { signoff } => {
                    let msg = format!(
                        "{} {} commit(s){}",
                        gettext("Cherry-pick"),
                        shas.len(),
                        if signoff { gettext(" with sign-off") } else { String::new() },
                    );
                    win.show_toast(&msg);
                    dlg.mark_done();
                }
                BatchOp::ExportPatches { dest_dir } => {
                    let shas_clone = shas.clone();
                    let repo_path  = win.imp().repo_path.borrow().clone();
                    let dlg_ref    = dlg.clone();
                    dlg.set_progress_visible(true);

                    let (tx, rx) = std::sync::mpsc::sync_channel::<()>(1);

                    std::thread::spawn(move || {
                        if let Some(repo_path) = repo_path {
                            let _ = std::fs::create_dir_all(&dest_dir);
                            for (i, sha) in shas_clone.iter().enumerate() {
                                let patch_path = dest_dir.join(
                                    format!("{:04}-{}.patch", i + 1, short_hash(sha))
                                );
                                if let Ok(repo) = git2::Repository::open(&repo_path) {
                                    if let Ok(oid) = git2::Oid::from_str(sha) {
                                        if let Ok(commit) = repo.find_commit(oid) {
                                            if let Ok(tree) = commit.tree() {
                                                let parent_tree = commit
                                                    .parent(0).ok()
                                                    .and_then(|p| p.tree().ok());
                                                if let Ok(diff) = repo.diff_tree_to_tree(
                                                    parent_tree.as_ref(),
                                                    Some(&tree),
                                                    None,
                                                ) {
                                                    let mut buf = Vec::new();
                                                    let _ = diff.print(
                                                        git2::DiffFormat::Patch,
                                                        |_d, _h, line| {
                                                            buf.extend_from_slice(line.content());
                                                            true
                                                        },
                                                    );
                                                    let _ = std::fs::write(&patch_path, &buf);
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        let _ = tx.send(());
                    });

                    glib::idle_add_local(move || match rx.try_recv() {
                        Ok(()) => { dlg_ref.mark_done(); glib::ControlFlow::Break }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    });
                }
                BatchOp::CopyShas { short } => {
                    let text = shas
                        .iter()
                        .map(|s| if short { short_hash(s).to_string() } else { s.clone() })
                        .collect::<Vec<_>>()
                        .join("\n");
                    if let Some(display) = gtk::gdk::Display::default() {
                        display.clipboard().set_text(&text);
                    }
                    win.show_toast(&format!("{} {} SHA(s)", gettext("Copied"), shas.len()));
                    dlg.mark_done();
                }
            }
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── SelectCommitsByPattern ────────────────────────────────────────────────

    pub fn show_select_by_pattern_dialog(&self) {
        let dialog = SelectCommitsByPattern::new();
        dialog.set_commits(&self.imp().all_commits.borrow());

        let win = self.clone();
        dialog.connect_pattern_selected(move |_, pattern, mode, icase| {
            let all = win.imp().all_commits.borrow().clone();
            let matching: Vec<String> = all
                .iter()
                .filter(|c| commit_matches_pattern(c, pattern, mode, icase))
                .map(|c| c.hash.clone())
                .collect();

            let list = win.imp().commit_list.clone();

            // Mark matching rows with CSS class "pattern-match".
            let mut row = list.first_child();
            while let Some(r) = row {
                if let Some(list_row) = r.downcast_ref::<gtk::ListBoxRow>() {
                    let hash_opt = unsafe {
                        list_row.data::<String>("hash").map(|p| p.as_ref().clone())
                    };
                    if let Some(hash) = hash_opt {
                        let is_match = matching.iter().any(|m| m.starts_with(short_hash(&hash)));
                        if is_match {
                            list_row.add_css_class("pattern-match");
                        } else {
                            list_row.remove_css_class("pattern-match");
                        }
                    }
                    row = list_row.next_sibling();
                } else {
                    row = r.next_sibling();
                }
            }

            // Scroll to first match.
            let mut row2 = list.first_child();
            while let Some(r) = row2 {
                if let Some(list_row) = r.downcast_ref::<gtk::ListBoxRow>() {
                    if list_row.has_css_class("pattern-match") {
                        list_row.grab_focus();
                        break;
                    }
                    row2 = list_row.next_sibling();
                } else {
                    row2 = r.next_sibling();
                }
            }

            win.show_toast(&format!("{} {} commit(s)", gettext("Selected"), matching.len()));
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── MergeConflictDialog ───────────────────────────────────────────────────

    /// Show the merge-conflict inspector only for merge commits (≥ 2 parents).
    fn try_show_merge_conflict_dialog(&self, hash: &str) {
        let repo_path = match self.imp().repo_path.borrow().clone() {
            Some(p) => p,
            None => return,
        };

        let repo = match git2::Repository::open(&repo_path) {
            Ok(r) => r,
            Err(_) => return,
        };

        let oid = match git2::Oid::from_str(hash) {
            Ok(o) => o,
            Err(_) => return,
        };

        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => return,
        };

        if commit.parent_count() < 2 { return; }

        let ours   = commit.parent(0).ok();
        let theirs = commit.parent(1).ok();

        let conflict_file = ours
            .as_ref()
            .zip(theirs.as_ref())
            .and_then(|(o, t)| {
                let ot = o.tree().ok()?;
                let tt = t.tree().ok()?;
                let diff = repo.diff_tree_to_tree(Some(&ot), Some(&tt), None).ok()?;
                let mut first_path = None;
                diff.foreach(
                    &mut |delta, _| {
                        if first_path.is_none() {
                            if let Some(p) = delta.new_file().path() {
                                first_path = Some(p.to_string_lossy().to_string());
                            }
                        }
                        true
                    },
                    None, None, None,
                ).ok()?;
                first_path
            })
            .unwrap_or_else(|| "(unknown)".to_string());

        let diff_text = ours
            .as_ref()
            .zip(theirs.as_ref())
            .and_then(|(o, t)| {
                let ot = o.tree().ok()?;
                let tt = t.tree().ok()?;
                let mut opts = git2::DiffOptions::new();
                opts.pathspec(&conflict_file);
                let diff = repo.diff_tree_to_tree(Some(&ot), Some(&tt), Some(&mut opts)).ok()?;
                let mut buf = Vec::new();
                diff.print(git2::DiffFormat::Patch, |_d, _h, line| {
                    buf.extend_from_slice(line.content());
                    true
                }).ok()?;
                Some(String::from_utf8_lossy(&buf).to_string())
            })
            .unwrap_or_default();

        let fmt_commit = |c: Option<&git2::Commit>| -> (String, String, String) {
            match c {
                Some(commit) => (
                    commit.id().to_string(),
                    commit.author().name().unwrap_or("").to_string(),
                    glib::DateTime::from_unix_local(commit.time().seconds())
                        .and_then(|d| d.format("%Y-%m-%d %H:%M"))
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                ),
                None => (String::new(), String::new(), String::new()),
            }
        };

        let (ours_sha, ours_author, ours_date)       = fmt_commit(ours.as_ref());
        let (theirs_sha, theirs_author, theirs_date) = fmt_commit(theirs.as_ref());

        let info = ConflictInfo {
            file_path: conflict_file,
            ours_sha, ours_author, ours_date,
            theirs_sha, theirs_author, theirs_date,
            diff_text,
        };

        let dialog = MergeConflictDialog::new();
        dialog.load_conflict(&info);

        let win = self.clone();
        dialog.connect_conflict_resolved(move |_, resolution, file_path, apply_all| {
            let msg = format!(
                "{}: {} \u{2018}{}\u{2019}{}",
                gettext("Conflict resolved"),
                resolution,
                file_path,
                if apply_all { format!(" ({})", gettext("applied to all")) } else { String::new() },
            );
            win.show_toast(&msg);
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── FilterTypesDialog ─────────────────────────────────────────────────────

    pub fn show_filter_types_dialog(&self) {
        let dialog = FilterTypesDialog::new();

        let win = self.clone();
        dialog.connect_file_type_selected(move |_, ext| {
            {
                let mut fs = win.imp().filter_state.borrow_mut();
                fs.files.other_ext = if ext.is_empty() { None } else { Some(ext.to_string()) };
            }
            let q = win.imp().last_query.borrow().clone();
            win.run_search(q);

            // Propagate into SearchFilterPopover when set_file_ext is exposed.
            if let Some(ref pop) = *win.imp().filter_popover.borrow() {
                let _ = pop; // placeholder — call pop.set_file_ext(ext) when exposed
            }
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── CSS loader ────────────────────────────────────────────────────────────

    fn setup_styles(&self) {
        let provider = gtk::CssProvider::new();
        provider.load_from_resource(
            "/io/github/johnpetersa19/TemporalExplorer/temporal-explorer.css",
        );
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display, &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    // ── Repository opening ─────────────────────────────────────────────────────

    fn open_repo_dialog(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Open Repository"))
            .modal(true)
            .build();
        let win = self.clone();
        dialog.select_folder(Some(self), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() { win.load_repository(path); }
            }
        });
    }

    pub fn load_repository(&self, path: PathBuf) {
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(prev) = self.imp().load_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }
        *self.imp().load_cancel.borrow_mut() = Some(cancel.clone());

        match git2::Repository::open(&path) {
            Ok(repo) => {
                let repo_name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("repository")
                    .to_string();

                let imp = self.imp();
                *imp.repo_path.borrow_mut()   = Some(path.clone());
                *imp.repository.borrow_mut()  = Some(DebugRepository(repo));
                *imp.repo_name.borrow_mut()   = repo_name.clone();
                *imp.current_dir.borrow_mut() = PathBuf::new();
                imp.history_back.borrow_mut().clear();
                imp.history_forward.borrow_mut().clear();
                imp.commit_nav_back.borrow_mut().clear();
                imp.commit_nav_forward.borrow_mut().clear();
                imp.toolbar.history_controls().reset();
                imp.window_title.set_title(&repo_name);
                imp.window_title.set_subtitle(path.to_str().unwrap_or(""));
                imp.toolbar.new_branch_button().set_sensitive(true);

                // Populate branch chips in the filter popover.
                if let Some(ref pop) = *imp.filter_popover.borrow() {
                    if let Some(ref repo_wrapper) = *imp.repository.borrow() {
                        let branches: Vec<String> = repo_wrapper
                            .branches(Some(git2::BranchType::Local))
                            .map(|iter| {
                                iter.filter_map(|b| {
                                    b.ok().and_then(|(b, _)| {
                                        b.name().ok().flatten().map(|n| n.to_string())
                                    })
                                }).collect()
                            })
                            .unwrap_or_default();
                        pop.populate_branch_chips(&branches);
                    }
                }

                self.load_timeline(cancel);
            }
            Err(e) => self.show_error(&format!("{}: {e}", gettext("Failed to open repository"))),
        }
    }

    // ── Timeline loading ───────────────────────────────────────────────────────

    /// Feeds new author names into the filter popover, deduplicating against
    /// authors already seen in previous pages.
    ///
    /// Called once per received page so the popover chips grow incrementally
    /// as commits stream in, without re-scanning the entire `all_commits` vec.
    fn update_author_chips_for_page(&self, page: &[CommitInfo]) {
        let imp = self.imp();
        let pop_borrow = imp.filter_popover.borrow();
        let Some(ref pop) = *pop_borrow else { return };

        // Build the set of authors already known to avoid duplicate chips.
        let mut seen: std::collections::HashSet<String> = imp
            .all_commits
            .borrow()
            .iter()
            .map(|c| c.author.clone())
            .collect();

        let new_authors: Vec<String> = page
            .iter()
            .map(|c| c.author.clone())
            .filter(|a| seen.insert(a.clone()))
            .collect();

        if !new_authors.is_empty() {
            pop.populate_author_chips(&new_authors);
        }
    }

    /// Displays an empty-repository state in the timeline sidebar.
    ///
    /// Called when `load_timeline` receives the EOS sentinel but `all_commits`
    /// is still empty — meaning the repository has no commits reachable from
    /// HEAD (freshly initialised repo, unborn branch, or detached HEAD with no
    /// history).  Showing an explicit message is far better than leaving the
    /// sidebar blank and making the user wonder whether loading is still in
    /// progress.
    fn show_empty_repository_state(&self) {
        let imp = self.imp();
        imp.year_list.remove_all();
        *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
        imp.timeline_stack.set_visible_child_name("years");
        imp.timeline_back_button.set_visible(false);
        imp.timeline_header_title.set_title(&gettext("Timeline"));
        imp.timeline_header_title.set_subtitle(&gettext("No commits found"));
        imp.split_view.set_show_sidebar(true);
    }

    fn load_timeline(&self, cancel: Arc<AtomicBool>) {
        // 500 commits per page: fast enough for incremental display,
        // large enough to avoid per-page GTK overhead on large repos.
        const TIMELINE_PAGE_SIZE: usize = 500;

        let repo_path = match self.imp().repo_path.borrow().clone() {
            Some(p) => p,
            None => return,
        };

        self.imp().loading_commits.set(true);
        self.imp().all_commits.borrow_mut().clear();

        // Show the empty state *before* the worker starts so the UI never
        // displays stale content from a previous repository during loading.
        self.show_empty_state();

        // Rendezvous channel (capacity 0): the worker blocks after each page
        // until the GTK main loop consumes it, bounding memory to ~1 page.
        //
        // Protocol:
        //   Ok(non-empty vec)  — one page of CommitInfo; more pages may follow.
        //   Ok(empty vec)      — end-of-stream (EOS) sentinel; no more pages.
        //   Err(msg)           — fatal git2 error; worker has exited.
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<CommitInfo>, String>>(0);

        let cancel_worker = cancel.clone();
        std::thread::spawn(move || {
            let result = HistoryReader::open(&repo_path).and_then(|reader| {
                reader.list_commits_paginated(TIMELINE_PAGE_SIZE, |page| {
                    // Honour cancellation between pages so switching repos is instant.
                    if cancel_worker.load(Ordering::Relaxed) { return; }
                    let _ = tx.send(Ok(page));
                })
            });

            // Send the EOS sentinel (empty vec) on success, or the error string.
            // The idle callback distinguishes them by the is_empty() check.
            match result {
                Ok(()) => { let _ = tx.send(Ok(Vec::new())); }
                Err(e) => { let _ = tx.send(Err(e.to_string())); }
            }
        });

        let win = self.clone();
        glib::idle_add_local(move || {
            // Abort idle callbacks belonging to a superseded load operation.
            if cancel.load(Ordering::Relaxed) {
                return glib::ControlFlow::Break;
            }

            match rx.try_recv() {
                // ── End-of-stream sentinel ────────────────────────────────────
                Ok(Ok(page)) if page.is_empty() => {
                    win.imp().loading_commits.set(false);

                    // Guard: if no commits arrived at all the repository has no
                    // history reachable from HEAD (empty repo, unborn branch, or
                    // detached HEAD with no ancestors).  Show an informative state
                    // instead of silently leaving the sidebar blank.
                    if win.imp().all_commits.borrow().is_empty() {
                        win.show_empty_repository_state();
                    } else {
                        win.imp().split_view.set_show_sidebar(true);
                        win.populate_year_list();
                    }

                    glib::ControlFlow::Break
                }

                // ── Data page: accumulate and refresh timeline ────────────────
                Ok(Ok(page)) => {
                    // Update author chips incrementally — before extending
                    // all_commits so the dedup set reflects the current state.
                    win.update_author_chips_for_page(&page);

                    win.imp().all_commits.borrow_mut().extend(page);
                    win.populate_year_list();
                    win.imp().split_view.set_show_sidebar(true);
                    glib::ControlFlow::Continue
                }

                // ── Fatal error from the worker thread ────────────────────────
                Ok(Err(e)) => {
                    win.imp().loading_commits.set(false);
                    win.show_error(&format!("{}: {e}", gettext("Failed to read history")));
                    glib::ControlFlow::Break
                }

                // ── Worker hasn't sent yet — yield back to the GTK main loop ──
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,

                // ── Channel closed unexpectedly (worker panicked) ─────────────
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    win.imp().loading_commits.set(false);
                    glib::ControlFlow::Break
                }
            }
        });
    }

    // ── Year list ─────────────────────────────────────────────────────────────

    fn populate_year_list(&self) {
        let imp = self.imp();
        imp.year_list.remove_all();

        let commits = imp.all_commits.borrow();
        for (year, count) in &timeline_filter::years_in_range(&commits) {
            imp.year_list.append(&commit_controller::build_year_row(*year, *count));
        }

        *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
        imp.timeline_stack.set_visible_child_name("years");
        imp.timeline_back_button.set_visible(false);
        imp.timeline_header_title.set_title(&gettext("Timeline"));
        imp.timeline_header_title.set_subtitle("");
    }

    // ── Year selected ─────────────────────────────────────────────────────────

    fn on_year_selected(&self, year: i32) {
        let imp = self.imp();
        imp.selected_year.set(year);
        imp.month_list.remove_all();

        let commits = imp.all_commits.borrow();
        for (month, count) in &timeline_filter::months_for_year(&commits, year) {
            imp.month_list.append(&commit_controller::build_month_row(*month, *count));
        }

        *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
        imp.timeline_stack.set_visible_child_name("months");
        imp.timeline_back_button.set_visible(true);
        imp.timeline_header_title.set_title(&year.to_string());
        imp.timeline_header_title.set_subtitle("");
    }

    // ── Month selected ────────────────────────────────────────────────────────

    fn on_month_selected(&self, month: u32) {
        let imp = self.imp();
        let year = imp.selected_year.get();
        imp.commit_list.remove_all();

        let commits  = imp.all_commits.borrow().clone();
        let filtered = timeline_filter::commits_for_month(&commits, year, month);
        commit_controller::populate_commit_list(&imp.commit_list, &filtered);

        *imp.timeline_level.borrow_mut() = TimelineLevel::Commits;
        imp.timeline_stack.set_visible_child_name("commits");
        imp.timeline_header_title.set_subtitle(&format!(
            "{} {}",
            timeline_filter::month_name(month),
            year,
        ));
    }

    // ── Timeline back ─────────────────────────────────────────────────────────

    fn timeline_pop(&self) {
        let imp = self.imp();
        match *imp.timeline_level.borrow() {
            TimelineLevel::Commits => {
                *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
                imp.timeline_stack.set_visible_child_name("months");
                imp.timeline_header_title.set_subtitle("");
            }
            TimelineLevel::Months => {
                *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
                imp.timeline_stack.set_visible_child_name("years");
                imp.timeline_back_button.set_visible(false);
                imp.timeline_header_title.set_title(&gettext("Timeline"));
                imp.timeline_header_title.set_subtitle("");
                imp.selected_year.set(0);
            }
            TimelineLevel::Years => {}
        }
    }

    // ── Commit selected ───────────────────────────────────────────────────────

    fn on_commit_selected(&self, hash: String) {
        let imp = self.imp();

        self.push_commit_nav(&hash);

        *imp.current_hash.borrow_mut() = Some(hash.clone());
        *imp.current_dir.borrow_mut()  = PathBuf::new();
        imp.history_back.borrow_mut().clear();
        imp.history_forward.borrow_mut().clear();

        {
            let commits = imp.all_commits.borrow();
            if let Some(commit) = commits.iter().find(|c| c.hash.starts_with(short_hash(&hash))) {
                imp.commit_hash_label.set_label(display_hash(&commit.hash));
                imp.commit_message_label.set_label(&commit.summary);
                imp.commit_date_label.set_label(&Self::format_timestamp(commit.timestamp));
                imp.commit_info_bar.set_revealed(true);
            }
        }

        self.try_show_merge_conflict_dialog(&hash);
        self.navigate_to_dir(PathBuf::new());
    }

    // ── Directory navigation ──────────────────────────────────────────────────

    pub fn navigate_to_dir(&self, dir: PathBuf) {
        let imp = self.imp();
        let hash      = match imp.current_hash.borrow().clone() { Some(h) => h, None => return };
        let repo_path = match imp.repo_path.borrow().clone()    { Some(p) => p, None => return };
        let repo_name = imp.repo_name.borrow().clone();

        *imp.current_dir.borrow_mut() = dir.clone();

        let win_ab1 = self.clone();
        let win_ab2 = self.clone();
        address_bar::rebuild_address_bar(
            imp.toolbar.address_bar(),
            &repo_name,
            &dir,
            move |path: PathBuf| { win_ab1.push_dir(path); },
            move || { win_ab2.enter_location_mode(); },
        );
        self.update_dir_nav_buttons();

        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(prev) = imp.load_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }
        *imp.load_cancel.borrow_mut() = Some(cancel);

        let dir_clone = dir.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<TreeNode>, String>>(1);
        std::thread::spawn(move || {
            let result = git2::Repository::open(&repo_path)
                .map_err(|e| e.to_string())
                .and_then(|repo| {
                    SnapshotResolver::new(&repo)
                        .resolve_dir(&hash, &dir_clone)
                        .map_err(|e| e.to_string())
                });
            let _ = tx.send(result);
        });

        let win = self.clone();
        glib::idle_add_local(move || match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(nodes) => win.render_dir(nodes),
                    Err(e)    => win.show_error(&format!("{}: {e}", gettext("Error reading tree"))),
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
    }

    fn render_dir(&self, mut nodes: Vec<TreeNode>) {
        let imp       = self.imp();
        let mode      = *imp.view_mode.borrow();
        let sort_mode = *imp.sort_mode.borrow();
        let hash      = imp.current_hash.borrow().clone().unwrap_or_default();

        let get_node_name = |node: &TreeNode| -> String {
            node.path()
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_lowercase()
        };

        match sort_mode {
            FileSortMode::Name | FileSortMode::Status => {
                nodes.sort_by(|a, b| {
                    b.is_dir().cmp(&a.is_dir()).then(get_node_name(a).cmp(&get_node_name(b)))
                });
            }
            FileSortMode::Extension => {
                nodes.sort_by(|a, b| {
                    let name_a = get_node_name(a);
                    let name_b = get_node_name(b);
                    let ext_a = std::path::Path::new(&name_a).extension().and_then(|e| e.to_str()).unwrap_or("");
                    let ext_b = std::path::Path::new(&name_b).extension().and_then(|e| e.to_str()).unwrap_or("");
                    b.is_dir().cmp(&a.is_dir())
                        .then(ext_a.cmp(ext_b))
                        .then(name_a.cmp(&name_b))
                });
            }
        }

        let win1 = self.clone();
        let win2 = self.clone();
        let on_enter_dir: OnEnterDir = Box::new(move |path: PathBuf| { win1.push_dir(path); });
        let on_open_file: OnOpenFile = Box::new(move |path: &std::path::Path, _h: &str| {
            win2.preview_file(path);
        });

        let widget: gtk::Widget = match mode {
            ViewMode::List =>
                list_view::build_list_view(&nodes, &hash, on_enter_dir, on_open_file).upcast(),
            ViewMode::Grid =>
                grid_view::build_grid_view(&nodes, &hash, on_enter_dir, on_open_file).upcast(),
        };
        self.replace_right_panel(widget);
    }

    // ── Right-panel management ────────────────────────────────────────────────

    pub fn replace_right_panel(&self, widget: gtk::Widget) {
        let imp = self.imp();
        clear_box(&imp.right_panel_content);
        imp.right_panel_content.append(&widget);
        imp.right_panel_stack.set_visible_child_name("content");
    }

    pub fn show_empty_state(&self) {
        let imp = self.imp();
        clear_box(&imp.right_panel_content);
        imp.right_panel_stack.set_visible_child_name("empty");
    }

    // ── File preview ──────────────────────────────────────────────────────────

    pub fn preview_file(&self, path: &std::path::Path) {
        let hash      = match self.imp().current_hash.borrow().clone() { Some(h) => h, None => return };
        let repo_path = match self.imp().repo_path.borrow().clone()    { Some(p) => p, None => return };

        match git2::Repository::open(&repo_path) {
            Ok(repo) => file_preview::show_file_preview(self, &repo, &hash, path),
            Err(e)   => self.show_error(&format!("{}: {e}", gettext("Cannot open repository"))),
        }
    }

    // ── Navigation helpers ────────────────────────────────────────────────────

    pub fn push_dir(&self, dir: PathBuf) {
        let imp  = self.imp();
        let prev = imp.current_dir.borrow().clone();
        imp.history_back.borrow_mut().push(prev);
        imp.history_forward.borrow_mut().clear();
        self.navigate_to_dir(dir);
    }

    /// Navigate back one step in the directory history of the current snapshot.
    ///
    /// Called by `setup_history_controls` when the user presses the back button
    /// while inside a snapshot's sub-directory (`current_dir` is non-empty).
    fn navigate_back(&self) {
        let imp = self.imp();
        if let Some(dir) = imp.history_back.borrow_mut().pop() {
            let cur = imp.current_dir.borrow().clone();
            imp.history_forward.borrow_mut().push(cur);
            self.navigate_to_dir(dir);
        }
    }

    /// Navigate forward one step in the directory history of the current snapshot.
    ///
    /// Called by `setup_history_controls` when the user presses the forward button
    /// while inside a snapshot's sub-directory (`current_dir` is non-empty).
    fn navigate_forward(&self) {
        let imp = self.imp();
        if let Some(dir) = imp.history_forward.borrow_mut().pop() {
            let cur = imp.current_dir.borrow().clone();
            imp.history_back.borrow_mut().push(cur);
            self.navigate_to_dir(dir);
        }
    }

    /// Update toolbar button sensitivity from the dir-nav back/forward stacks.
    ///
    /// # Note
    /// Commit-nav and dir-nav share the same `HistoryControls` widget in the
    /// toolbar.  `setup_history_controls` dispatches the signals to the correct
    /// handler based on context (dir-nav when inside a snapshot sub-directory,
    /// commit-nav otherwise).  This function updates the button state for the
    /// dir-nav case; `update_commit_nav_buttons` handles the commit-nav case.
    fn update_dir_nav_buttons(&self) {
        let imp = self.imp();
        let can_back    = !imp.history_back.borrow().is_empty();
        let can_forward = !imp.history_forward.borrow().is_empty();
        imp.toolbar.history_controls().set_sensitivity(can_back, can_forward);
    }

    // ── Location bar ──────────────────────────────────────────────────────────

    pub fn enter_location_mode(&self) {
        let imp     = self.imp();
        let current = imp.current_dir.borrow().clone();
        imp.toolbar.location_entry().set_text(current.to_str().unwrap_or(""));
        imp.toolbar.set_location_mode(true);
    }

    fn leave_location_mode(&self) {
        self.imp().toolbar.set_location_mode(false);
    }

    fn navigate_to_typed_path(&self, text: &str) {
        self.leave_location_mode();
        self.push_dir(PathBuf::from(text.trim()));
    }

    // ── Search ────────────────────────────────────────────────────────────────

    /// Debounce search input by 200 ms before executing `run_search`.
    fn on_search_changed(&self, query: String) {
        let imp = self.imp();

        if let Some(prev) = imp.search_debounce.borrow().as_ref() {
            prev.store(true, Ordering::Relaxed);
        }

        let flag = Arc::new(AtomicBool::new(false));
        *imp.search_debounce.borrow_mut() = Some(flag.clone());

        let win = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            if flag.load(Ordering::Relaxed) { return glib::ControlFlow::Break; }
            win.run_search(query.clone());
            glib::ControlFlow::Break
        });
    }

    fn run_search(&self, query: String) {
        let imp = self.imp();
        *imp.last_query.borrow_mut() = query.clone();

        let active_filter = imp.filter_state.borrow().clone();

        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(prev) = imp.search_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }
        *imp.search_cancel.borrow_mut() = Some(cancel);

        let list = imp.commit_list.clone();
        list.remove_all();

        let all           = imp.all_commits.borrow().clone();
        let selected_year = imp.selected_year.get();

        if !query.is_empty() {
            *imp.timeline_level.borrow_mut() = TimelineLevel::Commits;
            imp.timeline_stack.set_visible_child_name("commits");
            imp.timeline_back_button.set_visible(true);
            imp.timeline_header_title.set_subtitle("");
        }

        let filtered: Vec<CommitInfo> = if query.is_empty() && active_filter.is_empty() {
            if selected_year != 0 {
                all.into_iter()
                    .filter(|c| {
                        glib::DateTime::from_unix_local(c.timestamp)
                            .map(|d| d.year() == selected_year)
                            .unwrap_or(false)
                    })
                    .collect()
            } else {
                all
            }
        } else {
            let q = query.to_lowercase();
            all.into_iter()
                .filter(|c| {
                    let year_ok = selected_year == 0 || {
                        glib::DateTime::from_unix_local(c.timestamp)
                            .map(|d| d.year() == selected_year)
                            .unwrap_or(false)
                    };
                    if !year_ok { return false; }
                    if !active_filter.matches(c) { return false; }
                    if q.is_empty() { return true; }

                    let text_match =
                        c.summary.to_lowercase().contains(&q)
                        || c.hash.to_lowercase().contains(&q)
                        || short_hash(&c.hash).starts_with(&q)
                        || c.author.to_lowercase().contains(&q);

                    text_match || matches_calendar(c.timestamp, &q)
                })
                .collect()
        };

        commit_controller::populate_commit_list(&list, &filtered);
    }

    // ── Filter popover wiring ─────────────────────────────────────────────────
    //
    // filter_button is a #[template_child] from window.blp.
    // This function creates SearchFilterPopover, parents it to that button,
    // and wires popup/popdown/filters-changed.

    fn setup_filter_popover(&self) {
        let popover = SearchFilterPopover::new();
        let btn = self.imp().filter_button.get();

        popover.set_parent(&btn);

        {
            let pop = popover.clone();
            btn.connect_toggled(move |b| {
                if b.is_active() { pop.popup(); } else { pop.popdown(); }
            });
        }

        {
            let b = btn.clone();
            popover.connect_closed(move |_| { b.set_active(false); });
        }

        {
            let win = self.clone();
            popover.connect_local("filters-changed", false, move |_| {
                let pop = win.imp().filter_popover.borrow();
                if let Some(ref p) = *pop {
                    *win.imp().filter_state.borrow_mut() = p.current_filter();
                }
                drop(pop);
                let q = win.imp().last_query.borrow().clone();
                win.run_search(q);
                None
            });
        }

        *self.imp().filter_popover.borrow_mut() = Some(popover);
    }

    // ── Toast helper ──────────────────────────────────────────────────────────

    fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        if let Some(overlay) = self.imp().content_toolbar_view
            .parent()
            .and_then(|w| w.downcast::<adw::ToastOverlay>().ok())
        {
            overlay.add_toast(toast);
        }
    }

    // ── Error display ─────────────────────────────────────────────────────────

    fn show_error(&self, message: &str) {
        eprintln!("[TemporalExplorer] {message}");
        let dialog = adw::AlertDialog::new(Some(&gettext("Error")), Some(message));
        AlertDialogExt::add_response(&dialog, "ok", &gettext("OK"));
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── Timestamp formatter ───────────────────────────────────────────────────

    fn format_timestamp(ts: i64) -> String {
        glib::DateTime::from_unix_local(ts)
            .and_then(|d| d.format("%Y-%m-%d %H:%M"))
            .map(|s| s.to_string())
            .unwrap_or_else(|_| ts.to_string())
    }
}
