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

//! Main application window.
//!
//! This module is intentionally kept as an *orchestrator*: it owns the
//! GObject subclass boilerplate, wires GTK signals, and delegates all
//! widget-construction work to the purpose-built sub-modules:
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
//!
//! ## Right-panel layout
//!
//! `content_toolbar_view` has a permanent `Stack` child (`right_panel_stack`)
//! with two named pages:
//!
//! ```
//! right_panel_stack
//!   ├── "empty"   → adw::StatusPage (never re-parented)
//!   └── "content" → gtk::Box right_panel_content  (dynamic views appended here)
//! ```
//!
//! `replace_right_panel` clears `right_panel_content`, appends the new widget,
//! and flips the stack to "content".  `show_empty_state` clears the box and
//! flips back to "empty".  `set_content()` on `content_toolbar_view` is never
//! called at runtime, so the "parent must be NULL" assertion can never fire.
//!
//! ## Timeline navigation
//!
//! The left sidebar is a 3-level drill-down:
//!
//! ```
//! years  →  months (for selected year)  →  commits (for selected month)
//! ```
//!
//! The active level is tracked by [`TimelineLevel`] stored in
//! `imp.timeline_level`.  The back button (`timeline_back_button`) pops one
//! level; the `timeline_stack` `Stack` slides between the three pages.
//!
//! ## Search scope
//!
//! The search (`on_search_changed`) filters the `all_commits` in-memory
//! cache.  When a year is selected (`selected_year != 0`) the results are
//! scoped to that year.  When no year is selected (Years screen or after
//! pressing Back to the years level) the search spans all commits.

use adw::prelude::AdwApplicationWindowExt;
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
use crate::timeline_filter;
use crate::views::{list_view, grid_view};

// ── ViewMode ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ViewMode { #[default] List, Grid }

// ── TimelineLevel ──────────────────────────────────────────────────────────────────────────

/// Which page of the sidebar `timeline_stack` is currently visible.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum TimelineLevel {
    #[default]
    Years,
    Months,
    Commits,
}

// ── DebugRepository ─────────────────────────────────────────────────────────────────────────

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

// ── Private implementation ────────────────────────────────────────────────────────────────

mod imp {
    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/johnpetersa19/TemporalExplorer/window.ui")]
    pub struct TemporalExplorerWindow {
        // Header
        #[template_child] pub open_repo_button:     TemplateChild<gtk::Button>,
        #[template_child] pub nav_back_button:      TemplateChild<gtk::Button>,
        #[template_child] pub nav_forward_button:   TemplateChild<gtk::Button>,
        #[template_child] pub view_toggle_button:   TemplateChild<gtk::Button>,
        #[template_child] pub show_sidebar_button:  TemplateChild<gtk::ToggleButton>,
        #[template_child] pub window_title:         TemplateChild<adw::WindowTitle>,

        // Nautilus-style toolbar_switcher Stack
        #[template_child] pub toolbar_switcher:     TemplateChild<gtk::Stack>,
        #[template_child] pub address_bar:          TemplateChild<gtk::Box>,
        #[template_child] pub location_entry:       TemplateChild<gtk::Entry>,
        #[template_child] pub location_cancel_btn:  TemplateChild<gtk::Button>,

        // Left panel — timeline drill-down
        #[template_child] pub timeline_stack:        TemplateChild<gtk::Stack>,
        #[template_child] pub timeline_back_button:  TemplateChild<gtk::Button>,
        #[template_child] pub timeline_header_title: TemplateChild<adw::WindowTitle>,
        #[template_child] pub year_list:             TemplateChild<gtk::ListBox>,
        #[template_child] pub month_list:            TemplateChild<gtk::ListBox>,
        #[template_child] pub commit_search_entry:   TemplateChild<gtk::SearchEntry>,
        #[template_child] pub commit_list:           TemplateChild<gtk::ListBox>,

        // Right panel — permanent stack (never re-parented)
        #[template_child] pub content_toolbar_view: TemplateChild<adw::ToolbarView>,
        #[template_child] pub right_panel_stack:    TemplateChild<gtk::Stack>,
        #[template_child] pub right_panel_content:  TemplateChild<gtk::Box>,
        #[template_child] pub empty_state:          TemplateChild<adw::StatusPage>,
        #[template_child] pub split_view:           TemplateChild<adw::OverlaySplitView>,

        // Bottom bar
        #[template_child] pub commit_info_bar:      TemplateChild<gtk::ActionBar>,
        #[template_child] pub commit_hash_label:    TemplateChild<gtk::Label>,
        #[template_child] pub commit_message_label: TemplateChild<gtk::Label>,
        #[template_child] pub commit_date_label:    TemplateChild<gtk::Label>,

        // Runtime state
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

        pub timeline_level:   RefCell<TimelineLevel>,
        pub selected_year:    Cell<i32>,
        pub loading_commits:  Cell<bool>,
        pub dir_cache:        RefCell<DirCache>,
        pub search_debounce:  RefCell<Option<glib::SourceId>>,
        pub search_cancel:    RefCell<Option<Arc<AtomicBool>>>,
        pub load_cancel:      RefCell<Option<Arc<AtomicBool>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TemporalExplorerWindow {
        const NAME: &'static str = "TemporalExplorerWindow";
        type Type = super::TemporalExplorerWindow;
        type ParentType = adw::ApplicationWindow;
        fn class_init(klass: &mut Self::Class) { klass.bind_template(); }
        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) { obj.init_template(); }
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

// ── Public wrapper ──────────────────────────────────────────────────────────────────────────

glib::wrapper! {
    pub struct TemporalExplorerWindow(ObjectSubclass<imp::TemporalExplorerWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements
            gio::ActionGroup, gio::ActionMap,
            gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget,
            gtk::Native, gtk::Root, gtk::ShortcutManager;
}

// ── Free helper ────────────────────────────────────────────────────────────────────────────

/// Removes all children from a `gtk::Box` safely.
///
/// Snapshots the child list into a `Vec` before calling any `unparent()`
/// to prevent iterator-invalidation races with pending idle frames.
fn clear_box(container: &gtk::Box) {
    let mut children: Vec<gtk::Widget> = Vec::new();
    let mut w = container.first_child();
    while let Some(child) = w {
        w = child.next_sibling();
        children.push(child);
    }
    for child in children {
        child.unparent();
    }
}

impl TemporalExplorerWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    // ── Callback wiring ────────────────────────────────────────────────────────────────

    fn setup_callbacks(&self) {
        let imp = self.imp();

        // Open repository button
        let win = self.clone();
        imp.open_repo_button.connect_clicked(move |_| {
            win.open_repo_dialog();
        });

        // Nav back
        let win = self.clone();
        imp.nav_back_button.connect_clicked(move |_| {
            win.navigate_back();
        });

        // Nav forward
        let win = self.clone();
        imp.nav_forward_button.connect_clicked(move |_| {
            win.navigate_forward();
        });

        // View-mode toggle
        let win = self.clone();
        imp.view_toggle_button.connect_clicked(move |_| {
            win.toggle_view_mode();
        });

        // Address bar — activate entry on click
        let win = self.clone();
        imp.address_bar.connect_button_press_event(move |_, event| {
            if event.button() == 1 {
                win.enter_location_mode();
            }
            glib::Propagation::Proceed
        });

        // Location entry — confirm with Enter
        let win = self.clone();
        imp.location_entry.connect_activate(move |entry| {
            win.navigate_to_typed_path(entry.text().as_str());
        });

        // Location cancel
        let win = self.clone();
        imp.location_cancel_btn.connect_clicked(move |_| {
            win.leave_location_mode();
        });

        // Timeline back button
        let win = self.clone();
        imp.timeline_back_button.connect_clicked(move |_| {
            win.timeline_pop();
        });

        // Year list row activated
        let win = self.clone();
        imp.year_list.connect_row_activated(move |_, row| {
            let year = row.index();  // index == year offset; actual year stored in widget data
            if let Some(y) = row.data::<i32>("year") {
                win.on_year_selected(unsafe { *y.as_ref() });
            } else {
                win.on_year_selected(year);
            }
        });

        // Month list row activated
        let win = self.clone();
        imp.month_list.connect_row_activated(move |_, row| {
            if let Some(m) = row.data::<u32>("month") {
                win.on_month_selected(unsafe { *m.as_ref() });
            }
        });

        // Commit list row activated
        let win = self.clone();
        imp.commit_list.connect_row_activated(move |_, row| {
            if let Some(h) = row.data::<String>("hash") {
                win.on_commit_selected(unsafe { h.as_ref().clone() });
            }
        });

        // Search entry
        let win = self.clone();
        imp.commit_search_entry.connect_search_changed(move |entry| {
            win.on_search_changed(entry.text().to_string());
        });
    }

    // ── CSS loader ────────────────────────────────────────────────────────────────────────
    //
    // All application CSS lives in `src/temporal-explorer.css` (bundled via
    // GResource).  Nothing is hard-coded here as a string literal.

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

    // ── Repository opening ─────────────────────────────────────────────────────────────────

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
                let repo_name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("repository")
                    .to_string();

                *self.imp().repo_path.borrow_mut() = Some(path.clone());
                *self.imp().repository.borrow_mut() = Some(DebugRepository(repo));
                *self.imp().repo_name.borrow_mut() = repo_name.clone();
                *self.imp().current_dir.borrow_mut() = PathBuf::new();
                self.imp().history_back.borrow_mut().clear();
                self.imp().history_forward.borrow_mut().clear();

                self.imp().window_title.set_title(&repo_name);
                self.imp().window_title.set_subtitle(
                    path.to_str().unwrap_or(""),
                );

                self.load_timeline(cancel);
            }
            Err(e) => {
                self.show_error(&format!("Failed to open repository: {e}"));
            }
        }
    }

    // ── Timeline loading ───────────────────────────────────────────────────────────────────

    fn load_timeline(&self, cancel: Arc<AtomicBool>) {
        let repo_path = match self.imp().repo_path.borrow().clone() {
            Some(p) => p,
            None => return,
        };

        self.imp().loading_commits.set(true);
        self.show_empty_state();

        let (tx, rx) = glib::MainContext::channel(glib::Priority::DEFAULT);

        std::thread::spawn(move || {
            let commits = HistoryReader::read_all(&repo_path, &cancel);
            let _ = tx.send(commits);
        });

        let win = self.clone();
        rx.attach(None, move |commits| {
            win.imp().loading_commits.set(false);
            match commits {
                Ok(list) => {
                    *win.imp().all_commits.borrow_mut() = list;
                    win.populate_year_list();
                    win.imp().split_view.set_show_sidebar(true);
                }
                Err(e) => {
                    win.show_error(&format!("Failed to read history: {e}"));
                }
            }
            glib::ControlFlow::Break
        });
    }

    // ── Year list ─────────────────────────────────────────────────────────────────────────

    fn populate_year_list(&self) {
        let imp = self.imp();
        let list = &imp.year_list;

        // Clear existing rows
        while let Some(child) = list.first_child() {
            child.unparent();
        }

        let commits = imp.all_commits.borrow();
        let years = timeline_filter::group_by_year(&commits);

        for (year, count) in &years {
            let row = commit_controller::build_year_row(*year, *count);
            list.append(&row);
        }

        *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
        imp.timeline_stack.set_visible_child_name("years");
        imp.timeline_back_button.set_visible(false);
        imp.timeline_header_title.set_title(&gettext("Timeline"));
        imp.timeline_header_title.set_subtitle("");
    }

    // ── Year selected ──────────────────────────────────────────────────────────────────────

    fn on_year_selected(&self, year: i32) {
        let imp = self.imp();
        imp.selected_year.set(year);

        let list = &imp.month_list;
        while let Some(child) = list.first_child() {
            child.unparent();
        }

        let commits = imp.all_commits.borrow();
        let months = timeline_filter::group_by_month(&commits, year);

        for (month, count) in &months {
            let row = commit_controller::build_month_row(*month, *count);
            list.append(&row);
        }

        *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
        imp.timeline_stack.set_visible_child_name("months");
        imp.timeline_back_button.set_visible(true);
        imp.timeline_header_title.set_title(&year.to_string());
        imp.timeline_header_title.set_subtitle("");
    }

    // ── Month selected ─────────────────────────────────────────────────────────────────────

    fn on_month_selected(&self, month: u32) {
        let imp = self.imp();
        let year = imp.selected_year.get();

        let list = &imp.commit_list;
        while let Some(child) = list.first_child() {
            child.unparent();
        }

        let commits = imp.all_commits.borrow().clone();
        let filtered = timeline_filter::filter_by_month(&commits, year, month);

        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(prev) = imp.search_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }
        *imp.search_cancel.borrow_mut() = Some(cancel.clone());

        commit_controller::populate_commit_list(
            list.clone(), filtered, cancel,
        );

        *imp.timeline_level.borrow_mut() = TimelineLevel::Commits;
        imp.timeline_stack.set_visible_child_name("commits");
        imp.timeline_header_title.set_subtitle(&format!(
            "{} {}",
            timeline_filter::month_name(month),
            year,
        ));
    }

    // ── Timeline back ──────────────────────────────────────────────────────────────────────

    fn timeline_pop(&self) {
        let imp = self.imp();
        let level = *imp.timeline_level.borrow();
        match level {
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

    // ── Commit selected ────────────────────────────────────────────────────────────────────

    fn on_commit_selected(&self, hash: String) {
        let imp = self.imp();

        *imp.current_hash.borrow_mut() = Some(hash.clone());
        *imp.current_dir.borrow_mut() = PathBuf::new();
        imp.history_back.borrow_mut().clear();
        imp.history_forward.borrow_mut().clear();

        // Update bottom bar
        let commits = imp.all_commits.borrow();
        if let Some(commit) = commits.iter().find(|c| c.hash.starts_with(&hash[..7.min(hash.len())])) {
            imp.commit_hash_label.set_label(&commit.hash[..8.min(commit.hash.len())]);
            imp.commit_message_label.set_label(&commit.summary);
            imp.commit_date_label.set_label(&Self::format_timestamp(commit.timestamp));
            imp.commit_info_bar.set_revealed(true);
        }

        self.navigate_to_dir(PathBuf::new());
    }

    // ── Directory navigation ───────────────────────────────────────────────────────────────

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

        *imp.current_dir.borrow_mut() = dir.clone();
        address_bar::rebuild(self, &dir);
        self.update_nav_buttons();

        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(prev) = imp.load_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }
        *imp.load_cancel.borrow_mut() = Some(cancel.clone());

        let (tx, rx) = glib::MainContext::channel(glib::Priority::DEFAULT);
        let dir_clone = dir.clone();

        std::thread::spawn(move || {
            let result = SnapshotResolver::list_dir(&repo_path, &hash, &dir_clone, &cancel);
            let _ = tx.send(result);
        });

        let win = self.clone();
        rx.attach(None, move |result| {
            match result {
                Ok(nodes) => win.render_dir(nodes),
                Err(e) => win.show_error(&format!("Error reading tree: {e}")),
            }
            glib::ControlFlow::Break
        });
    }

    fn render_dir(&self, nodes: Vec<TreeNode>) {
        let imp = self.imp();
        let mode = *imp.view_mode.borrow();

        let widget: gtk::Widget = match mode {
            ViewMode::List => list_view::build(self, &nodes).upcast(),
            ViewMode::Grid => grid_view::build(self, &nodes).upcast(),
        };

        self.replace_right_panel(widget);
    }

    // ── Right-panel management ─────────────────────────────────────────────────────────────

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

    // ── File preview ───────────────────────────────────────────────────────────────────────

    pub fn preview_file(&self, path: &std::path::Path) {
        let hash = match self.imp().current_hash.borrow().clone() {
            Some(h) => h,
            None => return,
        };
        let repo_path = match self.imp().repo_path.borrow().clone() {
            Some(p) => p,
            None => return,
        };

        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = glib::MainContext::channel(glib::Priority::DEFAULT);
        let path_clone = path.to_path_buf();

        std::thread::spawn(move || {
            let result = SnapshotResolver::read_file(&repo_path, &hash, &path_clone, &cancel);
            let _ = tx.send((path_clone, result));
        });

        let win = self.clone();
        rx.attach(None, move |(p, result)| {
            match result {
                Ok(bytes) => file_preview::show(&win, &p, bytes),
                Err(e)    => win.show_error(&format!("Cannot preview file: {e}")),
            }
            glib::ControlFlow::Break
        });
    }

    // ── Navigation helpers ─────────────────────────────────────────────────────────────────

    pub fn push_dir(&self, dir: PathBuf) {
        let imp = self.imp();
        let prev = imp.current_dir.borrow().clone();
        imp.history_back.borrow_mut().push(prev);
        imp.history_forward.borrow_mut().clear();
        self.navigate_to_dir(dir);
    }

    fn navigate_back(&self) {
        let imp = self.imp();
        let prev = imp.history_back.borrow_mut().pop();
        if let Some(dir) = prev {
            let cur = imp.current_dir.borrow().clone();
            imp.history_forward.borrow_mut().push(cur);
            self.navigate_to_dir(dir);
        }
    }

    fn navigate_forward(&self) {
        let imp = self.imp();
        let next = imp.history_forward.borrow_mut().pop();
        if let Some(dir) = next {
            let cur = imp.current_dir.borrow().clone();
            imp.history_back.borrow_mut().push(cur);
            self.navigate_to_dir(dir);
        }
    }

    fn update_nav_buttons(&self) {
        let imp = self.imp();
        imp.nav_back_button.set_sensitive(!imp.history_back.borrow().is_empty());
        imp.nav_forward_button.set_sensitive(!imp.history_forward.borrow().is_empty());
    }

    // ── Location bar (Nautilus-style) ──────────────────────────────────────────────────────

    fn enter_location_mode(&self) {
        let imp = self.imp();
        let current = imp.current_dir.borrow();
        imp.location_entry.set_text(current.to_str().unwrap_or(""));
        imp.location_entry.grab_focus();
        imp.toolbar_switcher.set_visible_child_name("location");
    }

    fn leave_location_mode(&self) {
        self.imp().toolbar_switcher.set_visible_child_name("address");
    }

    fn navigate_to_typed_path(&self, text: &str) {
        self.leave_location_mode();
        let path = PathBuf::from(text.trim());
        self.push_dir(path);
    }

    // ── View mode toggle ───────────────────────────────────────────────────────────────────

    fn toggle_view_mode(&self) {
        let imp = self.imp();
        let new_mode = match *imp.view_mode.borrow() {
            ViewMode::List => {
                imp.view_toggle_button.set_icon_name("view-list-symbolic");
                ViewMode::Grid
            }
            ViewMode::Grid => {
                imp.view_toggle_button.set_icon_name("view-grid-symbolic");
                ViewMode::List
            }
        };
        *imp.view_mode.borrow_mut() = new_mode;

        // Re-render current directory
        let dir = imp.current_dir.borrow().clone();
        if imp.current_hash.borrow().is_some() {
            self.navigate_to_dir(dir);
        }
    }

    // ── Search ─────────────────────────────────────────────────────────────────────────────

    fn on_search_changed(&self, query: String) {
        let imp = self.imp();

        // Debounce: cancel previous scheduled search
        if let Some(source) = imp.search_debounce.borrow_mut().take() {
            source.remove();
        }

        let win = self.clone();
        let source = glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
            win.run_search(query.clone());
            glib::ControlFlow::Break
        });
        *imp.search_debounce.borrow_mut() = Some(source);
    }

    fn run_search(&self, query: String) {
        let imp = self.imp();
        *imp.last_query.borrow_mut() = query.clone();

        // Cancel any previous search
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(prev) = imp.search_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }
        *imp.search_cancel.borrow_mut() = Some(cancel.clone());

        let list = imp.commit_list.clone();
        // Clear current rows
        while let Some(child) = list.first_child() {
            child.unparent();
        }

        let all = imp.all_commits.borrow().clone();
        let selected_year = imp.selected_year.get();

        let filtered: Vec<CommitInfo> = if query.is_empty() {
            if selected_year != 0 {
                all.into_iter()
                    .filter(|c| {
                        let dt = chrono::DateTime::from_timestamp(c.timestamp, 0);
                        dt.map(|d| d.year() == selected_year).unwrap_or(false)
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
                        let dt = chrono::DateTime::from_timestamp(c.timestamp, 0);
                        dt.map(|d| d.year() == selected_year).unwrap_or(false)
                    };
                    year_ok && (
                        c.summary.to_lowercase().contains(&q)
                            || c.hash.to_lowercase().contains(&q)
                            || c.author.to_lowercase().contains(&q)
                    )
                })
                .collect()
        };

        commit_controller::populate_commit_list(list, filtered, cancel);
    }

    // ── Error display ──────────────────────────────────────────────────────────────────────

    fn show_error(&self, message: &str) {
        eprintln!("[TemporalExplorer] {message}");
        let dialog = adw::AlertDialog::new(Some(&gettext("Error")), Some(message));
        dialog.add_response("ok", &gettext("OK"));
        dialog.present(Some(self));
    }

    // ── Timestamp formatter ────────────────────────────────────────────────────────────────

    fn format_timestamp(ts: i64) -> String {
        use chrono::TimeZone;
        if let Some(dt) = chrono::Local.timestamp_opt(ts, 0).single() {
            dt.format("%Y-%m-%d %H:%M").unwrap_or_default().to_string()
        } else { ts.to_string() }
    }
}
