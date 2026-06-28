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
use gettextrs::gettext;
use glib::object::ObjectExt;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::address_bar;
use crate::batch_operations_dialog::{BatchOp, BatchOperationsDialog};
use crate::column_chooser::{ColumnChooser, ColumnVisibility};
use crate::commit_controller;
use crate::file_grid_captions_dialog::{CaptionFlags, FileGridCaptionsDialog};
use crate::file_preview;
use crate::filter_types_dialog::FilterTypesDialog;
use crate::git_engine::{CommitInfo, DirCache, HistoryReader, SnapshotResolver, TreeNode};
use crate::merge_conflict_dialog::{ConflictInfo, MergeConflictDialog};
use crate::new_branch_dialog::NewBranchDialog;
use crate::node_properties_dialog::{NodeProperties, NodePropertiesDialog};
use crate::operation_progress_dialog::OperationProgressDialog;
use crate::search_filter_popover::{FileTypeFilter, FilterState, SearchFilterPopover};
use crate::select_commits_by_pattern::{commit_matches_pattern, SelectCommitsByPattern};
use crate::timeline_filter;
use crate::toolbar::TemporalToolbar;
use crate::view_controls::FileSortMode;
use crate::views::grid_view::{FileGridMetadata, GridZoom};
use crate::views::list_view::{OnEnterDir, OnOpenFile};
use crate::views::{grid_view, list_view};

// ── ViewMode ───────────────────────────────────────────────────────────────────

enum SnapshotWriteMessage {
    Progress { fraction: f64, status: String },
    Done(Result<PathBuf, String>),
}

enum SearchProgressMessage {
    Progress { current: usize, total: usize },
    Done(Option<(Vec<CommitInfo>, Vec<(String, Vec<String>)>)>),
}

#[derive(Debug, Clone, Copy)]
enum FileSelectionCommand {
    SelectAll,
    UnselectAll,
    Invert,
}

/// Whether the right panel renders files as a list or a grid.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ViewMode {
    #[default]
    List,
    Grid,
}

// ── TimelineLevel ──────────────────────────────────────────────────────────────

/// The currently visible drill-down level of the left timeline panel.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum TimelineLevel {
    #[default]
    Years,
    Months,
    Commits,
}

// ── DebugRepository ────────────────────────────────────────────────────────────

/// Newtype wrapper that makes `git2::Repository` implement `Debug`.
pub struct DebugRepository(pub git2::Repository);

impl std::fmt::Debug for DebugRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Repository")
            .field(&"<git2::Repository>")
            .finish()
    }
}

impl std::ops::Deref for DebugRepository {
    type Target = git2::Repository;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

// ── Private implementation ─────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/window.ui")]
    pub struct TemporalExplorerWindow {
        // ── Toolbar / title ──────────────────────────────────────────────────
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub toolbar: TemplateChild<TemporalToolbar>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,

        // ── Timeline panel ───────────────────────────────────────────────────
        #[template_child]
        pub timeline_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub timeline_back_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub timeline_header_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub year_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub month_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub commit_search_entry: TemplateChild<gtk::SearchEntry>,

        // ── Filter button (declared in window.blp; popover wired at runtime) ─
        #[template_child]
        pub filter_button: TemplateChild<gtk::ToggleButton>,

        #[template_child]
        pub commit_list: TemplateChild<gtk::ListBox>,

        // SearchFilterPopover cannot be a TemplateChild — its type is not
        // registered in Blueprint at template-inflate time, so it is created
        // in setup_filter_popover() and stored here.
        pub filter_popover: RefCell<Option<SearchFilterPopover>>,

        // ── Right panel ──────────────────────────────────────────────────────
        #[template_child]
        pub content_toolbar_view: TemplateChild<adw::ToolbarView>,
        #[template_child]
        pub right_panel_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub right_panel_content: TemplateChild<gtk::Box>,
        #[template_child]
        pub empty_state: TemplateChild<adw::StatusPage>,
        #[template_child]
        pub split_view: TemplateChild<adw::OverlaySplitView>,

        // ── Commit info bar ──────────────────────────────────────────────────
        #[template_child]
        pub commit_info_bar: TemplateChild<gtk::ActionBar>,
        #[template_child]
        pub commit_hash_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub commit_message_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub commit_date_label: TemplateChild<gtk::Label>,

        // ── Runtime state ────────────────────────────────────────────────────
        pub settings: OnceCell<gio::Settings>,
        pub all_commits: RefCell<Vec<CommitInfo>>,
        pub commit_index: RefCell<HashMap<String, usize>>,
        pub year_counts: RefCell<Vec<(i32, usize)>>,
        pub branch_commit_index: RefCell<HashMap<String, HashSet<String>>>,
        pub seen_authors: RefCell<HashSet<String>>,
        pub changed_files_cache: RefCell<HashMap<String, Vec<String>>>,
        pub repo_path: RefCell<Option<PathBuf>>,
        pub repository: RefCell<Option<DebugRepository>>,
        pub last_query: RefCell<String>,
        pub current_hash: RefCell<Option<String>>,
        pub current_dir: RefCell<PathBuf>,
        pub current_nodes: RefCell<Vec<TreeNode>>,
        pub history_back: RefCell<Vec<PathBuf>>,
        pub history_forward: RefCell<Vec<PathBuf>>,
        pub view_mode: RefCell<ViewMode>,
        pub repo_name: RefCell<String>,

        // ── Commit navigation history ────────────────────────────────────────
        pub commit_nav_back: RefCell<Vec<String>>,
        pub commit_nav_forward: RefCell<Vec<String>>,

        pub sort_mode: RefCell<FileSortMode>,
        pub grid_zoom: RefCell<GridZoom>,
        pub grid_caption_flags: RefCell<CaptionFlags>,
        pub column_visibility: RefCell<ColumnVisibility>,
        pub show_hidden_files: Cell<bool>,

        pub timeline_level: RefCell<TimelineLevel>,
        pub selected_year: Cell<i32>,
        pub loading_commits: Cell<bool>,
        pub dir_cache: RefCell<DirCache>,
        pub search_debounce: RefCell<Option<Arc<AtomicBool>>>,
        pub search_cancel: RefCell<Option<Arc<AtomicBool>>>,

        pub filter_state: RefCell<FilterState>,
        pub load_cancel: RefCell<Option<Arc<AtomicBool>>>,
        pub context_node: RefCell<Option<TreeNode>>,
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
            self.settings
                .set(gio::Settings::new(
                    "io.github.johnpetersa19.TemporalExplorer",
                ))
                .ok();
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
#[allow(dead_code)]
fn commit_touches_path(
    repo: &git2::Repository,
    commit: &git2::Commit<'_>,
    path: &std::path::Path,
    is_dir: bool,
) -> bool {
    let Ok(tree) = commit.tree() else {
        return false;
    };

    let parent_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());

    let Ok(diff) = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) else {
        return false;
    };

    for delta in diff.deltas() {
        let old_path = delta.old_file().path();
        let new_path = delta.new_file().path();

        let matches = |candidate: Option<&std::path::Path>| -> bool {
            let Some(candidate) = candidate else {
                return false;
            };

            if is_dir {
                candidate == path || candidate.starts_with(path)
            } else {
                candidate == path
            }
        };

        if matches(old_path) || matches(new_path) {
            return true;
        }
    }

    false
}

fn file_matches_search_category(path: &str, filter: &FileTypeFilter) -> bool {
    let path_obj = std::path::Path::new(path);

    let ext = path_obj
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let in_folder = path_obj
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty());

    let audio_ext = matches!(
        ext.as_str(),
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "mid" | "midi"
    );

    let document_ext = matches!(
        ext.as_str(),
        "doc" | "docx" | "odt" | "ott" | "rtf" | "abw" | "pages"
    );

    let image_ext = matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" | "heic" | "avif"
    );

    let pdf_ext = ext == "pdf";

    let text_ext = matches!(
        ext.as_str(),
        "txt"
            | "md"
            | "markdown"
            | "rst"
            | "log"
            | "csv"
            | "json"
            | "jsonc"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "html"
            | "css"
            | "scss"
            | "js"
            | "ts"
            | "jsx"
            | "tsx"
            | "rs"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "py"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "go"
            | "java"
            | "kt"
            | "swift"
            | "php"
            | "rb"
            | "lua"
            | "blp"
            | "ui"
            | "desktop"
            | "service"
    );

    let video_ext = matches!(
        ext.as_str(),
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "flv" | "wmv" | "mpeg" | "mpg"
    );

    (filter.audio && audio_ext)
        || (filter.documents && document_ext)
        || (filter.folders && in_folder)
        || (filter.images && image_ext)
        || (filter.pdf && pdf_ext)
        || (filter.text && text_ext)
        || (filter.videos && video_ext)
        || filter.other_ext.as_deref().map_or(false, |wanted| {
            ext == wanted.trim_start_matches('.').to_lowercase()
        })
}

fn format_git_mode(mode: i32) -> String {
    match mode {
        0o040000 => "040000 · Directory".to_string(),
        0o100644 => "100644 · Read/write file".to_string(),
        0o100755 => "100755 · Executable file".to_string(),
        0o120000 => "120000 · Symbolic link".to_string(),
        0o160000 => "160000 · Git submodule".to_string(),
        other => format!("{other:o} · Git mode"),
    }
}

fn format_file_size(size: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    let size_f = size as f64;

    if size_f >= GIB {
        format!("{:.1} GB", size_f / GIB)
    } else if size_f >= MIB {
        format!("{:.1} MB", size_f / MIB)
    } else if size_f >= KIB {
        format!("{:.1} KB", size_f / KIB)
    } else {
        format!("{size} B")
    }
}

fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

#[inline]
fn year_from_timestamp(ts: i64) -> Option<i32> {
    glib::DateTime::from_unix_local(ts).ok().map(|dt| dt.year())
}

fn bump_year_count(year_counts: &mut Vec<(i32, usize)>, year: i32) {
    if let Some((_, count)) = year_counts.iter_mut().find(|(y, _)| *y == year) {
        *count += 1;
    } else {
        year_counts.push((year, 1));
        year_counts.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    }
}

fn collect_branch_commit_index(
    repo: &git2::Repository,
) -> (Vec<String>, HashMap<String, HashSet<String>>) {
    let mut branch_names = Vec::new();
    let mut index: HashMap<String, HashSet<String>> = HashMap::new();

    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else {
        return (branch_names, index);
    };

    for branch_result in branches {
        let Ok((branch, _)) = branch_result else {
            continue;
        };

        let Ok(Some(name)) = branch.name() else {
            continue;
        };

        let Some(target) = branch.get().target() else {
            continue;
        };

        let name = name.to_string();
        let mut hashes = HashSet::new();

        if let Ok(mut walk) = repo.revwalk() {
            if walk.push(target).is_ok() {
                for oid in walk.flatten() {
                    hashes.insert(oid.to_string());
                }
            }
        }

        branch_names.push(name.clone());
        index.insert(name, hashes);
    }

    branch_names.sort();
    branch_names.dedup();

    (branch_names, index)
}

// ── Calendar / date matching ───────────────────────────────────────────────────

/// Returns `true` if `query` matches any date-related representation of `ts`.
///
/// Recognised formats: ISO date (`2024-03-15`), year-month (`2024-03`),
/// bare year (`2024`), full/abbreviated English month name, and the
/// locale-translated month name returned by [`timeline_filter::month_name`].
fn matches_calendar(ts: i64, q: &str) -> bool {
    // Fast reject for ordinary text queries. Calendar matches are short:
    // years, ISO dates, year-month values, timestamps, and month names.
    if q.len() < 2 || q.len() > 16 {
        return false;
    }

    let Ok(dt) = glib::DateTime::from_unix_local(ts) else {
        return false;
    };

    let year = dt.year();
    let month = dt.month() as u32;
    let day = dt.day_of_month();

    // Common numeric filters first, before allocating formatted strings.
    if q.len() == 4 {
        if let Ok(q_year) = q.parse::<i32>() {
            return year == q_year;
        }
    }

    let month_full = match month {
        1 => "january",
        2 => "february",
        3 => "march",
        4 => "april",
        5 => "may",
        6 => "june",
        7 => "july",
        8 => "august",
        9 => "september",
        10 => "october",
        11 => "november",
        12 => "december",
        _ => "",
    };
    let month_abbr = &month_full[..3.min(month_full.len())];

    // Month-only queries do not need ISO/timestamp allocation.
    if q.chars().all(|c| c.is_ascii_alphabetic()) {
        return month_full.contains(q)
            || month_abbr == q
            || timeline_filter::month_name(month)
                .to_lowercase()
                .contains(q);
    }

    let iso_date = format!("{:04}-{:02}-{:02}", year, month, day);
    let year_month = format!("{:04}-{:02}", year, month);
    let human = dt
        .format("%Y-%m-%d %H:%M")
        .map(|s| s.to_string())
        .unwrap_or_default();

    iso_date.contains(q) || year_month.contains(q) || human.contains(q)
}

// ── Short-hash helpers ─────────────────────────────────────────────────────────

const SHORT_HASH_LEN: usize = 8;

/// Returns the canonical short prefix of a full commit hash.
#[inline]
fn short_hash(hash: &str) -> &str {
    &hash[..SHORT_HASH_LEN.min(hash.len())]
}

/// Returns the short hash used in visible UI labels.
#[inline]
fn display_hash(hash: &str) -> &str {
    short_hash(hash)
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
        imp.toolbar.open_repo_button().connect_clicked(move |_| {
            win.open_repo_dialog();
        });

        // Keep show_sidebar_button in sync with the split-view's show-sidebar property.
        imp.toolbar
            .show_sidebar_button()
            .bind_property("active", &imp.split_view.get(), "show-sidebar")
            .bidirectional()
            .sync_create()
            .build();

        // new_branch_button fires win.new-branch via action-name in Blueprint;
        // start insensitive until a repository is loaded.
        imp.toolbar.new_branch_button().set_sensitive(false);

        let win_g = self.clone();
        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        gesture.connect_pressed(move |_, _n, _, _| {
            win_g.enter_location_mode();
        });
        imp.toolbar.address_bar().add_controller(gesture);

        let win = self.clone();
        imp.toolbar.location_entry().connect_activate(move |entry| {
            win.navigate_to_typed_path(entry.text().as_str());
        });

        let win = self.clone();
        imp.toolbar.location_cancel_btn().connect_clicked(move |_| {
            win.leave_location_mode();
        });

        let win = self.clone();
        imp.toolbar.search_button().connect_toggled(move |button| {
            if button.is_active() {
                win.enter_search_mode();
            } else if win.imp().toolbar.is_search_mode() {
                win.leave_search_mode();
            }
        });

        let win = self.clone();
        imp.toolbar.search_close_btn().connect_clicked(move |_| {
            win.leave_search_mode();
        });

        let win = self.clone();
        imp.toolbar
            .search_entry()
            .connect_search_changed(move |entry| {
                let query = entry.text().to_string();

                if win.imp().commit_search_entry.text().as_str() != query {
                    win.imp().commit_search_entry.set_text(&query);
                }

                win.on_search_changed(query);
            });

        let win = self.clone();
        imp.timeline_back_button.connect_clicked(move |_| {
            win.timeline_pop();
        });

        let win = self.clone();
        imp.year_list.connect_row_activated(move |_, row| {
            let year = row
                .widget_name()
                .parse::<i32>()
                .unwrap_or_else(|_| row.index());

            win.on_year_selected(year);
        });

        let win = self.clone();
        imp.month_list.connect_row_activated(move |_, row| {
            if let Ok(month) = row.widget_name().parse::<u32>() {
                win.on_month_selected(month);
            }
        });

        let win = self.clone();
        imp.commit_list.connect_row_activated(move |_, row| {
            let hash = row.widget_name().to_string();

            if !hash.is_empty() {
                win.on_commit_selected(hash);
            }
        });

        let win = self.clone();
        imp.commit_search_entry
            .connect_search_changed(move |entry| {
                let query = entry.text().to_string();

                if win.imp().toolbar.search_entry().text().as_str() != query {
                    win.imp().toolbar.set_search_text(&query);
                }

                win.on_search_changed(query);
            });

        self.setup_filter_popover();
        self.setup_history_controls();
        self.setup_view_controls();
        self.setup_saved_view_preferences();
        self.setup_actions();
    }

    // ── GAction registration ──────────────────────────────────────────────────

    fn setup_actions(&self) {
        let actions: &[(&str, fn(&TemporalExplorerWindow))] = &[
            ("select-all-files", Self::select_all_files),
            ("unselect-all-files", Self::unselect_all_files),
            ("invert-file-selection", Self::invert_file_selection),
            ("reload-repository", Self::reload_repository),
            ("open-repository-system", Self::open_repository_in_system),
            ("open-repository-console", Self::open_repository_in_console),
            (
                "copy-current-repository-path",
                Self::copy_current_repository_path,
            ),
            ("show-captions", Self::show_file_grid_captions_dialog),
            ("current-properties", Self::show_current_folder_properties),
            ("open-repository", Self::open_repo_dialog),
            ("batch-operations", Self::show_batch_operations_dialog),
            ("select-by-pattern", Self::show_select_by_pattern_dialog),
            ("filter-file-type", Self::show_filter_types_dialog),
            ("show-column-chooser", Self::show_column_chooser),
            ("new-branch", Self::show_new_branch_dialog),
            ("toggle-sidebar", Self::toggle_sidebar),
            ("search-commits", Self::focus_commit_search),
            ("toggle-filter-popover", Self::toggle_filter_popover),
            ("open-date-range-dialog", Self::open_date_range_dialog),
            ("clear-all-filters", Self::clear_all_filters),
            ("list-view", Self::switch_to_list_view),
            ("grid-view", Self::switch_to_grid_view),
            ("zoom-in", Self::zoom_in),
            ("zoom-out", Self::zoom_out),
            ("reset-zoom", Self::reset_zoom),
            ("copy-current-commit-sha", Self::copy_current_commit_sha),
            (
                "show-current-commit-details",
                Self::show_current_commit_details,
            ),
            (
                "preview-selected-file",
                Self::preview_selected_file_from_action,
            ),
            ("open-context-menu", Self::open_context_menu_from_action),
            (
                "show-merge-conflicts",
                Self::show_merge_conflicts_for_current,
            ),
            ("select-all-commits", Self::select_all_commits),
            ("invert-commit-selection", Self::invert_commit_selection),
            ("first-commit", Self::select_first_commit),
            ("latest-commit", Self::select_latest_commit),
            ("context-open", Self::context_open_selected),
            ("context-open-with", Self::context_open_with_selected),
            ("context-export", Self::context_export_selected),
            ("context-copy-path", Self::context_copy_path_selected),
            ("context-copy-content", Self::context_copy_content_selected),
            ("context-show-system", Self::context_show_system_selected),
            ("context-properties", Self::context_properties_selected),
        ];

        for (name, handler) in actions {
            let win = self.clone();
            let h = *handler;
            let action = gio::SimpleAction::new(name, None);
            action.connect_activate(move |_, _| h(&win));
            self.add_action(&action);
        }

        // Menus declared inside TemporalToolbar use actions such as win.reload-repository.
        // Insert action groups explicitly into both the custom toolbar and the
        // MenuButton, so PopoverMenu action lookup is stable.
        let toolbar = self.imp().toolbar.get();

        toolbar.insert_action_group("win", Some(self));
        toolbar
            .main_menu_button()
            .insert_action_group("win", Some(self));

        if let Some(app) = self.application() {
            toolbar.insert_action_group("app", Some(&app));
            toolbar
                .main_menu_button()
                .insert_action_group("app", Some(&app));

            app.set_accels_for_action("win.reload-repository", &["F5"]);
            app.set_accels_for_action("win.select-all-files", &["<Control>a"]);
            app.set_accels_for_action("win.invert-file-selection", &["<Control><Shift>a"]);
            app.set_accels_for_action("win.open-repository", &["<Control>o"]);
            app.set_accels_for_action("win.first-commit", &["<Control>Home"]);
            app.set_accels_for_action("win.latest-commit", &["<Control>End"]);
            app.set_accels_for_action("win.toggle-sidebar", &["F9"]);
            app.set_accels_for_action("win.search-commits", &["<Control>f"]);
            app.set_accels_for_action("win.toggle-filter-popover", &["<Control><Shift>f"]);
            app.set_accels_for_action("win.open-date-range-dialog", &["<Control><Shift>d"]);
            app.set_accels_for_action("win.filter-file-type", &["<Control><Shift>t"]);
            app.set_accels_for_action("win.clear-all-filters", &["<Control>Escape"]);
            app.set_accels_for_action("win.list-view", &["<Control>1"]);
            app.set_accels_for_action("win.grid-view", &["<Control>2"]);
            app.set_accels_for_action("win.zoom-in", &["<Control>plus", "<Control>KP_Add"]);
            app.set_accels_for_action("win.zoom-out", &["<Control>minus", "<Control>KP_Subtract"]);
            app.set_accels_for_action("win.reset-zoom", &["<Control>0"]);
            app.set_accels_for_action("win.show-column-chooser", &["<Control><Shift>c"]);
            app.set_accels_for_action("win.show-captions", &["<Control><Shift>g"]);
            app.set_accels_for_action("win.copy-current-commit-sha", &["<Control>c"]);
            app.set_accels_for_action("win.show-current-commit-details", &["<Control>i"]);
            app.set_accels_for_action("win.preview-selected-file", &["space"]);
            app.set_accels_for_action("win.open-context-menu", &["<Shift>F10"]);
            app.set_accels_for_action("win.batch-operations", &["<Control><Shift>b"]);
            app.set_accels_for_action("win.show-merge-conflicts", &["<Control><Shift>m"]);
            app.set_accels_for_action("win.select-by-pattern", &["<Control>s"]);
            app.set_accels_for_action("win.invert-commit-selection", &["<Control><Shift>i"]);
        }
    }

    // ── Folder / main menu actions ───────────────────────────────────────────

    fn reload_repository(&self) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };

        self.load_repository(repo_path);
    }

    fn open_repository_in_system(&self) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };

        let file = gio::File::for_path(&repo_path);
        let uri = file.uri();

        if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>).is_err() {
            self.show_error(&gettext("Could not open repository in the system"));
        }
    }

    fn open_repository_in_console(&self) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };

        let attempts: &[(&str, &[&str])] = &[
            ("kgx", &["--working-directory"]),
            ("gnome-terminal", &["--working-directory"]),
            ("konsole", &["--workdir"]),
            ("xfce4-terminal", &["--working-directory"]),
        ];

        for (program, args) in attempts {
            let mut command = Command::new(program);
            command.args(*args).arg(&repo_path);

            if command.spawn().is_ok() {
                return;
            }
        }

        self.show_error(&gettext("Could not open a console for this repository"));
    }

    fn copy_current_repository_path(&self) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };

        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&repo_path.to_string_lossy());
            self.show_toast(&gettext("Repository path copied"));
        }
    }

    fn find_flow_box(widget: &gtk::Widget) -> Option<gtk::FlowBox> {
        if let Ok(flow) = widget.clone().downcast::<gtk::FlowBox>() {
            return Some(flow);
        }

        let mut child = widget.first_child();

        while let Some(w) = child {
            let next = w.next_sibling();

            if let Some(found) = Self::find_flow_box(&w) {
                return Some(found);
            }

            child = next;
        }

        None
    }

    fn find_grid_view(widget: &gtk::Widget) -> Option<gtk::GridView> {
        if let Ok(grid) = widget.clone().downcast::<gtk::GridView>() {
            return Some(grid);
        }

        let mut child = widget.first_child();

        while let Some(w) = child {
            let next = w.next_sibling();

            if let Some(found) = Self::find_grid_view(&w) {
                return Some(found);
            }

            child = next;
        }

        None
    }

    fn find_list_box(widget: &gtk::Widget) -> Option<gtk::ListBox> {
        if let Ok(list) = widget.clone().downcast::<gtk::ListBox>() {
            return Some(list);
        }

        let mut child = widget.first_child();

        while let Some(w) = child {
            let next = w.next_sibling();

            if let Some(found) = Self::find_list_box(&w) {
                return Some(found);
            }

            child = next;
        }

        None
    }

    fn set_file_selection(&self, command: FileSelectionCommand) -> Option<usize> {
        let root = self.imp().right_panel_content.get();

        if let Some(list) = Self::find_list_box(root.upcast_ref()) {
            return Some(Self::set_list_box_selection(&list, command));
        }

        if let Some(grid) = Self::find_grid_view(root.upcast_ref()) {
            let command = match command {
                FileSelectionCommand::SelectAll => grid_view::GridSelectionCommand::SelectAll,
                FileSelectionCommand::UnselectAll => grid_view::GridSelectionCommand::UnselectAll,
                FileSelectionCommand::Invert => grid_view::GridSelectionCommand::Invert,
            };
            return Some(grid_view::set_grid_view_selection(&grid, command));
        }

        if let Some(flow) = Self::find_flow_box(root.upcast_ref()) {
            return Some(Self::set_flow_box_selection(&flow, command));
        }

        None
    }

    fn set_list_box_selection(list: &gtk::ListBox, command: FileSelectionCommand) -> usize {
        match command {
            FileSelectionCommand::SelectAll => {
                list.select_all();
                list.selected_rows().len()
            }
            FileSelectionCommand::UnselectAll => {
                list.unselect_all();
                0
            }
            FileSelectionCommand::Invert => {
                let mut selected = 0usize;
                let mut child = list.first_child();

                while let Some(widget) = child {
                    child = widget.next_sibling();

                    if let Ok(row) = widget.downcast::<gtk::ListBoxRow>() {
                        if row.is_selected() {
                            list.unselect_row(&row);
                        } else {
                            list.select_row(Some(&row));
                            selected += 1;
                        }
                    }
                }

                selected
            }
        }
    }

    fn set_flow_box_selection(flow: &gtk::FlowBox, command: FileSelectionCommand) -> usize {
        match command {
            FileSelectionCommand::SelectAll => {
                flow.select_all();
                flow.selected_children().len()
            }
            FileSelectionCommand::UnselectAll => {
                flow.unselect_all();
                0
            }
            FileSelectionCommand::Invert => {
                let mut selected = 0usize;
                let mut child = flow.first_child();

                while let Some(widget) = child {
                    child = widget.next_sibling();

                    if let Ok(flow_child) = widget.downcast::<gtk::FlowBoxChild>() {
                        if flow_child.is_selected() {
                            flow.unselect_child(&flow_child);
                        } else {
                            flow.select_child(&flow_child);
                            selected += 1;
                        }
                    }
                }

                selected
            }
        }
    }

    fn select_all_files(&self) {
        match self.set_file_selection(FileSelectionCommand::SelectAll) {
            Some(count) => {
                self.show_toast(&format!("{} {} item(s)", gettext("Selected"), count));
            }
            None => self.show_toast(&gettext("No file view available")),
        }
    }

    fn unselect_all_files(&self) {
        match self.set_file_selection(FileSelectionCommand::UnselectAll) {
            Some(_) => self.show_toast(&gettext("Selection cleared")),
            None => self.show_toast(&gettext("No file view available")),
        }
    }

    fn invert_file_selection(&self) {
        match self.set_file_selection(FileSelectionCommand::Invert) {
            Some(count) => {
                self.show_toast(&format!("{} {} item(s)", gettext("Selected"), count));
            }
            None => self.show_toast(&gettext("No file view available")),
        }
    }

    fn show_current_folder_properties(&self) {
        if self.imp().current_hash.borrow().is_none() {
            self.show_toast(&gettext("No snapshot selected"));
            return;
        }

        let current_dir = self.imp().current_dir.borrow().clone();
        self.show_node_properties(&TreeNode::Dir(current_dir));
    }

    fn toggle_sidebar(&self) {
        let current = self.imp().split_view.shows_sidebar();
        self.imp().split_view.set_show_sidebar(!current);
    }

    fn focus_commit_search(&self) {
        self.enter_search_mode();
    }

    fn toggle_filter_popover(&self) {
        self.enter_search_mode();
        let button = self.imp().toolbar.search_filter_button();
        button.set_active(!button.is_active());
    }

    fn open_date_range_dialog(&self) {
        self.enter_search_mode();
        if let Some(ref popover) = *self.imp().filter_popover.borrow() {
            popover.open_date_range_dialog();
        }
    }

    fn clear_all_filters(&self) {
        if let Some(ref popover) = *self.imp().filter_popover.borrow() {
            popover.reset_all();
        }
        self.imp().commit_search_entry.set_text("");
        self.imp().toolbar.set_search_text("");
        self.on_search_changed(String::new());
    }

    fn switch_to_list_view(&self) {
        self.set_view_mode_from_action(false);
    }

    fn switch_to_grid_view(&self) {
        self.set_view_mode_from_action(true);
    }

    fn set_view_mode_from_action(&self, is_grid: bool) {
        *self.imp().view_mode.borrow_mut() = if is_grid {
            ViewMode::Grid
        } else {
            ViewMode::List
        };
        self.imp().toolbar.view_controls().set_view_mode(is_grid);

        if let Some(settings) = self.imp().settings.get() {
            settings
                .set_string("default-view", if is_grid { "grid" } else { "list" })
                .ok();
        }

        self.reload_current_dir_if_possible();
    }

    fn zoom_in(&self) {
        let raw = match *self.imp().grid_zoom.borrow() {
            GridZoom::Small => 1,
            GridZoom::Normal => 2,
            GridZoom::Large => 2,
        };
        self.set_grid_zoom_from_action(raw);
    }

    fn zoom_out(&self) {
        let raw = match *self.imp().grid_zoom.borrow() {
            GridZoom::Small => 0,
            GridZoom::Normal => 0,
            GridZoom::Large => 1,
        };
        self.set_grid_zoom_from_action(raw);
    }

    fn reset_zoom(&self) {
        self.set_grid_zoom_from_action(1);
    }

    fn set_grid_zoom_from_action(&self, raw: u32) {
        let raw = raw.min(2);
        let zoom = match raw {
            0 => GridZoom::Small,
            2 => GridZoom::Large,
            _ => GridZoom::Normal,
        };

        *self.imp().grid_zoom.borrow_mut() = zoom;
        self.imp().toolbar.view_controls().set_zoom_level(raw);

        if let Some(settings) = self.imp().settings.get() {
            settings.set_uint("grid-zoom-level", raw).ok();
        }

        self.reload_current_dir_if_possible();
    }

    fn copy_current_commit_sha(&self) {
        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_toast(&gettext("No commit selected"));
            return;
        };

        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&hash);
            self.show_toast(&gettext("Commit SHA copied"));
        }
    }

    fn show_current_commit_details(&self) {
        if self.imp().current_hash.borrow().is_none() {
            self.show_toast(&gettext("No commit selected"));
            return;
        }

        self.show_current_folder_properties();
    }

    fn preview_selected_file_from_action(&self) {
        match self.selected_file_view_node() {
            Some(TreeNode::File(path)) => self.preview_file(&path),
            Some(_) => self.show_toast(&gettext("Selected item is not a file")),
            None => self.show_toast(&gettext("No file selected")),
        }
    }

    fn open_context_menu_from_action(&self) {
        if let Some((node, anchor)) = self.selected_file_view_node_and_anchor() {
            self.show_file_context_menu(node, &anchor);
        } else {
            self.show_toast(&gettext("No file selected"));
        }
    }

    fn show_merge_conflicts_for_current(&self) {
        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_toast(&gettext("No commit selected"));
            return;
        };

        self.try_show_merge_conflict_dialog(&hash);
    }

    fn select_all_commits(&self) {
        let mut count = 0usize;
        let mut row = self.imp().commit_list.first_child();

        while let Some(widget) = row {
            row = widget.next_sibling();
            if let Ok(list_row) = widget.downcast::<gtk::ListBoxRow>() {
                if !list_row.widget_name().is_empty() {
                    list_row.add_css_class("pattern-match");
                    count += 1;
                }
            }
        }

        self.show_toast(&format!("{} {} commit(s)", gettext("Selected"), count));
    }

    fn invert_commit_selection(&self) {
        let mut count = 0usize;
        let mut row = self.imp().commit_list.first_child();

        while let Some(widget) = row {
            row = widget.next_sibling();
            if let Ok(list_row) = widget.downcast::<gtk::ListBoxRow>() {
                if list_row.widget_name().is_empty() {
                    continue;
                }

                if list_row.has_css_class("pattern-match") {
                    list_row.remove_css_class("pattern-match");
                } else {
                    list_row.add_css_class("pattern-match");
                    count += 1;
                }
            }
        }

        self.show_toast(&format!("{} {} commit(s)", gettext("Selected"), count));
    }

    fn select_first_commit(&self) {
        let hash = self
            .imp()
            .all_commits
            .borrow()
            .last()
            .map(|commit| commit.hash.clone());

        if let Some(hash) = hash {
            self.on_commit_selected(hash);
        } else {
            self.show_toast(&gettext("No commits loaded"));
        }
    }

    fn select_latest_commit(&self) {
        let hash = self
            .imp()
            .all_commits
            .borrow()
            .first()
            .map(|commit| commit.hash.clone());

        if let Some(hash) = hash {
            self.on_commit_selected(hash);
        } else {
            self.show_toast(&gettext("No commits loaded"));
        }
    }

    fn reload_current_dir_if_possible(&self) {
        if self.imp().current_hash.borrow().is_some() {
            let dir = self.imp().current_dir.borrow().clone();
            self.navigate_to_dir(dir);
        }
    }

    // ── NewBranchDialog ───────────────────────────────────────────────────────

    fn show_new_branch_dialog(&self) {
        let dialog = NewBranchDialog::new();
        let win = self.clone();
        dialog.connect_branch_created(move |_, name| {
            win.create_branch(name);
        });
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
                    .map(|_| ()) // ← drop Branch<'_> here; borrow of `repo` ends
                    .map_err(|e| {
                        format!(
                            "{} \u{2018}{}\u{2019}: {e}",
                            gettext("Failed to create branch"),
                            name
                        )
                    })
            });

        // ── 4. Report outcome ─────────────────────────────────────────────────
        match result {
            Ok(()) => {
                let toast = adw::Toast::new(&format!(
                    "{} \u{2018}{}\u{2019}",
                    gettext("Created branch"),
                    name,
                ));
                if let Some(overlay) = self
                    .imp()
                    .content_toolbar_view
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
        self.imp()
            .toolbar
            .history_controls()
            .connect_local("navigate-back", false, move |_| {
                // ── Step 1: snapshot condition into owned bools ───────────────────
                // All borrow() guards are dropped at the end of this block.
                // `imp` is NOT stored in a let-binding that outlives this block,
                // because doing so would keep an implicit borrow alive across the
                // dispatch call below, which triggers the RefCell panic.
                let (has_hash, in_subdir) = {
                    let imp = win.imp();
                    let has_hash = imp.current_hash.borrow().is_some();
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
        self.imp()
            .toolbar
            .history_controls()
            .connect_local("navigate-forward", false, move |_| {
                // Same pattern as "navigate-back".
                let (has_hash, in_subdir) = {
                    let imp = win.imp();
                    let has_hash = imp.current_hash.borrow().is_some();
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
    /// call-site would alias those borrows and cause a fatal RefCell panic inside
    /// the non-unwinding glib closure marshaller (SIGABRT).
    fn navigate_commit_back(&self) {
        // ── Extract all needed values; every borrow guard drops at end of block ──
        let (prev_hash, current) = {
            let imp = self.imp();
            let prev = imp.commit_nav_back.borrow_mut().pop();
            let cur = imp.current_hash.borrow().clone();
            (prev, cur)
            // imp, commit_nav_back borrow_mut, and current_hash borrow all dropped here
        };

        if let Some(prev) = prev_hash {
            if let Some(cur) = current {
                // No RefCell is borrowed at this point.
                self.imp().commit_nav_forward.borrow_mut().push(cur);
            }
            // All borrows are released here before jump_to_commit_hash writes
            // to current_hash, current_dir, history_back, and history_forward.
            self.jump_to_commit_hash(prev);
        }
    }

    /// Navigate forward one step in the commit history.
    ///
    /// # Borrow safety
    ///
    /// Mirrors `navigate_commit_back`: all RefCell values are extracted into
    /// owned locals before `jump_to_commit_hash` is called, avoiding any
    /// simultaneous borrow_mut aliasing.
    fn navigate_commit_forward(&self) {
        // ── Extract all needed values; every borrow guard drops at end of block ──
        let (next_hash, current) = {
            let imp = self.imp();
            let next = imp.commit_nav_forward.borrow_mut().pop();
            let cur = imp.current_hash.borrow().clone();
            (next, cur)
            // imp, commit_nav_forward borrow_mut, and current_hash borrow all dropped here
        };

        if let Some(next) = next_hash {
            if let Some(cur) = current {
                // No RefCell is borrowed at this point.
                self.imp().commit_nav_back.borrow_mut().push(cur);
            }
            // All borrows released before jump_to_commit_hash runs.
            self.jump_to_commit_hash(next);
        }
    }

    /// Navigate directly to `hash`, resetting dir history and updating the info bar.
    fn jump_to_commit_hash(&self, hash: String) {
        let imp = self.imp();
        *imp.current_hash.borrow_mut() = Some(hash.clone());
        *imp.current_dir.borrow_mut() = PathBuf::new();
        imp.history_back.borrow_mut().clear();
        imp.history_forward.borrow_mut().clear();

        {
            let commits = imp.all_commits.borrow();
            let index = imp.commit_index.borrow();

            let commit = index
                .get(&hash)
                .and_then(|idx| commits.get(*idx))
                .or_else(|| {
                    commits
                        .iter()
                        .find(|c| c.hash.starts_with(short_hash(&hash)))
                });

            if let Some(commit) = commit {
                imp.commit_hash_label.set_label(display_hash(&commit.hash));
                imp.commit_message_label.set_label(&commit.summary);
                imp.commit_date_label
                    .set_label(&Self::format_timestamp(commit.timestamp));
                imp.commit_info_bar.set_revealed(true);
            }
        }
        self.update_commit_nav_buttons();
        self.navigate_to_dir(PathBuf::new());
    }

    fn update_commit_nav_buttons(&self) {
        let imp = self.imp();
        let can_back = !imp.commit_nav_back.borrow().is_empty();
        let can_forward = !imp.commit_nav_forward.borrow().is_empty();
        imp.toolbar
            .history_controls()
            .set_sensitivity(can_back, can_forward);
    }

    // ── Saved view preferences ────────────────────────────────────────────────

    fn setup_saved_view_preferences(&self) {
        let imp = self.imp();
        let Some(settings) = imp.settings.get() else {
            return;
        };

        let saved_view = settings.string("default-view");
        let is_grid = saved_view.as_str() == "grid";
        *imp.view_mode.borrow_mut() = if is_grid {
            ViewMode::Grid
        } else {
            ViewMode::List
        };
        imp.toolbar.view_controls().set_view_mode(is_grid);

        let zoom_level = settings.uint("grid-zoom-level").min(2);
        *imp.grid_zoom.borrow_mut() = match zoom_level {
            0 => GridZoom::Small,
            2 => GridZoom::Large,
            _ => GridZoom::Normal,
        };
        imp.toolbar.view_controls().set_zoom_level(zoom_level);

        let sort_mode = match settings.string("file-sort-mode").as_str() {
            "name-desc" => FileSortMode::NameDescending,
            "last-modified" => FileSortMode::LastModified,
            "first-modified" => FileSortMode::FirstModified,
            "size" => FileSortMode::Size,
            "type" => FileSortMode::Extension,
            _ => FileSortMode::Name,
        };
        *imp.sort_mode.borrow_mut() = sort_mode;
        imp.toolbar.view_controls().set_sort_mode(sort_mode);

        *imp.grid_caption_flags.borrow_mut() =
            CaptionFlags::from_bits_truncate(settings.uint("grid-caption-flags"));

        let show_hidden = settings.boolean("show-hidden-files");
        imp.show_hidden_files.set(show_hidden);
        imp.toolbar
            .view_controls()
            .set_show_hidden_files(show_hidden);
    }

    // ── ViewControls wiring ───────────────────────────────────────────────────

    fn setup_view_controls(&self) {
        let win = self.clone();
        self.imp()
            .toolbar
            .view_controls()
            .connect_local("view-mode-changed", false, move |args| {
                let is_grid = args[1].get::<bool>().unwrap_or(false);
                *win.imp().view_mode.borrow_mut() = if is_grid {
                    ViewMode::Grid
                } else {
                    ViewMode::List
                };
                if let Some(settings) = win.imp().settings.get() {
                    settings
                        .set_string("default-view", if is_grid { "grid" } else { "list" })
                        .ok();
                }
                let dir = win.imp().current_dir.borrow().clone();
                if win.imp().current_hash.borrow().is_some() {
                    win.navigate_to_dir(dir);
                }
                None
            });

        let win = self.clone();
        self.imp()
            .toolbar
            .view_controls()
            .connect_local("sort-changed", false, move |args| {
                let raw = args[1].get::<u32>().unwrap_or(0);
                let mode = match raw {
                    1 => FileSortMode::NameDescending,
                    2 => FileSortMode::LastModified,
                    3 => FileSortMode::FirstModified,
                    4 => FileSortMode::Size,
                    5 => FileSortMode::Extension,
                    _ => FileSortMode::Name,
                };
                *win.imp().sort_mode.borrow_mut() = mode;
                let sort_key = match mode {
                    FileSortMode::Name => "name",
                    FileSortMode::NameDescending => "name-desc",
                    FileSortMode::LastModified => "last-modified",
                    FileSortMode::FirstModified => "first-modified",
                    FileSortMode::Size => "size",
                    FileSortMode::Status => "status",
                    FileSortMode::Extension => "type",
                };
                if let Some(settings) = win.imp().settings.get() {
                    settings.set_string("file-sort-mode", sort_key).ok();
                }
                let dir = win.imp().current_dir.borrow().clone();
                if win.imp().current_hash.borrow().is_some() {
                    win.navigate_to_dir(dir);
                }
                None
            });

        let win = self.clone();
        self.imp()
            .toolbar
            .view_controls()
            .connect_local("zoom-changed", false, move |args| {
                let raw = args[1].get::<u32>().unwrap_or(1);
                let zoom = match raw {
                    0 => GridZoom::Small,
                    2 => GridZoom::Large,
                    _ => GridZoom::Normal,
                };
                *win.imp().grid_zoom.borrow_mut() = zoom;
                if let Some(settings) = win.imp().settings.get() {
                    settings.set_uint("grid-zoom-level", raw.min(2)).ok();
                }
                let dir = win.imp().current_dir.borrow().clone();
                if win.imp().current_hash.borrow().is_some() {
                    win.navigate_to_dir(dir);
                }
                None
            });

        let win = self.clone();
        self.imp().toolbar.view_controls().connect_local(
            "hidden-files-changed",
            false,
            move |args| {
                let show_hidden = args[1].get::<bool>().unwrap_or(false);
                win.imp().show_hidden_files.set(show_hidden);

                if let Some(settings) = win.imp().settings.get() {
                    settings.set_boolean("show-hidden-files", show_hidden).ok();
                }

                win.reload_current_dir_if_possible();
                None
            },
        );

        let win = self.clone();
        self.imp()
            .toolbar
            .view_controls()
            .connect_local("captions-requested", false, move |_| {
                win.show_file_grid_captions_dialog();
                None
            });
    }

    // ── FileGridCaptionsDialog ────────────────────────────────────────────────

    pub fn show_file_grid_captions_dialog(&self) {
        let dialog = FileGridCaptionsDialog::new();
        dialog.set_flags(*self.imp().grid_caption_flags.borrow());

        let win = self.clone();
        dialog.connect_captions_changed(move |_, flags| {
            *win.imp().grid_caption_flags.borrow_mut() = flags;
            if let Some(settings) = win.imp().settings.get() {
                settings.set_uint("grid-caption-flags", flags.bits()).ok();
            }

            let dir = win.imp().current_dir.borrow().clone();
            if win.imp().current_hash.borrow().is_some() {
                win.navigate_to_dir(dir);
            }
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── ColumnChooser ─────────────────────────────────────────────────────────

    pub fn show_column_chooser(&self) {
        let dialog = ColumnChooser::new();
        dialog.apply_visibility(&self.imp().column_visibility.borrow());

        let win = self.clone();
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
        dialog.connect_operation_requested(move |dlg, op, shas| match op {
            BatchOp::CherryPick { signoff } => {
                let msg = format!(
                    "{} {} commit(s){}",
                    gettext("Cherry-pick"),
                    shas.len(),
                    if signoff {
                        gettext(" with sign-off")
                    } else {
                        String::new()
                    },
                );
                win.show_toast(&msg);
                dlg.mark_done();
            }
            BatchOp::ExportPatches { dest_dir } => {
                let shas_clone = shas.clone();
                let repo_path = win.imp().repo_path.borrow().clone();
                let dlg_ref = dlg.clone();
                dlg.set_progress_visible(true);

                let (tx, rx) = std::sync::mpsc::sync_channel::<()>(1);

                std::thread::spawn(move || {
                    if let Some(repo_path) = repo_path {
                        let _ = std::fs::create_dir_all(&dest_dir);

                        if let Ok(repo) = git2::Repository::open(&repo_path) {
                            for (i, sha) in shas_clone.iter().enumerate() {
                                let patch_path = dest_dir.join(format!(
                                    "{:04}-{}.patch",
                                    i + 1,
                                    short_hash(sha)
                                ));

                                if let Ok(oid) = git2::Oid::from_str(sha) {
                                    if let Ok(commit) = repo.find_commit(oid) {
                                        if let Ok(tree) = commit.tree() {
                                            let parent_tree =
                                                commit.parent(0).ok().and_then(|p| p.tree().ok());
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
                    Ok(()) => {
                        dlg_ref.mark_done();
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(_) => glib::ControlFlow::Break,
                });
            }
            BatchOp::CopyShas { short } => {
                let text = shas
                    .iter()
                    .map(|s| {
                        if short {
                            short_hash(s).to_string()
                        } else {
                            s.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Some(display) = gtk::gdk::Display::default() {
                    display.clipboard().set_text(&text);
                }
                win.show_toast(&format!("{} {} SHA(s)", gettext("Copied"), shas.len()));
                dlg.mark_done();
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
            let all = win.imp().all_commits.borrow();
            let matching: std::collections::HashSet<String> = all
                .iter()
                .filter(|c| commit_matches_pattern(c, pattern, mode, icase))
                .map(|c| short_hash(&c.hash).to_string())
                .collect();

            let list = win.imp().commit_list.clone();

            // Mark matching rows with CSS class "pattern-match".
            let mut row = list.first_child();
            while let Some(r) = row {
                if let Some(list_row) = r.downcast_ref::<gtk::ListBoxRow>() {
                    let hash = list_row.widget_name().to_string();

                    if !hash.is_empty() {
                        let is_match = matching.contains(short_hash(&hash));

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

            win.show_toast(&format!(
                "{} {} commit(s)",
                gettext("Selected"),
                matching.len()
            ));
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

        if commit.parent_count() < 2 {
            return;
        }

        let ours = commit.parent(0).ok();
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
                    None,
                    None,
                    None,
                )
                .ok()?;
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
                let diff = repo
                    .diff_tree_to_tree(Some(&ot), Some(&tt), Some(&mut opts))
                    .ok()?;
                let mut buf = Vec::new();
                diff.print(git2::DiffFormat::Patch, |_d, _h, line| {
                    buf.extend_from_slice(line.content());
                    true
                })
                .ok()?;
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

        let (ours_sha, ours_author, ours_date) = fmt_commit(ours.as_ref());
        let (theirs_sha, theirs_author, theirs_date) = fmt_commit(theirs.as_ref());

        let info = ConflictInfo {
            file_path: conflict_file,
            ours_sha,
            ours_author,
            ours_date,
            theirs_sha,
            theirs_author,
            theirs_date,
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
                if apply_all {
                    format!(" ({})", gettext("applied to all"))
                } else {
                    String::new()
                },
            );
            win.show_toast(&msg);
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── FilterTypesDialog ─────────────────────────────────────────────────────

    pub fn show_filter_types_dialog(&self) {
        // Ensure changed_files is populated for every commit before opening the
        // dialog so that extension filtering works correctly on the first open.
        // load_changed_files is idempotent — subsequent calls are no-ops.
        if let Some(ref repo_wrapper) = *self.imp().repository.borrow() {
            // Open a dedicated handle so we do not hold a borrow on `repository`
            // across the mutable borrow of `all_commits` below.
            let repo_path = self.imp().repo_path.borrow().clone();
            if let Some(path) = repo_path {
                if let Ok(repo) = git2::Repository::open(&path) {
                    let _ = repo_wrapper; // keep lint happy; real work uses `repo`
                    for commit in self.imp().all_commits.borrow_mut().iter_mut() {
                        if !commit.has_changed_files_loaded() {
                            commit.load_changed_files(&repo);
                        }
                    }
                }
            }
        }

        let dialog = FilterTypesDialog::new();

        let win = self.clone();
        dialog.connect_file_type_selected(move |_, ext| {
            {
                let mut fs = win.imp().filter_state.borrow_mut();
                fs.files.other_ext = if ext.is_empty() {
                    None
                } else {
                    Some(ext.to_string())
                };
            }
            let q = win.imp().last_query.borrow().clone();
            win.run_search(q);

            if let Some(ref pop) = *win.imp().filter_popover.borrow() {
                pop.set_file_ext_filter(ext);
            }
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── CSS loader ────────────────────────────────────────────────────────────

    fn setup_styles(&self) {
        let provider = gtk::CssProvider::new();
        provider
            .load_from_resource("/io/github/johnpetersa19/TemporalExplorer/temporal-explorer.css");
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
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
                if let Some(path) = file.path() {
                    win.load_repository(path);
                }
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
                let repo_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("repository")
                    .to_string();

                let imp = self.imp();
                *imp.repo_path.borrow_mut() = Some(path.clone());
                *imp.repository.borrow_mut() = Some(DebugRepository(repo));
                *imp.repo_name.borrow_mut() = repo_name.clone();
                *imp.current_dir.borrow_mut() = PathBuf::new();
                imp.history_back.borrow_mut().clear();
                imp.history_forward.borrow_mut().clear();
                imp.commit_nav_back.borrow_mut().clear();
                imp.commit_nav_forward.borrow_mut().clear();
                imp.toolbar.history_controls().reset();
                imp.window_title.set_title(&repo_name);
                imp.window_title.set_subtitle(path.to_str().unwrap_or(""));
                imp.toolbar.new_branch_button().set_sensitive(true);

                // Populate branch chips and build a branch -> commit-hash index.
                let (branches, branch_index) =
                    collect_branch_commit_index(&imp.repository.borrow().as_ref().unwrap().0);
                *imp.branch_commit_index.borrow_mut() = branch_index;

                if let Some(ref pop) = *imp.filter_popover.borrow() {
                    pop.populate_branch_chips(&branches);
                }

                self.load_timeline(cancel);
            }
            Err(e) => self.show_error(&format!("{}: {e}", gettext("Failed to open repository"))),
        }
    }

    // ── Search mode ────────────────────────────────────────────────────────────

    fn enter_search_mode(&self) {
        self.imp().toolbar.set_search_mode(true);

        let sidebar_query = self.imp().commit_search_entry.text().to_string();
        if self.imp().toolbar.search_entry().text().as_str() != sidebar_query {
            self.imp().toolbar.set_search_text(&sidebar_query);
        }

        self.imp().toolbar.search_entry().grab_focus();
    }

    fn leave_search_mode(&self) {
        self.imp().toolbar.set_search_mode(false);
        self.imp().toolbar.search_button().set_active(false);
    }

    // ── Timeline loading ───────────────────────────────────────────────────────

    fn rebuild_author_chips_from_all_commits(&self) {
        let imp = self.imp();
        let pop_borrow = imp.filter_popover.borrow();
        let Some(ref pop) = *pop_borrow else { return };

        let commits = imp.all_commits.borrow();

        let mut authors: Vec<String> = commits
            .iter()
            .map(|commit| commit.author.trim().to_string())
            .filter(|author| !author.is_empty())
            .collect();

        authors.sort_by_key(|author| author.to_lowercase());
        authors.dedup_by(|a, b| a.eq_ignore_ascii_case(b));

        pop.populate_author_chips(&authors);
    }

    /// Feeds new author names into the filter popover, deduplicating against
    /// authors already seen in previous pages.
    ///
    /// Called once per received page so the popover chips grow incrementally
    /// as commits stream in, without re-scanning the entire `all_commits` vec.
    fn update_author_chips_for_page(&self, page: &[CommitInfo]) {
        let imp = self.imp();
        let pop_borrow = imp.filter_popover.borrow();
        let Some(ref pop) = *pop_borrow else { return };

        let mut changed = false;
        let mut authors = Vec::new();

        {
            let mut seen = imp.seen_authors.borrow_mut();

            for commit in page {
                let author = commit.author.trim();
                if !author.is_empty() && seen.insert(author.to_string()) {
                    changed = true;
                }
            }

            if changed {
                authors = seen.iter().cloned().collect();
            }
        }

        if changed {
            authors.sort();
            pop.populate_author_chips(&authors);
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
        imp.timeline_header_title
            .set_subtitle(&gettext("No commits found"));
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
        self.imp().commit_index.borrow_mut().clear();
        self.imp().year_counts.borrow_mut().clear();
        self.imp().seen_authors.borrow_mut().clear();
        self.imp().changed_files_cache.borrow_mut().clear();

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
                    if cancel_worker.load(Ordering::Relaxed) {
                        return;
                    }
                    let _ = tx.send(Ok(page));
                })
            });

            // Send the EOS sentinel (empty vec) on success, or the error string.
            // The idle callback distinguishes them by the is_empty() check.
            match result {
                Ok(()) => {
                    let _ = tx.send(Ok(Vec::new()));
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                }
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
                        win.rebuild_author_chips_from_all_commits();
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

                    {
                        let imp = win.imp();
                        let base = imp.all_commits.borrow().len();

                        {
                            let mut index = imp.commit_index.borrow_mut();
                            let mut year_counts = imp.year_counts.borrow_mut();

                            for (offset, commit) in page.iter().enumerate() {
                                index.insert(commit.hash.clone(), base + offset);

                                if let Some(year) = year_from_timestamp(commit.timestamp) {
                                    bump_year_count(&mut year_counts, year);
                                }
                            }
                        }

                        imp.all_commits.borrow_mut().extend(page);
                    }

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

    fn visible_timeline_commits<'a>(&'a self, commits: &'a [CommitInfo]) -> Vec<&'a CommitInfo> {
        let filter = self.imp().filter_state.borrow().clone();

        commits
            .iter()
            .filter(|commit| {
                // Author filter
                if let Some(ref author) = filter.author {
                    let wanted_author = author.to_lowercase();
                    if !commit.author.to_lowercase().contains(&wanted_author) {
                        return false;
                    }
                }

                // Date range filter
                if let Some(since) = filter.date.from {
                    if commit.timestamp < since {
                        return false;
                    }
                }

                if let Some(until) = filter.date.to {
                    if commit.timestamp >= until {
                        return false;
                    }
                }

                // Branch filter is intentionally not handled here because it depends
                // on branch_commit_index and is already handled in run_search().
                // The timeline sidebar remains commit-date/author/date driven.

                true
            })
            .collect()
    }

    fn year_counts_for_visible_commits(&self, commits: &[CommitInfo]) -> Vec<(i32, usize)> {
        let visible = self.visible_timeline_commits(commits);
        let mut counts: Vec<(i32, usize)> = Vec::new();

        for commit in visible {
            if let Some(year) = year_from_timestamp(commit.timestamp) {
                bump_year_count(&mut counts, year);
            }
        }

        counts
    }

    // ── Year list ─────────────────────────────────────────────────────────────

    fn populate_year_list(&self) {
        let imp = self.imp();
        imp.year_list.remove_all();

        let commits = imp.all_commits.borrow();
        let year_counts = self.year_counts_for_visible_commits(&commits);

        for (year, count) in year_counts.iter() {
            imp.year_list
                .append(&commit_controller::build_year_row(*year, *count));
        }

        *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
        imp.timeline_stack.set_visible_child_name("years");
        imp.timeline_back_button.set_visible(false);
        imp.timeline_header_title.set_title(&gettext("Timeline"));

        if self.imp().filter_state.borrow().is_active() {
            imp.timeline_header_title
                .set_subtitle(&format!("{} year(s)", year_counts.len()));
        } else {
            imp.timeline_header_title.set_subtitle("");
        }
    }

    // ── Year selected ─────────────────────────────────────────────────────────

    fn on_year_selected(&self, year: i32) {
        let imp = self.imp();
        imp.selected_year.set(year);
        imp.month_list.remove_all();
        imp.commit_list.remove_all();

        let commits = imp.all_commits.borrow();
        let visible = self.visible_timeline_commits(&commits);
        let visible_owned: Vec<CommitInfo> = visible.iter().map(|c| (*c).clone()).collect();

        // Populate the month drill-down list for the selected year, respecting
        // the active author/date filters.
        for (month, count) in &timeline_filter::months_for_year(&visible_owned, year) {
            imp.month_list
                .append(&commit_controller::build_month_row(*month, *count));
        }

        // Also filter the visible commit list to the whole selected year.
        let filtered = timeline_filter::commits_for_year(&visible_owned, year);
        commit_controller::populate_commit_list_refs(&imp.commit_list, &filtered);

        *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
        imp.timeline_stack.set_visible_child_name("months");
        imp.timeline_back_button.set_visible(true);
        imp.timeline_header_title.set_title(&year.to_string());
        imp.timeline_header_title
            .set_subtitle(&format!("{} commit(s)", filtered.len()));
    }

    // ── Month selected ────────────────────────────────────────────────────────

    fn on_month_selected(&self, month: u32) {
        let imp = self.imp();
        let year = imp.selected_year.get();
        imp.commit_list.remove_all();

        let commits = imp.all_commits.borrow();
        let visible = self.visible_timeline_commits(&commits);
        let visible_owned: Vec<CommitInfo> = visible.iter().map(|c| (*c).clone()).collect();

        let filtered = timeline_filter::commits_for_month(&visible_owned, year, month);
        commit_controller::populate_commit_list_refs(&imp.commit_list, &filtered);

        *imp.timeline_level.borrow_mut() = TimelineLevel::Commits;
        imp.timeline_stack.set_visible_child_name("commits");
        imp.timeline_header_title.set_subtitle(&format!(
            "{} {} · {} commit(s)",
            timeline_filter::month_name(month),
            year,
            filtered.len(),
        ));
    }

    // ── Timeline back ─────────────────────────────────────────────────────────

    fn timeline_pop(&self) {
        let imp = self.imp();
        let level = *imp.timeline_level.borrow();
        match level {
            TimelineLevel::Commits => {
                if imp.selected_year.get() > 0 {
                    *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
                    imp.timeline_stack.set_visible_child_name("months");
                    imp.timeline_header_title.set_subtitle("");
                } else {
                    *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
                    imp.timeline_stack.set_visible_child_name("years");
                    imp.timeline_back_button.set_visible(false);
                    imp.timeline_header_title.set_title(&gettext("Timeline"));
                    imp.timeline_header_title.set_subtitle("");
                }
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
        *imp.current_dir.borrow_mut() = PathBuf::new();
        imp.history_back.borrow_mut().clear();
        imp.history_forward.borrow_mut().clear();

        {
            let commits = imp.all_commits.borrow();
            let index = imp.commit_index.borrow();

            let commit = index
                .get(&hash)
                .and_then(|idx| commits.get(*idx))
                .or_else(|| {
                    commits
                        .iter()
                        .find(|c| c.hash.starts_with(short_hash(&hash)))
                });

            if let Some(commit) = commit {
                imp.commit_hash_label.set_label(display_hash(&commit.hash));
                imp.commit_message_label.set_label(&commit.summary);
                imp.commit_date_label
                    .set_label(&Self::format_timestamp(commit.timestamp));
                imp.commit_info_bar.set_revealed(true);
            }
        }

        // Lazily populate changed_files for the selected commit so that
        // FilterTypesDialog and any future diff-aware UI have the data ready.
        // load_changed_files is idempotent — this is a no-op if already loaded.
        {
            let repo_path = imp.repo_path.borrow().clone();
            if let Some(path) = repo_path {
                if let Ok(repo) = git2::Repository::open(&path) {
                    let mut commits = imp.all_commits.borrow_mut();
                    let index = imp.commit_index.borrow();

                    let commit = index.get(&hash).and_then(|idx| commits.get_mut(*idx));

                    if let Some(commit) = commit {
                        commit.load_changed_files(&repo);
                    } else if let Some(commit) = commits
                        .iter_mut()
                        .find(|c| c.hash.starts_with(short_hash(&hash)))
                    {
                        commit.load_changed_files(&repo);
                    }
                }
            }
        }

        self.try_show_merge_conflict_dialog(&hash);
        self.navigate_to_dir(PathBuf::new());
    }

    // ── Directory navigation ──────────────────────────────────────────────────

    pub fn navigate_to_dir(&self, dir: PathBuf) {
        let imp = self.imp();
        let hash = match imp.current_hash.borrow().clone() {
            Some(h) => h,
            None => return,
        };
        let repo_path = match imp.repo_path.borrow().clone() {
            Some(p) => p,
            None => return,
        };
        let repo_name = imp.repo_name.borrow().clone();

        *imp.current_dir.borrow_mut() = dir.clone();

        let win_ab1 = self.clone();
        let win_ab2 = self.clone();
        address_bar::rebuild_address_bar(
            imp.toolbar.address_bar(),
            &repo_name,
            &dir,
            move |path: PathBuf| {
                win_ab1.push_dir(path);
            },
            move || {
                win_ab2.enter_location_mode();
            },
        );
        self.update_dir_nav_buttons();

        // Cancellation token for directory loads.
        //
        // Each navigation invalidates the previous token before starting a new
        // worker.  The token is shared with the UI polling closure through Arc so
        // stale workers cannot update the right panel after the user has already
        // moved to another directory.
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(prev) = imp.load_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }
        *imp.load_cancel.borrow_mut() = Some(cancel.clone());

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
                    Err(e) => win.show_error(&format!("{}: {e}", gettext("Error reading tree"))),
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
    }

    fn render_dir(&self, nodes: Vec<TreeNode>) {
        let imp = self.imp();
        let mode = *imp.view_mode.borrow();
        let sort_mode = *imp.sort_mode.borrow();
        let grid_zoom = *imp.grid_zoom.borrow();
        let grid_caption_flags = *imp.grid_caption_flags.borrow();
        let hash = imp.current_hash.borrow().clone().unwrap_or_default();
        let nodes = self.visible_nodes_for_current_settings(nodes);

        // Metadata can be expensive, especially First/Last Modified because it
        // scans commit history. Only build it when the active sort/captions need it.
        let metadata = if self.file_grid_metadata_needed(sort_mode, grid_caption_flags) {
            self.build_file_grid_metadata(&nodes, &hash, sort_mode, grid_caption_flags)
        } else {
            HashMap::new()
        };

        let mut decorated: Vec<(TreeNode, String, String)> = nodes
            .into_iter()
            .map(|node| {
                let name = node
                    .path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase();

                let ext = std::path::Path::new(&name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_string();

                (node, name, ext)
            })
            .collect();

        match sort_mode {
            FileSortMode::Name | FileSortMode::Status => {
                decorated.sort_by(|a, b| b.0.is_dir().cmp(&a.0.is_dir()).then(a.1.cmp(&b.1)));
            }
            FileSortMode::NameDescending => {
                decorated.sort_by(|a, b| b.0.is_dir().cmp(&a.0.is_dir()).then(b.1.cmp(&a.1)));
            }
            FileSortMode::LastModified => {
                decorated.sort_by(|a, b| {
                    let ma = metadata
                        .get(a.0.path())
                        .and_then(|m| m.last_modified)
                        .unwrap_or(i64::MIN);
                    let mb = metadata
                        .get(b.0.path())
                        .and_then(|m| m.last_modified)
                        .unwrap_or(i64::MIN);
                    b.0.is_dir()
                        .cmp(&a.0.is_dir())
                        .then(mb.cmp(&ma))
                        .then(a.1.cmp(&b.1))
                });
            }
            FileSortMode::FirstModified => {
                decorated.sort_by(|a, b| {
                    let ma = metadata
                        .get(a.0.path())
                        .and_then(|m| m.first_modified)
                        .unwrap_or(i64::MAX);
                    let mb = metadata
                        .get(b.0.path())
                        .and_then(|m| m.first_modified)
                        .unwrap_or(i64::MAX);
                    b.0.is_dir()
                        .cmp(&a.0.is_dir())
                        .then(ma.cmp(&mb))
                        .then(a.1.cmp(&b.1))
                });
            }
            FileSortMode::Size => {
                decorated.sort_by(|a, b| {
                    let sa = metadata.get(a.0.path()).and_then(|m| m.size).unwrap_or(0);
                    let sb = metadata.get(b.0.path()).and_then(|m| m.size).unwrap_or(0);
                    b.0.is_dir()
                        .cmp(&a.0.is_dir())
                        .then(sb.cmp(&sa))
                        .then(a.1.cmp(&b.1))
                });
            }
            FileSortMode::Extension => {
                decorated.sort_by(|a, b| {
                    b.0.is_dir()
                        .cmp(&a.0.is_dir())
                        .then(a.2.cmp(&b.2))
                        .then(a.1.cmp(&b.1))
                });
            }
        }

        let nodes: Vec<TreeNode> = decorated.into_iter().map(|(node, _, _)| node).collect();
        *imp.current_nodes.borrow_mut() = nodes.clone();

        let win1 = self.clone();
        let win2 = self.clone();
        let win3 = self.clone();
        let win4 = self.clone();
        let win5 = self.clone();
        let win6 = self.clone();
        let on_enter_dir: OnEnterDir = Box::new(move |path: PathBuf| {
            win1.push_dir(path);
        });
        let on_open_file: OnOpenFile = Box::new(move |path: &std::path::Path, _h: &str| {
            win2.preview_file(path);
        });
        let on_context_menu: grid_view::OnContextMenu =
            Box::new(move |node: &TreeNode, anchor: &gtk::Widget| {
                win3.show_file_context_menu(node.clone(), anchor);
            });
        let on_background_context_menu: grid_view::OnBackgroundContextMenu =
            Box::new(move |anchor: &gtk::Widget, x, y| {
                win4.show_file_background_context_menu(anchor, x, y);
            });
        let on_list_context_menu: list_view::OnContextMenu =
            Box::new(move |node: &TreeNode, anchor: &gtk::Widget| {
                win5.show_file_context_menu(node.clone(), anchor);
            });
        let on_list_background_context_menu: list_view::OnBackgroundContextMenu =
            Box::new(move |anchor: &gtk::Widget, x, y| {
                win6.show_file_background_context_menu(anchor, x, y);
            });

        let widget: gtk::Widget = match mode {
            ViewMode::List => list_view::build_list_view(
                &nodes,
                &hash,
                on_enter_dir,
                on_open_file,
                on_list_context_menu,
                on_list_background_context_menu,
            )
            .upcast(),
            ViewMode::Grid => grid_view::build_grid_view(
                &nodes,
                &hash,
                grid_zoom,
                grid_caption_flags,
                &metadata,
                on_enter_dir,
                on_open_file,
                on_context_menu,
                on_background_context_menu,
            )
            .upcast(),
        };
        self.replace_right_panel(widget);
    }

    fn visible_nodes_for_current_settings(&self, nodes: Vec<TreeNode>) -> Vec<TreeNode> {
        if self.imp().show_hidden_files.get() {
            return nodes;
        }

        nodes
            .into_iter()
            .filter(|node| {
                node.path()
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map_or(true, |name| !name.starts_with('.'))
            })
            .collect()
    }

    fn selected_file_view_node(&self) -> Option<TreeNode> {
        self.selected_file_view_node_and_anchor()
            .map(|(node, _)| node)
    }

    fn selected_file_view_node_and_anchor(&self) -> Option<(TreeNode, gtk::Widget)> {
        let root = self.imp().right_panel_content.get();

        if let Some(list) = Self::find_list_box(root.upcast_ref()) {
            if let Some(row) = list.selected_rows().first() {
                let idx = row.index();
                if idx >= 0 {
                    return self
                        .imp()
                        .current_nodes
                        .borrow()
                        .get(idx as usize)
                        .cloned()
                        .map(|node| (node, row.clone().upcast::<gtk::Widget>()));
                }
            }
        }

        if let Some(grid) = Self::find_grid_view(root.upcast_ref()) {
            if let Some((item, anchor)) = grid_view::grid_view_selected_item(&grid) {
                return Some((item.node, anchor));
            }
        }

        if let Some(flow) = Self::find_flow_box(root.upcast_ref()) {
            if let Some(child) = flow.selected_children().first() {
                let idx = child.index();
                if idx >= 0 {
                    return self
                        .imp()
                        .current_nodes
                        .borrow()
                        .get(idx as usize)
                        .cloned()
                        .map(|node| (node, child.clone().upcast::<gtk::Widget>()));
                }
            }
        }

        None
    }

    // ── File grid metadata ────────────────────────────────────────────────────

    fn file_grid_metadata_needed(
        &self,
        sort_mode: FileSortMode,
        caption_flags: CaptionFlags,
    ) -> bool {
        matches!(
            sort_mode,
            FileSortMode::Size | FileSortMode::LastModified | FileSortMode::FirstModified
        ) || caption_flags.intersects(CaptionFlags::SIZE | CaptionFlags::DATE)
    }

    fn build_file_grid_metadata(
        &self,
        nodes: &[TreeNode],
        hash: &str,
        sort_mode: FileSortMode,
        caption_flags: CaptionFlags,
    ) -> HashMap<PathBuf, FileGridMetadata> {
        let mut out = HashMap::new();

        let imp = self.imp();
        let repo_ref = imp.repository.borrow();
        let Some(repo_wrapper) = repo_ref.as_ref() else {
            return out;
        };
        let repo: &git2::Repository = &repo_wrapper.0;

        let tree = repo
            .revparse_single(hash)
            .ok()
            .and_then(|obj| obj.peel_to_commit().ok())
            .and_then(|commit| commit.tree().ok());

        let needs_size =
            matches!(sort_mode, FileSortMode::Size) || caption_flags.contains(CaptionFlags::SIZE);

        let needs_dates = matches!(
            sort_mode,
            FileSortMode::LastModified | FileSortMode::FirstModified
        ) || caption_flags.contains(CaptionFlags::DATE);

        for node in nodes {
            let mut meta = FileGridMetadata::default();
            let path = node.path().to_path_buf();

            if needs_size {
                if let Some(ref tree) = tree {
                    meta.size = self.git_blob_size(repo, tree, node);
                    meta.size_label = meta.size.map(format_file_size);
                }
            }

            out.insert(path, meta);
        }

        if needs_dates {
            self.fill_file_grid_dates(repo, nodes, &mut out);
        }

        out
    }

    fn git_blob_size(
        &self,
        repo: &git2::Repository,
        tree: &git2::Tree<'_>,
        node: &TreeNode,
    ) -> Option<u64> {
        if !matches!(node, TreeNode::File(_)) {
            return None;
        }

        let entry = tree.get_path(node.path()).ok()?;
        let blob = repo.find_blob(entry.id()).ok()?;
        Some(blob.size() as u64)
    }

    fn fill_file_grid_dates(
        &self,
        repo: &git2::Repository,
        nodes: &[TreeNode],
        metadata: &mut HashMap<PathBuf, FileGridMetadata>,
    ) {
        // This replaces the old O(nodes × commits × diff) path.
        // We now scan the history once and reuse each commit diff for all
        // currently visible nodes.
        let commits = self.imp().all_commits.borrow();

        let visible_paths: Vec<(PathBuf, bool)> = nodes
            .iter()
            .map(|node| (node.path().to_path_buf(), node.is_dir()))
            .collect();

        for commit_info in commits.iter() {
            let Ok(commit) = repo.find_commit(commit_info.oid()) else {
                continue;
            };

            let Ok(tree) = commit.tree() else {
                continue;
            };

            let parent_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());

            let Ok(diff) = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) else {
                continue;
            };

            for delta in diff.deltas() {
                let old_path = delta.old_file().path();
                let new_path = delta.new_file().path();

                for (path, is_dir) in &visible_paths {
                    let matches_path = |candidate: Option<&std::path::Path>| -> bool {
                        let Some(candidate) = candidate else {
                            return false;
                        };

                        if *is_dir {
                            candidate == path || candidate.starts_with(path)
                        } else {
                            candidate == path
                        }
                    };

                    if matches_path(old_path) || matches_path(new_path) {
                        if let Some(meta) = metadata.get_mut(path) {
                            // all_commits is newest -> oldest.
                            // First hit is the last modification.
                            if meta.last_modified.is_none() {
                                meta.last_modified = Some(commit_info.timestamp);
                                meta.last_modified_label =
                                    Some(Self::format_timestamp(commit_info.timestamp));
                            }

                            // Keep updating; after the loop, this is the oldest hit.
                            meta.first_modified = Some(commit_info.timestamp);
                            meta.first_modified_label =
                                Some(Self::format_timestamp(commit_info.timestamp));
                        }
                    }
                }
            }
        }
    }

    // ── File context menu ─────────────────────────────────────────────────────

    fn show_file_background_context_menu(&self, anchor: &gtk::Widget, x: f64, y: f64) {
        let menu = gio::Menu::new();

        let selection_section = gio::Menu::new();
        selection_section.append(Some(&gettext("Select All")), Some("win.select-all-files"));
        selection_section.append(
            Some(&gettext("Invert Selection")),
            Some("win.invert-file-selection"),
        );
        selection_section.append(
            Some(&gettext("Clear Selection")),
            Some("win.unselect-all-files"),
        );
        menu.append_section(None, &selection_section);

        let view_section = gio::Menu::new();
        view_section.append(Some(&gettext("Captions…")), Some("win.show-captions"));
        menu.append_section(None, &view_section);

        let folder_section = gio::Menu::new();
        folder_section.append(
            Some(&gettext("Open Repository in System")),
            Some("win.open-repository-system"),
        );
        folder_section.append(
            Some(&gettext("Open in Console")),
            Some("win.open-repository-console"),
        );
        folder_section.append(
            Some(&gettext("Copy Repository Path")),
            Some("win.copy-current-repository-path"),
        );
        menu.append_section(None, &folder_section);

        let properties_section = gio::Menu::new();
        properties_section.append(Some(&gettext("Properties")), Some("win.current-properties"));
        menu.append_section(None, &properties_section);

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_has_arrow(false);
        popover.add_css_class("nautilus-context-menu");
        popover.set_parent(anchor);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
            x.round() as i32,
            y.round() as i32,
            1,
            1,
        )));

        popover.connect_closed(|p| {
            let popover = p.clone();

            glib::idle_add_local_once(move || {
                popover.unparent();
            });
        });

        popover.popup();
    }

    fn show_file_context_menu(&self, node: TreeNode, anchor: &gtk::Widget) {
        *self.imp().context_node.borrow_mut() = Some(node.clone());

        let is_file = !node.is_dir() && !node.is_submodule();

        let menu = gio::Menu::new();

        let open_section = gio::Menu::new();
        open_section.append(Some(&gettext("Open")), Some("win.context-open"));
        open_section.append(Some(&gettext("Open With…")), Some("win.context-open-with"));
        menu.append_section(None, &open_section);

        let edit_section = gio::Menu::new();
        edit_section.append(Some(&gettext("Export File…")), Some("win.context-export"));
        edit_section.append(
            Some(&gettext("Copy Repository Path")),
            Some("win.context-copy-path"),
        );
        edit_section.append(
            Some(&gettext("Copy Content")),
            Some("win.context-copy-content"),
        );
        menu.append_section(None, &edit_section);

        let system_section = gio::Menu::new();
        system_section.append(
            Some(&gettext("Show in System")),
            Some("win.context-show-system"),
        );
        menu.append_section(None, &system_section);

        let properties_section = gio::Menu::new();
        properties_section.append(Some(&gettext("Properties")), Some("win.context-properties"));
        menu.append_section(None, &properties_section);

        let group = gio::SimpleActionGroup::new();

        let open_action = gio::SimpleAction::new("context-open", None);
        {
            let win = self.clone();
            let node = node.clone();

            open_action.connect_activate(move |_, _| {
                if node.is_dir() || node.is_submodule() {
                    win.push_dir(node.path().to_path_buf());
                } else {
                    win.open_snapshot_node_with_default_app(&node);
                }
            });
        }
        group.add_action(&open_action);

        let open_with_action = gio::SimpleAction::new("context-open-with", None);
        open_with_action.set_enabled(is_file);
        {
            let win = self.clone();
            let node = node.clone();

            open_with_action.connect_activate(move |_, _| {
                win.open_snapshot_node_with_app_chooser(&node);
            });
        }
        group.add_action(&open_with_action);

        let export_action = gio::SimpleAction::new("context-export", None);
        export_action.set_enabled(is_file);
        {
            let win = self.clone();
            let node = node.clone();

            export_action.connect_activate(move |_, _| {
                win.export_snapshot_node(&node);
            });
        }
        group.add_action(&export_action);

        let copy_path_action = gio::SimpleAction::new("context-copy-path", None);
        {
            let win = self.clone();
            let node = node.clone();

            copy_path_action.connect_activate(move |_, _| {
                win.copy_repository_path(&node);
            });
        }
        group.add_action(&copy_path_action);

        let copy_content_action = gio::SimpleAction::new("context-copy-content", None);
        copy_content_action.set_enabled(is_file);
        {
            let win = self.clone();
            let node = node.clone();

            copy_content_action.connect_activate(move |_, _| {
                win.copy_snapshot_node_content(&node);
            });
        }
        group.add_action(&copy_content_action);

        let show_system_action = gio::SimpleAction::new("context-show-system", None);
        {
            let win = self.clone();
            let node = node.clone();

            show_system_action.connect_activate(move |_, _| {
                win.show_node_in_system(&node);
            });
        }
        group.add_action(&show_system_action);

        let properties_action = gio::SimpleAction::new("context-properties", None);
        {
            let win = self.clone();
            let node = node.clone();

            properties_action.connect_activate(move |_, _| {
                win.show_node_properties(&node);
            });
        }
        group.add_action(&properties_action);

        // Não muda o design: continua Gtk.PopoverMenu nativo.
        // O grupo "win" local garante que win.context-* seja resolvido.
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.insert_action_group("win", Some(&group));
        popover.set_has_arrow(false);
        popover.add_css_class("nautilus-context-menu");
        popover.set_parent(anchor);

        // Atrasa o unparent para o GTK terminar a ativação do item.
        popover.connect_closed(|p| {
            let popover = p.clone();

            glib::idle_add_local_once(move || {
                popover.unparent();
            });
        });

        popover.popup();
    }

    fn with_context_node<F>(&self, f: F)
    where
        F: FnOnce(&Self, TreeNode),
    {
        let node = self.imp().context_node.borrow().clone();

        if let Some(node) = node {
            f(self, node);
        }
    }

    fn context_open_selected(&self) {
        self.with_context_node(|win, node| {
            if node.is_dir() || node.is_submodule() {
                win.push_dir(node.path().to_path_buf());
            } else {
                win.open_snapshot_node_with_default_app(&node);
            }
        });
    }

    fn context_open_with_selected(&self) {
        self.with_context_node(|win, node| {
            win.open_snapshot_node_with_app_chooser(&node);
        });
    }

    fn context_export_selected(&self) {
        self.with_context_node(|win, node| {
            win.export_snapshot_node(&node);
        });
    }

    fn context_copy_path_selected(&self) {
        self.with_context_node(|win, node| {
            win.copy_repository_path(&node);
        });
    }

    fn context_copy_content_selected(&self) {
        self.with_context_node(|win, node| {
            win.copy_snapshot_node_content(&node);
        });
    }

    fn context_show_system_selected(&self) {
        self.with_context_node(|win, node| {
            win.show_node_in_system(&node);
        });
    }

    fn context_properties_selected(&self) {
        self.with_context_node(|win, node| {
            win.show_node_properties(&node);
        });
    }

    fn write_snapshot_node_to_path_with_progress<F>(
        &self,
        node: &TreeNode,
        target: PathBuf,
        title: &str,
        initial_status: &str,
        on_done: F,
    ) where
        F: FnOnce(&Self, PathBuf) + 'static,
    {
        if node.is_dir() || node.is_submodule() {
            self.show_toast(&gettext(
                "Only files can be opened or exported from a snapshot",
            ));
            return;
        }

        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_error(&gettext("No repository loaded"));
            return;
        };

        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_error(&gettext("No snapshot selected"));
            return;
        };

        let node_path = node.path().to_path_buf();
        let cancel = Arc::new(AtomicBool::new(false));

        let dialog = OperationProgressDialog::new();
        dialog.setup(title, initial_status, cancel.clone());
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));

        let (tx, rx) = std::sync::mpsc::sync_channel::<SnapshotWriteMessage>(16);
        let worker_cancel = cancel.clone();

        std::thread::spawn(move || {
            let result = (|| -> Result<PathBuf, String> {
                let repo = git2::Repository::open(&repo_path)
                    .map_err(|e| format!("Cannot open repository: {e}"))?;

                let obj = repo
                    .revparse_single(&hash)
                    .map_err(|e| format!("Cannot read snapshot: {e}"))?;

                let commit = obj
                    .peel_to_commit()
                    .map_err(|e| format!("Cannot peel commit: {e}"))?;

                let tree = commit
                    .tree()
                    .map_err(|e| format!("Cannot read tree: {e}"))?;

                let entry = tree
                    .get_path(&node_path)
                    .map_err(|e| format!("Cannot find file in snapshot: {e}"))?;

                let blob = repo
                    .find_blob(entry.id())
                    .map_err(|e| format!("Cannot read file blob: {e}"))?;

                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("Cannot create destination directory: {e}"))?;
                }

                let content = blob.content();
                let total = content.len().max(1);
                let mut written = 0usize;

                let mut file = std::fs::File::create(&target)
                    .map_err(|e| format!("Cannot create destination file: {e}"))?;

                for chunk in content.chunks(64 * 1024) {
                    if worker_cancel.load(Ordering::Relaxed) {
                        return Err("Operation cancelled".to_string());
                    }

                    file.write_all(chunk)
                        .map_err(|e| format!("Cannot write destination file: {e}"))?;

                    written += chunk.len();

                    let fraction = written as f64 / total as f64;
                    let status = format!("Writing {} / {} bytes", written, total);

                    let _ = tx.send(SnapshotWriteMessage::Progress { fraction, status });
                }

                file.flush()
                    .map_err(|e| format!("Cannot flush destination file: {e}"))?;

                Ok(target)
            })();

            let _ = tx.send(SnapshotWriteMessage::Done(result));
        });

        let win = self.clone();
        let callback = std::rc::Rc::new(std::cell::RefCell::new(Some(on_done)));
        let callback_ref = callback.clone();

        glib::idle_add_local(move || loop {
            match rx.try_recv() {
                Ok(SnapshotWriteMessage::Progress { fraction, status }) => {
                    dialog.set_progress(fraction, &status);
                }
                Ok(SnapshotWriteMessage::Done(result)) => {
                    dialog.finish_and_close();

                    match result {
                        Ok(path) => {
                            if let Some(callback) = callback_ref.borrow_mut().take() {
                                callback(&win, path);
                            }
                        }
                        Err(msg) => {
                            if msg != "Operation cancelled" {
                                win.show_error(&msg);
                            } else {
                                win.show_toast(&gettext("Operation cancelled"));
                            }
                        }
                    }

                    return glib::ControlFlow::Break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    return glib::ControlFlow::Continue;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    dialog.finish_and_close();
                    return glib::ControlFlow::Break;
                }
            }
        });
    }

    fn temp_target_for_snapshot_node(&self, node: &TreeNode) -> Option<PathBuf> {
        let hash = self.imp().current_hash.borrow().clone()?;

        let file_name = node
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("snapshot-file");

        Some(
            std::env::temp_dir()
                .join("temporal-explorer")
                .join(short_hash(&hash))
                .join(file_name),
        )
    }

    #[allow(dead_code)]
    fn materialize_snapshot_node_to_temp(&self, node: &TreeNode) -> Option<PathBuf> {
        if node.is_dir() || node.is_submodule() {
            self.show_toast(&gettext(
                "Only files can be opened or exported from a snapshot",
            ));
            return None;
        }

        let hash = self.imp().current_hash.borrow().clone()?;
        let repo_ref = self.imp().repository.borrow();
        let repo = &repo_ref.as_ref()?.0;

        let obj = repo.revparse_single(&hash).ok()?;
        let commit = obj.peel_to_commit().ok()?;
        let tree = commit.tree().ok()?;
        let entry = tree.get_path(node.path()).ok()?;
        let blob = repo.find_blob(entry.id()).ok()?;

        let file_name = node
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("snapshot-file");

        let short = short_hash(&hash);
        let target_dir = std::env::temp_dir().join("temporal-explorer").join(short);

        std::fs::create_dir_all(&target_dir).ok()?;

        let target = target_dir.join(file_name);
        std::fs::write(&target, blob.content()).ok()?;

        Some(target)
    }

    fn open_snapshot_node_with_default_app(&self, node: &TreeNode) {
        let Some(target) = self.temp_target_for_snapshot_node(node) else {
            return;
        };

        self.write_snapshot_node_to_path_with_progress(
            node,
            target,
            &gettext("Opening File"),
            &gettext("Materializing snapshot file…"),
            |win, path| {
                let uri = format!("file://{}", path.to_string_lossy());

                if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
                    .is_err()
                {
                    win.show_error(&gettext("Could not open file with the default application"));
                }
            },
        );
    }

    fn open_snapshot_node_with_app_chooser(&self, node: &TreeNode) {
        let Some(target) = self.temp_target_for_snapshot_node(node) else {
            return;
        };

        self.write_snapshot_node_to_path_with_progress(
            node,
            target,
            &gettext("Preparing File"),
            &gettext("Materializing snapshot file…"),
            |win, path| {
                let win = win.clone();
                let parent = win.clone();
                let file = gio::File::for_path(&path);
                let launcher = gtk::FileLauncher::new(Some(&file));
                launcher.set_always_ask(true);
                launcher.launch(Some(&parent), None::<&gio::Cancellable>, move |result| {
                    if result.is_err() {
                        win.show_error(&gettext(
                            "Could not open file with the selected application",
                        ));
                    }
                });
            },
        );
    }

    fn export_snapshot_node(&self, node: &TreeNode) {
        if node.is_dir() || node.is_submodule() {
            self.show_toast(&gettext("Only files can be exported from a snapshot"));
            return;
        }

        let file_name = node
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("snapshot-file")
            .to_string();

        let dialog = gtk::FileDialog::builder()
            .title(gettext("Export File"))
            .modal(true)
            .accept_label(gettext("Export"))
            .initial_name(file_name)
            .build();

        let win = self.clone();
        let node = node.clone();

        dialog.save(Some(self), None::<&gio::Cancellable>, move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(target) = file.path() else {
                return;
            };

            win.write_snapshot_node_to_path_with_progress(
                &node,
                target,
                &gettext("Exporting File"),
                &gettext("Exporting snapshot file…"),
                move |win, path| {
                    win.show_toast(&format!("{}: {}", gettext("Exported"), path.display()));
                },
            );
        });
    }

    fn copy_repository_path(&self, node: &TreeNode) {
        let text = node.path().to_string_lossy().to_string();

        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&text);
            self.show_toast(&gettext("Repository path copied"));
        }
    }

    fn copy_snapshot_node_content(&self, node: &TreeNode) {
        if node.is_dir() || node.is_submodule() {
            self.show_toast(&gettext("Only file content can be copied"));
            return;
        }

        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            return;
        };

        let repo_ref = self.imp().repository.borrow();
        let Some(repo_wrapper) = repo_ref.as_ref() else {
            return;
        };
        let repo = &repo_wrapper.0;

        let content = repo
            .revparse_single(&hash)
            .ok()
            .and_then(|obj| obj.peel_to_commit().ok())
            .and_then(|commit| commit.tree().ok())
            .and_then(|tree| tree.get_path(node.path()).ok())
            .and_then(|entry| repo.find_blob(entry.id()).ok())
            .and_then(|blob| {
                std::str::from_utf8(blob.content())
                    .ok()
                    .map(ToOwned::to_owned)
            });

        if let Some(text) = content {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text);
                self.show_toast(&gettext("File content copied"));
            }
        } else {
            self.show_toast(&gettext(
                "This file is binary or could not be copied as text",
            ));
        }
    }

    fn show_node_in_system(&self, node: &TreeNode) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            return;
        };

        let working_path = repo_path.join(node.path());

        if !working_path.exists() {
            self.show_toast(&gettext(
                "This file does not exist in the current working tree",
            ));
            return;
        }

        let file = gio::File::for_path(&working_path);
        let uri = file.uri();

        if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>).is_err() {
            self.show_error(&gettext("Could not show file in the system"));
        }
    }

    fn show_node_properties(&self, node: &TreeNode) {
        let hash = self.imp().current_hash.borrow().clone().unwrap_or_default();
        let short = short_hash(&hash).to_string();
        let repo_name = self.imp().repo_name.borrow().clone();

        let snapshot_date = {
            let commits = self.imp().all_commits.borrow();
            let index = self.imp().commit_index.borrow();

            index
                .get(&hash)
                .and_then(|idx| commits.get(*idx))
                .map(|commit| Self::format_timestamp(commit.timestamp))
                .unwrap_or_else(|| gettext("Unknown snapshot date"))
        };

        let name = node
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let repository_path = node.path().display().to_string();

        let parent_folder = node
            .path()
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.display().to_string())
            .unwrap_or_else(|| {
                self.imp()
                    .repo_path
                    .borrow()
                    .clone()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| gettext("Repository root"))
            });

        let kind = if node.is_dir() {
            gettext("Folder")
        } else if node.is_submodule() {
            gettext("Submodule")
        } else {
            gettext("File")
        };

        let icon_name = if node.is_dir() {
            "folder-symbolic".to_string()
        } else if node.is_submodule() {
            "folder-remote-symbolic".to_string()
        } else {
            "text-x-generic-symbolic".to_string()
        };

        let mut git_object = gettext("Not available");
        let mut git_mode = gettext("Not available");
        let mut size = gettext("Not available");

        if let Some(hash) = self.imp().current_hash.borrow().clone() {
            let repo_ref = self.imp().repository.borrow();

            if let Some(repo_wrapper) = repo_ref.as_ref() {
                let repo = &repo_wrapper.0;

                if let Some((object_id, object_mode, object_size)) = repo
                    .revparse_single(&hash)
                    .ok()
                    .and_then(|obj| obj.peel_to_commit().ok())
                    .and_then(|commit| commit.tree().ok())
                    .and_then(|tree| tree.get_path(node.path()).ok())
                    .and_then(|entry| {
                        let oid = entry.id();
                        let mode = entry.filemode();

                        if matches!(node, TreeNode::File(_)) {
                            repo.find_blob(oid)
                                .ok()
                                .map(|blob| (oid.to_string(), mode, Some(blob.size() as u64)))
                        } else {
                            Some((oid.to_string(), mode, None))
                        }
                    })
                {
                    git_object = object_id;
                    git_mode = format_git_mode(object_mode);

                    if let Some(object_size) = object_size {
                        size = format_file_size(object_size);
                    }
                }
            }
        }

        let working_tree_status = self
            .imp()
            .repo_path
            .borrow()
            .clone()
            .map(|repo_path| repo_path.join(node.path()).exists())
            .unwrap_or(false);

        let system_status = if working_tree_status {
            gettext("Exists in working tree")
        } else {
            gettext("Only available in selected snapshot")
        };

        let props = NodeProperties {
            name,
            kind,
            icon_name,
            repository: repo_name,
            repository_path,
            parent_folder,
            snapshot_commit: short,
            full_commit: hash,
            snapshot_date,
            git_object,
            git_mode,
            size,
            system_status,
        };

        let dialog = NodePropertiesDialog::new();
        dialog.set_properties(&props);

        let win = self.clone();
        dialog.connect_favorite_toggled(move |_, active| {
            let msg = if active {
                gettext("Marked as favorite")
            } else {
                gettext("Removed from favorites")
            };
            win.show_toast(&msg);
        });

        let win = self.clone();
        let node_for_location = node.clone();
        dialog.connect_open_location_requested(move |_, _| {
            win.show_node_in_system(&node_for_location);
        });

        let win = self.clone();
        dialog.connect_icon_edit_requested(move |_| {
            win.show_toast(&gettext("Snapshot item icons cannot be edited"));
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    // ── Right-panel management    // ── Right-panel management ────────────────────────────────────────────────

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
        let hash = match self.imp().current_hash.borrow().clone() {
            Some(h) => h,
            None => return,
        };
        let repo_path = match self.imp().repo_path.borrow().clone() {
            Some(p) => p,
            None => return,
        };
        let file_path = path.to_path_buf();

        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(String, String), String>>(1);
        let worker_path = file_path.clone();
        let worker_hash = hash.clone();

        std::thread::spawn(move || {
            let result = git2::Repository::open(&repo_path)
                .map_err(|e| e.to_string())
                .map(|repo| file_preview::read_file_preview(&repo, &worker_hash, &worker_path));

            let _ = tx.send(result);
        });

        let win = self.clone();
        glib::idle_add_local(move || match rx.try_recv() {
            Ok(Ok((title, body))) => {
                file_preview::show_file_preview_text(&win, &title, &body, &hash, &file_path);
                glib::ControlFlow::Break
            }
            Ok(Err(e)) => {
                win.show_error(&format!("{}: {e}", gettext("Cannot open repository")));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        });
    }

    // ── Navigation helpers ────────────────────────────────────────────────────

    pub fn push_dir(&self, dir: PathBuf) {
        let imp = self.imp();
        let prev = imp.current_dir.borrow().clone();
        imp.history_back.borrow_mut().push(prev);
        imp.history_forward.borrow_mut().clear();
        self.navigate_to_dir(dir);
    }

    /// Navigate back one step in the directory history of the current snapshot.
    ///
    /// # Borrow safety
    ///
    /// All RefCell guards (`history_back`, `current_dir`, `history_forward`) are
    /// resolved into owned values inside a tightly-scoped block.  That block ends
    /// — and every guard is dropped — **before** `navigate_to_dir` is called.
    /// `navigate_to_dir` itself borrows `current_dir` mutably, so any live guard
    /// on it at call-time would cause a "RefCell already mutably borrowed" panic
    /// inside the non-unwinding glib closure marshaller (fatal SIGABRT).
    fn navigate_back(&self) {
        // ── Extract all needed values; every borrow guard drops at end of block ──
        let (dir_to_go, cur_dir) = {
            let imp = self.imp();
            let dir_to_go = imp.history_back.borrow_mut().pop();
            let cur_dir = imp.current_dir.borrow().clone();
            (dir_to_go, cur_dir)
            // imp, history_back borrow_mut, and current_dir borrow are all dropped here
        };

        if let Some(dir) = dir_to_go {
            self.imp().history_forward.borrow_mut().push(cur_dir);
            self.navigate_to_dir(dir);
        }
    }

    /// Navigate forward one step in the directory history of the current snapshot.
    ///
    /// # Borrow safety
    ///
    /// Mirrors `navigate_back`: all guards are resolved into owned locals before
    /// `navigate_to_dir` is called, preventing any live borrow aliasing.
    fn navigate_forward(&self) {
        // ── Extract all needed values; every borrow guard drops at end of block ──
        let (dir_to_go, cur_dir) = {
            let imp = self.imp();
            let dir_to_go = imp.history_forward.borrow_mut().pop();
            let cur_dir = imp.current_dir.borrow().clone();
            (dir_to_go, cur_dir)
            // imp, history_forward borrow_mut, and current_dir borrow are all dropped here
        };

        if let Some(dir) = dir_to_go {
            self.imp().history_back.borrow_mut().push(cur_dir);
            self.navigate_to_dir(dir);
        }
    }

    fn update_dir_nav_buttons(&self) {
        let imp = self.imp();
        let can_back = !imp.history_back.borrow().is_empty();
        let can_forward = !imp.history_forward.borrow().is_empty();
        imp.toolbar
            .history_controls()
            .set_sensitivity(can_back, can_forward);
    }

    // ── Location bar ─────────────────────────────────────────────────────────

    fn enter_location_mode(&self) {
        let imp = self.imp();
        let path_str = imp.current_dir.borrow().to_string_lossy().to_string();
        imp.toolbar.location_entry().set_text(&path_str);
        imp.toolbar.set_location_mode(true);
        imp.toolbar.location_entry().grab_focus();
    }

    fn leave_location_mode(&self) {
        self.imp().toolbar.set_location_mode(false);
    }

    fn navigate_to_typed_path(&self, text: &str) {
        self.imp().toolbar.set_location_mode(false);
        let path = PathBuf::from(text);
        self.push_dir(path);
    }

    // ── Timestamp formatting ──────────────────────────────────────────────────

    fn format_timestamp(ts: i64) -> String {
        glib::DateTime::from_unix_local(ts)
            .and_then(|dt| dt.format("%Y-%m-%d %H:%M"))
            .map(|s| s.to_string())
            .unwrap_or_else(|_| ts.to_string())
    }

    // ── Toast / error helpers ─────────────────────────────────────────────────

    pub fn show_toast(&self, msg: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(msg));
    }

    pub fn show_error(&self, msg: &str) {
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Error"))
            .body(msg)
            .build();
        dialog.add_response("ok", &gettext("OK"));
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    fn refresh_timeline_after_filter_change(&self) {
        let imp = self.imp();
        let level = *imp.timeline_level.borrow();
        let selected_year = imp.selected_year.get();

        match level {
            TimelineLevel::Years => {
                self.populate_year_list();
            }
            TimelineLevel::Months | TimelineLevel::Commits => {
                if selected_year > 0 {
                    let commits = imp.all_commits.borrow();
                    let visible = self.visible_timeline_commits(&commits);
                    let visible_owned: Vec<CommitInfo> =
                        visible.iter().map(|c| (*c).clone()).collect();

                    let year_has_commits =
                        !timeline_filter::commits_for_year(&visible_owned, selected_year)
                            .is_empty();

                    drop(commits);

                    if year_has_commits {
                        self.on_year_selected(selected_year);
                    } else {
                        self.populate_year_list();
                    }
                } else {
                    self.populate_year_list();
                }
            }
        }
    }

    // ── Search / filter ───────────────────────────────────────────────────────

    fn setup_filter_popover(&self) {
        let imp = self.imp();
        let popover = SearchFilterPopover::new();

        // Nautilus-like behavior:
        // the search filter popover is anchored to the search field in the top
        // toolbar, not to the timeline/sidebar search entry.
        let filter_button = imp.toolbar.search_filter_button().clone();

        popover.set_parent(&filter_button);

        filter_button.connect_toggled({
            let pop = popover.clone();
            move |btn| {
                if btn.is_active() {
                    pop.popup();
                } else {
                    pop.popdown();
                }
            }
        });

        popover.connect_closed({
            let btn = filter_button.downgrade();
            move |_| {
                if let Some(b) = btn.upgrade() {
                    b.set_active(false);
                }
            }
        });

        let win = self.clone();
        popover.connect_filters_changed(move |popover_ref| {
            let state = popover_ref.filter_state();
            *win.imp().filter_state.borrow_mut() = state;

            // Rebuild the timeline sidebar from the active FilterState.
            // Without this, changing Author/Date only updated the commit search
            // result list and left Years/Months stale.
            win.refresh_timeline_after_filter_change();

            let q = win.imp().last_query.borrow().clone();
            win.run_search(q);
        });

        *imp.filter_popover.borrow_mut() = Some(popover);
    }

    fn on_search_changed(&self, query: String) {
        *self.imp().last_query.borrow_mut() = query.clone();

        let trimmed = query.trim().to_string();

        if trimmed.is_empty() {
            // Search cleared: restore the normal timeline view using the already
            // loaded in-memory commit history. Do not reload the repository.
            if let Some(prev) = self.imp().search_debounce.borrow_mut().take() {
                prev.store(true, Ordering::Relaxed);
            }

            self.refresh_timeline_after_filter_change();
            return;
        }

        {
            let imp = self.imp();
            *imp.timeline_level.borrow_mut() = TimelineLevel::Commits;
            imp.selected_year.set(0);
            imp.timeline_stack.set_visible_child_name("commits");
            imp.timeline_back_button.set_visible(true);
            imp.timeline_header_title
                .set_title(&gettext("Search Results"));
            imp.timeline_header_title.set_subtitle("");
        }

        // Cancel any in-flight debounce.
        if let Some(prev) = self.imp().search_debounce.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }

        let token = Arc::new(AtomicBool::new(false));
        *self.imp().search_debounce.borrow_mut() = Some(token.clone());

        let win = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
            if token.load(Ordering::Relaxed) {
                return;
            }
            win.run_search(query);
        });
    }

    pub fn run_search(&self, query: String) {
        // Cancel any previous search before starting a new worker.
        if let Some(prev) = self.imp().search_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        *self.imp().search_cancel.borrow_mut() = Some(cancel.clone());

        let all_commits = self.imp().all_commits.borrow().clone();
        let filter = self.imp().filter_state.borrow().clone();
        let repo_path = self.imp().repo_path.borrow().clone();
        let changed_files_cache_snapshot = self.imp().changed_files_cache.borrow().clone();
        let branch_hashes = filter
            .branch
            .as_ref()
            .and_then(|branch| self.imp().branch_commit_index.borrow().get(branch).cloned());
        let q = query.to_lowercase();

        let file_filter_active = filter.files.is_active();

        let progress_dialog = if file_filter_active {
            let dialog = OperationProgressDialog::new();
            dialog.setup(
                &gettext("Indexing Changed Files"),
                &gettext("Reading changed-file metadata…"),
                cancel.clone(),
            );
            AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
            Some(dialog)
        } else {
            None
        };

        let total_commits = all_commits.len().max(1);
        let (tx, rx) = std::sync::mpsc::sync_channel::<SearchProgressMessage>(32);
        let worker_cancel = cancel.clone();

        std::thread::spawn(move || {
            let mut results = Vec::new();
            let mut changed_files_cache = changed_files_cache_snapshot;
            let mut newly_cached_changed_files: Vec<(String, Vec<String>)> = Vec::new();

            // For file-type filters, open the repository only once in this worker.
            // Changed files are cached by commit hash for the lifetime of the app.
            let repo_for_file_filter = if filter.files.is_active() {
                repo_path
                    .as_ref()
                    .and_then(|path| git2::Repository::open(path).ok())
            } else {
                None
            };

            for (idx, commit) in all_commits.into_iter().enumerate() {
                if worker_cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(SearchProgressMessage::Done(None));
                    return;
                }

                if file_filter_active && idx % 25 == 0 {
                    let _ = tx.send(SearchProgressMessage::Progress {
                        current: idx + 1,
                        total: total_commits,
                    });
                }

                // ── Author filter ─────────────────────────────────────────
                if let Some(ref author) = filter.author {
                    let wanted_author = author.to_lowercase();
                    if !commit.author.to_lowercase().contains(&wanted_author) {
                        continue;
                    }
                }

                // ── Branch filter ─────────────────────────────────────────
                if let Some(ref hashes) = branch_hashes {
                    if !hashes.contains(&commit.hash) {
                        continue;
                    }
                }

                // ── Date range filter ─────────────────────────────────────
                if let Some(since) = filter.date.from {
                    if commit.timestamp < since {
                        continue;
                    }
                }

                if let Some(until) = filter.date.to {
                    if commit.timestamp > until {
                        continue;
                    }
                }

                // ── File-type / extension filter ──────────────────────────
                if filter.files.is_active() {
                    let mut commit = commit;

                    // changed_files is loaded lazily in the UI. For file filters,
                    // load it inside the worker so the filter has real data.
                    if commit.changed_files.is_empty() {
                        if let Some(cached_files) = changed_files_cache.get(&commit.hash) {
                            commit.changed_files = cached_files.clone();
                        } else if let Some(ref repo) = repo_for_file_filter {
                            commit.load_changed_files(repo);

                            changed_files_cache
                                .insert(commit.hash.clone(), commit.changed_files.clone());

                            newly_cached_changed_files
                                .push((commit.hash.clone(), commit.changed_files.clone()));
                        }
                    }

                    let has_file_match = commit
                        .changed_files
                        .iter()
                        .any(|file| file_matches_search_category(file, &filter.files));

                    if !has_file_match {
                        continue;
                    }

                    results.push(commit);
                    continue;
                }

                // ── Text query ────────────────────────────────────────────
                if !q.is_empty()
                    && !commit.summary.to_lowercase().contains(&q)
                    && !commit.hash.starts_with(&q)
                    && !commit.author.to_lowercase().contains(&q)
                    && !matches_calendar(commit.timestamp, &q)
                {
                    continue;
                }

                results.push(commit);
            }

            let _ = tx.send(SearchProgressMessage::Done(Some((
                results,
                newly_cached_changed_files,
            ))));
        });

        let win = self.clone();
        glib::idle_add_local(move || {
            if cancel.load(Ordering::Relaxed) {
                if let Some(ref dialog) = progress_dialog {
                    dialog.finish_and_close();
                }
                return glib::ControlFlow::Break;
            }

            loop {
                match rx.try_recv() {
                    Ok(SearchProgressMessage::Progress { current, total }) => {
                        if let Some(ref dialog) = progress_dialog {
                            let fraction = current as f64 / total.max(1) as f64;
                            dialog.set_progress(
                                fraction,
                                &format!(
                                    "{} {}/{}",
                                    gettext("Reading changed-file metadata…"),
                                    current,
                                    total
                                ),
                            );
                        }
                    }

                    Ok(SearchProgressMessage::Done(Some((
                        results,
                        newly_cached_changed_files,
                    )))) => {
                        if let Some(ref dialog) = progress_dialog {
                            dialog.finish_and_close();
                        }

                        if !cancel.load(Ordering::Relaxed) {
                            if !newly_cached_changed_files.is_empty() {
                                let imp = win.imp();

                                {
                                    let mut cache = imp.changed_files_cache.borrow_mut();
                                    for (hash, files) in newly_cached_changed_files {
                                        cache.insert(hash.clone(), files.clone());
                                    }
                                }

                                {
                                    let cache = imp.changed_files_cache.borrow();
                                    let mut commits = imp.all_commits.borrow_mut();
                                    let index = imp.commit_index.borrow();

                                    for (hash, files) in cache.iter() {
                                        if let Some(idx) = index.get(hash) {
                                            if let Some(commit) = commits.get_mut(*idx) {
                                                if commit.changed_files.is_empty() {
                                                    commit.changed_files = files.clone();
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            commit_controller::populate_commit_list(
                                &win.imp().commit_list,
                                &results,
                            );
                        }

                        return glib::ControlFlow::Break;
                    }

                    Ok(SearchProgressMessage::Done(None)) => {
                        if let Some(ref dialog) = progress_dialog {
                            dialog.finish_and_close();
                        }
                        return glib::ControlFlow::Break;
                    }

                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return glib::ControlFlow::Continue;
                    }

                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(ref dialog) = progress_dialog {
                            dialog.finish_and_close();
                        }
                        return glib::ControlFlow::Break;
                    }
                }
            }
        });
    }
}
