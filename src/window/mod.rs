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
//! Every widget with an `id` in `ui/window.blp` **must** appear as a
//! `#[template_child]` in `imp::TemporalExplorerWindow`.  The only
//! exception is `SearchFilterPopover`, created at runtime and stored
//! as `RefCell<Option<…>>` because its parent (`filter_button`) is
//! not known at template-inflate time.
//!
//! ### Panel widgets declared in `.blp` and wired here
//!
//! | Widget id             | Blueprint file   | Rust field             |
//! |-----------------------|------------------|------------------------|
//! | `filter_button`       | `ui/window.blp`  | `#[template_child]`    |
//! | `commit_search_entry` | `ui/window.blp`  | `#[template_child]`    |
//! | `timeline_stack`      | `ui/window.blp`  | `#[template_child]`    |
//! | `tab_view`            | `ui/window.blp`  | `#[template_child]`    |
//! | `commit_info_bar`     | `ui/window.blp`  | `#[template_child]`    |
//! | `toolbar`             | `ui/window.blp`  | `#[template_child]`    |
//! | `new_branch_button`   | `ui/toolbar.blp` | via `TemporalToolbar`  |

use adw::prelude::{AdwDialogExt, AlertDialogExt};
use adw::subclass::prelude::*;
use gettextrs::gettext;
use glib::object::ObjectExt;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::address_bar;
use crate::batch_operations_dialog::{BatchOp, BatchOperationsDialog};
use crate::clone_repository_dialog::CloneRepositoryDialog;
use crate::column_chooser::{ColumnChooser, ColumnVisibility};
use crate::commit_controller;
use crate::commit_details_dialog::{CommitDetails, CommitDetailsDialog};
use crate::file_grid_captions_dialog::{CaptionFlags, FileGridCaptionsDialog};
use crate::file_preview;
use crate::filter_types_dialog::FilterTypesDialog;
use crate::git_engine::{
    build_commit_file_index, CommitFileIndex, CommitInfo, DirCache, FileIndexStore, HistoryReader,
    SnapshotResolver, TreeNode,
};
use crate::merge_conflict_dialog::{ConflictInfo, MergeConflictDialog};
use crate::new_branch_dialog::NewBranchDialog;
use crate::node_properties_dialog::{NodeProperties, NodePropertiesDialog};
use crate::operation_progress_dialog::OperationProgressDialog;
use crate::search_filter_popover::{FilterState, SearchFilterPopover};
use crate::select_commits_by_pattern::{commit_matches_pattern, SelectCommitsByPattern};
use crate::timeline_filter;
use crate::toolbar::TemporalToolbar;
use crate::view_controls::FileSortMode;
use crate::views::grid_view::{FileGridMetadata, GridZoom};
use crate::views::list_view::{OnEnterDir, OnOpenFile};
use crate::views::{grid_view, list_view};

mod tabs;
mod timeline;
mod navigation;
mod operations;
mod views;
mod context_menu;
mod files;
mod favorites;
mod util;
mod git_ops;


const SNAPSHOT_SEARCH_MAX_FILES: usize = 100_000;
const SNAPSHOT_SEARCH_MAX_FILE_SIZE: usize = 1024 * 1024;
const SNAPSHOT_SEARCH_MAX_MATCHES: usize = 200;
const SNAPSHOT_SEARCH_MAX_DEPTH: usize = 64;

#[derive(Clone, Debug, Default)]
pub(super) struct TabState {
    hash: Option<String>,
    oid: Option<git2::Oid>,
    dir: PathBuf,
    nodes: Vec<TreeNode>,
    history_back: Vec<PathBuf>,
    history_forward: Vec<PathBuf>,
    commit_nav_back: Vec<String>,
    commit_nav_forward: Vec<String>,
}

#[derive(Debug, Clone)]
struct FileFilterResultsCache {
    filter: FilterState,
    query: String,
    scope: Option<(i32, u32)>,
    result_indices: Vec<usize>,
    file_match_counts: HashMap<git2::Oid, usize>,
}

/// Whether the right panel renders files as a list or a grid.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ViewMode {
    #[default]
    List,
    Grid,
}

/// The currently visible drill-down level of the left timeline panel.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum TimelineLevel {
    #[default]
    Years,
    Months,
    Commits,
}

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

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/TemporalExplorer/window.ui")]
    pub struct TemporalExplorerWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub toolbar: TemplateChild<TemporalToolbar>,
        #[template_child]
        pub timeline_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub timeline_back_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub sidebar_search_button: TemplateChild<gtk::ToggleButton>,
        #[template_child]
        pub sidebar_search_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub sidebar_menu_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub timeline_header_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub year_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub month_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub commit_search_entry: TemplateChild<gtk::SearchEntry>,

        #[template_child]
        pub filter_button: TemplateChild<gtk::ToggleButton>,

        #[template_child]
        pub commit_list: TemplateChild<gtk::ListBox>,

        // SearchFilterPopover cannot be a TemplateChild — its type is not
        // registered in Blueprint at template-inflate time, so it is created
        // in setup_filter_popover() and stored here.
        pub filter_popover: RefCell<Option<SearchFilterPopover>>,

        #[template_child]
        pub content_toolbar_view: TemplateChild<adw::ToolbarView>,
        #[template_child]
        pub tab_bar: TemplateChild<adw::TabBar>,
        #[template_child]
        pub tab_view: TemplateChild<adw::TabView>,
        #[template_child]
        pub tab_overview: TemplateChild<adw::TabOverview>,
        #[template_child]
        pub split_view: TemplateChild<adw::OverlaySplitView>,

        #[template_child]
        pub commit_info_bar: TemplateChild<gtk::ActionBar>,
        #[template_child]
        pub commit_hash_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub commit_message_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub commit_date_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub mobile_controls_bar: TemplateChild<gtk::ActionBar>,
        #[template_child]
        pub sidebar_bottom_bar: TemplateChild<gtk::ActionBar>,
        #[template_child]
        pub progress_indicator_button: TemplateChild<gtk::MenuButton>,
        #[template_child]
        pub progress_indicator_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub operations_popover: TemplateChild<gtk::Popover>,
        #[template_child]
        pub operations_list: TemplateChild<gtk::ListBox>,
        #[template_child]
        pub operation_status_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub operation_progress_bar: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub operation_cancel_button: TemplateChild<gtk::Button>,

        pub settings: OnceCell<gio::Settings>,
        pub all_commits: RefCell<Vec<CommitInfo>>,
        pub commit_index: RefCell<HashMap<String, usize>>,
        pub year_counts: RefCell<Vec<(i32, usize)>>,
        pub branch_commit_index: RefCell<HashMap<String, HashSet<String>>>,
        pub seen_authors: RefCell<HashSet<String>>,
        pub changed_files_cache: RefCell<HashMap<String, Vec<String>>>,
        pub file_index_store: RefCell<Option<FileIndexStore>>,
        pub(super) file_filter_results_cache: RefCell<Option<FileFilterResultsCache>>,
        pub repo_path: RefCell<Option<PathBuf>>,
        pub repository: RefCell<Option<DebugRepository>>,
        pub last_query: RefCell<String>,
        pub current_hash: RefCell<Option<String>>,
        pub current_oid: Cell<Option<git2::Oid>>,
        pub current_dir: RefCell<PathBuf>,
        pub current_nodes: RefCell<Vec<TreeNode>>,
        pub history_back: RefCell<Vec<PathBuf>>,
        pub history_forward: RefCell<Vec<PathBuf>>,
        pub view_mode: RefCell<ViewMode>,
        pub repo_name: RefCell<String>,

        pub commit_nav_back: RefCell<Vec<String>>,
        pub commit_nav_forward: RefCell<Vec<String>>,

        pub sort_mode: RefCell<FileSortMode>,
        pub grid_zoom: RefCell<GridZoom>,
        pub grid_caption_flags: RefCell<CaptionFlags>,
        pub column_visibility: RefCell<ColumnVisibility>,
        pub show_hidden_files: Cell<bool>,

        pub timeline_level: RefCell<TimelineLevel>,
        pub selected_year: Cell<i32>,
        pub selected_month: Cell<u32>,
        pub loading_commits: Cell<bool>,
        pub dir_cache: RefCell<DirCache>,
        pub search_debounce: RefCell<Option<Arc<AtomicBool>>>,
        pub search_cancel: RefCell<Option<Arc<AtomicBool>>>,

        pub filter_state: RefCell<FilterState>,
        pub load_cancel: RefCell<Option<Arc<AtomicBool>>>,
        pub context_node: RefCell<Option<TreeNode>>,
        pub active_context_popover: RefCell<Option<gtk::PopoverMenu>>,
        pub(super) tab_states: RefCell<HashMap<usize, TabState>>,
        pub(super) closed_tabs: RefCell<Vec<(TabState, String)>>,
        pub pending_favorite: RefCell<Option<(PathBuf, String, PathBuf)>>,
        pub(super) pending_tab_state: RefCell<Option<(TabState, String)>>,
        pub operation_cancel: RefCell<Option<Arc<AtomicBool>>>,
        pub operation_history: RefCell<Vec<String>>,
        pub(super) switching_tab: Cell<bool>,
        pub(super) last_window_width: Cell<i32>,
        pub(super) last_window_height: Cell<i32>,
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
                .set(gio::Settings::new("io.github.TemporalExplorer"))
                .ok();
            self.obj().restore_window_state();
            self.obj().setup_callbacks();
            self.obj().setup_styles();
        }
    }

    impl WidgetImpl for TemporalExplorerWindow {}
    impl WindowImpl for TemporalExplorerWindow {
        fn close_request(&self) -> glib::Propagation {
            if let Some(settings) = self.settings.get() {
                let width = self.last_window_width.get().max(500);
                let height = self.last_window_height.get().max(400);
                settings.set_int("window-width", width).ok();
                settings.set_int("window-height", height).ok();
                settings
                    .set_boolean("window-maximized", self.obj().is_maximized())
                    .ok();
                gio::Settings::sync();
            }
            self.parent_close_request()
        }
    }
    impl ApplicationWindowImpl for TemporalExplorerWindow {}
    impl AdwApplicationWindowImpl for TemporalExplorerWindow {}
}

glib::wrapper! {
pub struct TemporalExplorerWindow(ObjectSubclass<imp::TemporalExplorerWindow>)
@extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
@implements
gio::ActionGroup, gio::ActionMap,
gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl TemporalExplorerWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    fn restore_window_state(&self) {
        let Some(settings) = self.imp().settings.get() else {
            return;
        };
        // Same minimum size as the Nautilus (360x294).
        let width = settings.int("window-width").max(360);
        let height = settings.int("window-height").max(294);
        self.set_default_size(width, height);
        self.imp().last_window_width.set(width);
        self.imp().last_window_height.set(height);

        let win = self.clone();
        self.connect_notify_local(Some("width"), move |_, _| {
            if !win.is_maximized() && win.width() >= 360 && win.height() >= 294 {
                win.imp().last_window_width.set(win.width());
                win.imp().last_window_height.set(win.height());
            }
        });
        let win = self.clone();
        self.connect_notify_local(Some("height"), move |_, _| {
            if !win.is_maximized() && win.width() >= 500 && win.height() >= 400 {
                win.imp().last_window_width.set(win.width());
                win.imp().last_window_height.set(win.height());
            }
        });

        if settings.boolean("window-maximized") {
            self.set_maximized(true);
        }
    }

    fn setup_callbacks(&self) {
        let imp = self.imp();

        self.setup_tabs();

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

        // The visibility of the history and view controls is driven by the
        // window `Adw.Breakpoint` (Nautilus parity), which toggles the
        // `toolbar.show-history-controls` and `toolbar.show-view-controls`
        // properties.

        // new_branch_button fires win.new-branch via action-name in Blueprint;
        // start insensitive until a repository is loaded.
        imp.toolbar.new_branch_button().set_sensitive(false);

        let win_g = self.clone();
        let gesture = gtk::GestureClick::new();
        gesture.set_button(1);
        gesture.connect_pressed(move |gesture, _n, x, y| {
            let Some(widget) = gesture.widget() else {
                return;
            };
            let clicked_empty_pathbar_area = widget
                .pick(x, y, gtk::PickFlags::DEFAULT)
                .is_some_and(|picked| picked == widget);

            if clicked_empty_pathbar_area {
                win_g.enter_location_mode();
            }
        });
        imp.toolbar.address_bar().add_controller(gesture);

        let address_bar_scrolled = imp.toolbar.address_bar_scrolled().clone();
        let pathbar_scroll =
            gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        pathbar_scroll.connect_scroll(move |_, _, dy| {
            if dy == 0.0 {
                return gtk::glib::Propagation::Proceed;
            }

            let adjustment = address_bar_scrolled.hadjustment();
            let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
            let next = (adjustment.value() + dy * adjustment.step_increment())
                .clamp(adjustment.lower(), max);
            adjustment.set_value(next);

            gtk::glib::Propagation::Stop
        });
        imp.toolbar
            .address_bar_scrolled()
            .add_controller(pathbar_scroll);

        let win = self.clone();
        imp.toolbar.location_entry().connect_activate(move |entry| {
            win.navigate_to_typed_path(entry.text().as_str());
        });

        let win = self.clone();
        imp.toolbar.location_cancel_btn().connect_clicked(move |_| {
            win.leave_location_mode();
        });

        let win = self.clone();
        let location_focus = gtk::EventControllerFocus::new();
        location_focus.connect_leave(move |_| {
            if !win.imp().toolbar.location_entry().has_focus() {
                win.leave_location_mode();
            }
        });
        imp.toolbar.location_entry().add_controller(location_focus);

        let win = self.clone();
        imp.toolbar.search_button().connect_toggled(move |button| {
            if button.is_active() {
                win.enter_search_mode();
            } else if !win.imp().commit_search_entry.text().is_empty() {
                win.leave_search_mode();
            }
        });

        let win = self.clone();
        imp.sidebar_search_button.connect_toggled(move |button| {
            if button.is_active() {
                win.enter_search_mode();
            } else if !win.imp().commit_search_entry.text().is_empty() {
                win.leave_search_mode();
            }
        });

        let win = self.clone();
        imp.tab_overview.connect_create_tab(move |_| {
            win.new_tab_page()
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

                win.on_search_changed(query);
            });

        self.setup_filter_popover();
        self.setup_history_controls();
        self.setup_view_controls();
        self.setup_saved_view_preferences();
        self.setup_settings_change_listeners();
        self.setup_actions();
    }

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
            ("edit-location", Self::edit_location),
            ("prompt-root-location", Self::prompt_root_location),
            ("copy-current-location", Self::copy_current_location),
            ("open-repository", Self::open_repo_dialog),
            ("clone-repository", Self::show_clone_repository_dialog),
            ("batch-operations", Self::show_batch_operations_dialog),
            ("select-by-pattern", Self::show_select_by_pattern_dialog),
            ("filter-file-type", Self::show_filter_types_dialog),
            ("show-column-chooser", Self::show_column_chooser),
            ("new-branch", Self::show_new_branch_dialog),
            ("new-window", Self::new_window),
            ("new-tab", Self::new_tab),
            ("close-tab", Self::close_current_tab),
            ("move-tab-left", Self::move_tab_left),
            ("move-tab-right", Self::move_tab_right),
            ("close-other-tabs", Self::close_other_tabs),
            ("restore-closed-tab", Self::restore_closed_tab),
            ("open-tab-overview", Self::open_tab_overview),
            ("move-tab-to-new-window", Self::move_tab_to_new_window),
            ("show-favorites", Self::show_favorites),
            ("search-snapshot-content", Self::search_snapshot_content),
            ("show-operation-panel", Self::show_operation_panel),
            ("show-operation-history", Self::show_operation_history),
            ("cancel-operation", Self::cancel_operation),
            ("close-operation-panel", Self::close_operation_panel),
            ("navigate-back", Self::navigate_back),
            ("navigate-forward", Self::navigate_forward),
            (
                "restore-project-snapshot",
                Self::confirm_restore_project_snapshot,
            ),
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
        let imp = self.imp();
        imp.sidebar_menu_button.insert_action_group("win", Some(self));

        if let Some(app) = self.application() {
            imp.sidebar_menu_button.insert_action_group("app", Some(&app));

            app.set_accels_for_action("win.reload-repository", &["F5"]);
            app.set_accels_for_action("win.new-window", &["<Control>n"]);
            app.set_accels_for_action("win.new-tab", &["<Control>t"]);
            app.set_accels_for_action("win.close-tab", &["<Control>w"]);
            app.set_accels_for_action("win.edit-location", &["<Control>l"]);
            app.set_accels_for_action("win.prompt-root-location", &["slash", "KP_Divide"]);
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
}

impl TemporalExplorerWindow {

    fn setup_styles(&self) {
        let provider = gtk::CssProvider::new();
        provider.load_from_resource("/io/github/TemporalExplorer/temporal-explorer.css");
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

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

}
