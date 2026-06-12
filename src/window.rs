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
        glib::Object::builder().property("application", application).build()
    }

    // ── Styles ──────────────────────────────────────────────────────────────────────────────

    fn setup_styles(&self) {
        let provider = gtk::CssProvider::new();
        provider.load_from_string("
            .nautilus-pathbar {
                background-color: color-mix(in srgb, currentColor 8%, transparent);
                border-radius: 9999px;
                padding: 2px 4px;
                min-height: 32px;
            }
            .nautilus-path-button {
                min-width: 8px;
                border-radius: 9999px;
                padding: 0 8px;
                min-height: 28px;
            }
            .nautilus-path-button label { font-weight: 600; }
            .nautilus-path-button:not(.current-dir) label,
            .nautilus-path-button:not(.current-dir) image { opacity: 0.55; }
            .nautilus-path-button:not(.current-dir):hover label,
            .nautilus-path-button:not(.current-dir):hover image { opacity: 0.85; }
            .nautilus-path-button.current-dir { background: none; box-shadow: none; }
            .nautilus-path-separator { opacity: 0.35; margin: 0 1px; -gtk-icon-size: 12px; }
            .location-bar { min-width: 320px; }
            .location-bar entry { border-radius: 9999px 0 0 9999px; }
            .location-bar button { border-radius: 0 9999px 9999px 0; }
            .nautilus-view-cell {
                border-radius: 8px;
                padding: 8px 6px 6px 6px;
                transition: background-color 150ms ease;
            }
            .nautilus-view-cell:hover {
                background-color: color-mix(in srgb, currentColor 7%, transparent);
            }
            flowboxchild:selected .nautilus-view-cell {
                background-color: color-mix(in srgb, @accent_bg_color 18%, transparent);
                outline: 1.5px solid color-mix(in srgb, @accent_bg_color 60%, transparent);
                outline-offset: -1.5px;
            }
            flowboxchild:selected:focus .nautilus-view-cell {
                background-color: color-mix(in srgb, @accent_bg_color 28%, transparent);
                outline-color: @accent_bg_color;
            }
            .nautilus-list-row {
                border-radius: 6px;
                transition: background-color 120ms ease;
            }
            .nautilus-list-row:hover {
                background-color: color-mix(in srgb, currentColor 5%, transparent);
            }
            row:selected.nautilus-list-row,
            row:selected .nautilus-list-row {
                background-color: color-mix(in srgb, @accent_bg_color 18%, transparent);
            }
            .commit-hash {
                font-family: monospace;
                font-size: 0.85em;
                opacity: 0.75;
                letter-spacing: 0.04em;
            }
        ");
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display, &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
    }

    // ── Signal wiring ─────────────────────────────────────────────────────────────────────────────

    fn setup_callbacks(&self) {
        let imp = self.imp();

        imp.open_repo_button.connect_clicked(glib::clone!(
            #[weak(rename_to = w)] self, move |_| w.open_repository_dialog()
        ));
        imp.nav_back_button.connect_clicked(glib::clone!(
            #[weak(rename_to = w)] self, move |_| w.navigate_back()
        ));
        imp.nav_forward_button.connect_clicked(glib::clone!(
            #[weak(rename_to = w)] self, move |_| w.navigate_forward()
        ));
        imp.view_toggle_button.connect_clicked(glib::clone!(
            #[weak(rename_to = w)] self, move |_| w.toggle_view_mode()
        ));

        imp.timeline_back_button.connect_clicked(glib::clone!(
            #[weak(rename_to = w)] self, move |_| w.on_timeline_back()
        ));
        imp.year_list.connect_row_selected(glib::clone!(
            #[weak(rename_to = w)] self,
            move |_, row| if let Some(r) = row { w.on_year_selected(r); }
        ));
        imp.month_list.connect_row_selected(glib::clone!(
            #[weak(rename_to = w)] self,
            move |_, row| if let Some(r) = row { w.on_month_selected(r); }
        ));
        imp.commit_list.connect_row_selected(glib::clone!(
            #[weak(rename_to = w)] self, move |_, row| w.on_commit_selected(row)
        ));

        imp.commit_search_entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = w)] self,
            move |e| {
                let query = e.text().to_string();
                let imp = w.imp();
                if let Some(id) = imp.search_debounce.borrow_mut().take() {
                    id.remove();
                }
                let new_id = glib::timeout_add_local(
                    std::time::Duration::from_millis(200),
                    glib::clone!(
                        #[weak] w,
                        #[upgrade_or] glib::ControlFlow::Break,
                        move || {
                            w.on_search_changed(&query);
                            w.imp().search_debounce.borrow_mut().take();
                            glib::ControlFlow::Break
                        }
                    ),
                );
                *imp.search_debounce.borrow_mut() = Some(new_id);
            }
        ));

        imp.location_entry.connect_activate(glib::clone!(
            #[weak(rename_to = w)] self, move |entry| {
                let text = entry.text().to_string();
                w.navigate_to_location_text(&text);
                w.show_pathbar();
            }
        ));

        let key_ctrl = gtk::EventControllerKey::new();
        let weak_self = self.downgrade();
        key_ctrl.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(w) = weak_self.upgrade() { w.show_pathbar(); }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        imp.location_entry.add_controller(key_ctrl);

        imp.location_cancel_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = w)] self, move |_| w.show_pathbar()
        ));
    }

    // ── toolbar_switcher helpers ──────────────────────────────────────────────────────────────────

    fn show_pathbar(&self) {
        self.imp().toolbar_switcher.set_visible_child_name("pathbar");
    }

    #[allow(dead_code)]
    fn show_location_entry(&self) {
        let imp = self.imp();
        let dir = imp.current_dir.borrow().clone();
        address_bar::switch_to_location_entry(
            &imp.toolbar_switcher,
            &imp.location_entry,
            &dir,
        );
    }

    fn navigate_to_location_text(&self, text: &str) {
        let trimmed = text.trim().trim_matches('/');
        let target = if trimmed.is_empty() { PathBuf::new() } else { PathBuf::from(trimmed) };

        if !target.as_os_str().is_empty() {
            let imp = self.imp();
            let hash = match imp.current_hash.borrow().clone() {
                Some(h) => h,
                None => return,
            };
            let cached = imp.dir_cache.borrow_mut().get(&hash, target.as_path()).is_some();
            if !cached {
                let repo_ref = imp.repository.borrow();
                let repo = match repo_ref.as_ref() {
                    Some(r) => r,
                    None => { self.show_error_toast(&gettext("No repository open.")); return; }
                };
                let resolver = SnapshotResolver::new(repo);
                if let Err(e) = resolver.resolve_dir(&hash, target.as_path()) {
                    drop(repo_ref);
                    self.show_error_toast(&format!("{}: {e}", gettext("Cannot resolve snapshot")));
                    return;
                }
                drop(repo_ref);
            }
        }
        self.enter_dir(target);
    }

    // ── Timeline drill-down ───────────────────────────────────────────────────────────────────────

    fn show_year_list(&self) {
        let imp = self.imp();
        commit_controller::populate_month_list(&imp.month_list, &[], 0);
        *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
        imp.timeline_stack.set_visible_child_name("years");
        imp.timeline_back_button.set_visible(false);
        imp.timeline_header_title.set_title(&gettext("Timeline"));
        imp.timeline_header_title.set_subtitle("");
        imp.commit_search_entry.set_visible(false);
    }

    fn on_year_selected(&self, row: &gtk::ListBoxRow) {
        let year: i32 = match row.widget_name().parse() {
            Ok(y) => y,
            Err(_) => return,
        };
        let imp = self.imp();
        imp.selected_year.set(year);
        let all = imp.all_commits.borrow();
        commit_controller::populate_month_list(&imp.month_list, &all, year);
        drop(all);
        *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
        imp.timeline_stack.set_visible_child_name("months");
        imp.timeline_back_button.set_visible(true);
        imp.timeline_header_title.set_title(&year.to_string());
        imp.timeline_header_title.set_subtitle("");
        imp.commit_search_entry.set_visible(false);
        imp.year_list.unselect_all();
    }

    fn on_month_selected(&self, row: &gtk::ListBoxRow) {
        let month: u32 = match row.widget_name().parse() {
            Ok(m) if m >= 1 && m <= 12 => m,
            _ => return,
        };
        let imp = self.imp();
        let year = imp.selected_year.get();
        let commits = {
            let all = imp.all_commits.borrow();
            timeline_filter::commits_for_month(&all, year, month)
        };
        commit_controller::populate_commit_list(&imp.commit_list, &commits);
        *imp.timeline_level.borrow_mut() = TimelineLevel::Commits;
        imp.timeline_stack.set_visible_child_name("commits");
        imp.timeline_back_button.set_visible(true);
        imp.timeline_header_title.set_title(timeline_filter::month_name(month));
        imp.timeline_header_title.set_subtitle(&year.to_string());
        imp.commit_search_entry.set_visible(true);
        imp.commit_info_bar.set_revealed(false);
        self.show_empty_state();
        imp.month_list.unselect_all();
    }

    fn on_timeline_back(&self) {
        let level = *self.imp().timeline_level.borrow();
        match level {
            TimelineLevel::Months  => self.show_year_list(),
            TimelineLevel::Commits => {
                let imp = self.imp();
                commit_controller::populate_commit_list(&imp.commit_list, &[]);
                let year = imp.selected_year.get();
                *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
                imp.timeline_stack.set_visible_child_name("months");
                imp.timeline_back_button.set_visible(true);
                imp.timeline_header_title.set_title(&year.to_string());
                imp.timeline_header_title.set_subtitle("");
                imp.commit_search_entry.set_visible(false);
                imp.commit_info_bar.set_revealed(false);
                self.show_empty_state();
            }
            TimelineLevel::Years => {}
        }
    }

    fn populate_year_list(&self) {
        let imp = self.imp();
        let all = imp.all_commits.borrow();
        commit_controller::populate_year_list(&imp.year_list, &all);
        drop(all);
        self.show_year_list();
    }

    // ── View mode toggle ────────────────────────────────────────────────────────────────────────────

    fn toggle_view_mode(&self) {
        let imp = self.imp();
        let new_mode = match *imp.view_mode.borrow() {
            ViewMode::List => ViewMode::Grid,
            ViewMode::Grid => ViewMode::List,
        };
        *imp.view_mode.borrow_mut() = new_mode;
        let icon = match new_mode {
            ViewMode::List => "view-grid-symbolic",
            ViewMode::Grid => "view-list-symbolic",
        };
        imp.view_toggle_button.set_icon_name(icon);
        let maybe_hash = imp.current_hash.borrow().clone();
        if let Some(hash) = maybe_hash {
            let dir = imp.current_dir.borrow().clone();
            self.browse_dir_inner(&hash, &dir);
        }
    }

    // ── Open repository ────────────────────────────────────────────────────────────────────────────

    fn open_repository_dialog(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Open Git Repository"))
            .modal(true)
            .build();
        dialog.select_folder(
            Some(self), None::<&gio::Cancellable>,
            glib::clone!(
                #[weak(rename_to = w)] self,
                move |result| {
                    if let Ok(folder) = result {
                        if let Some(path) = folder.path() { w.load_repository(path); }
                    }
                }
            ),
        );
    }

    fn load_repository(&self, path: PathBuf) {
        let imp = self.imp();

        if let Some(old_cancel) = imp.load_cancel.borrow().as_ref() {
            old_cancel.store(true, Ordering::Relaxed);
        }

        let reader = match HistoryReader::open(&path) {
            Ok(r) => r,
            Err(e) => {
                self.show_error_toast(&format!("{}: {e}", gettext("Failed to open repository")));
                return;
            }
        };
        let repo = reader.into_git2();

        let repo_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&gettext("Repository"))
            .to_string();
        imp.window_title.set_title(&repo_name);
        imp.window_title.set_subtitle(&gettext("Loading…"));
        *imp.repo_name.borrow_mut()    = repo_name;
        *imp.repo_path.borrow_mut()    = Some(path.clone());
        *imp.repository.borrow_mut()   = Some(DebugRepository(repo));
        *imp.all_commits.borrow_mut()  = Vec::new();
        *imp.last_query.borrow_mut()   = String::new();
        *imp.current_hash.borrow_mut() = None;
        *imp.current_dir.borrow_mut()  = PathBuf::new();
        imp.history_back.borrow_mut().clear();
        imp.history_forward.borrow_mut().clear();
        imp.nav_back_button.set_sensitive(false);
        imp.nav_forward_button.set_sensitive(false);
        imp.dir_cache.borrow_mut().clear();
        commit_controller::populate_commit_list(&imp.commit_list, &[]);
        imp.commit_info_bar.set_revealed(false);
        self.show_empty_state();
        self.show_year_list();

        imp.loading_commits.set(true);
        imp.commit_search_entry.set_sensitive(false);
        imp.commit_search_entry.set_tooltip_text(Some(
            &gettext("Search will be available once all commits are loaded"),
        ));

        let (tx, rx) = std::sync::mpsc::channel::<Vec<CommitInfo>>();
        std::thread::spawn(move || {
            if let Ok(bg_reader) = HistoryReader::open(&path) {
                let _ = bg_reader.list_commits_paginated(500, |page| {
                    let _ = tx.send(page);
                });
            }
        });

        let cancel = Arc::new(AtomicBool::new(false));
        *imp.load_cancel.borrow_mut() = Some(Arc::clone(&cancel));

        let weak_self = self.downgrade();
        glib::idle_add_local(move || {
            if cancel.load(Ordering::Relaxed) { return glib::ControlFlow::Break; }
            match rx.try_recv() {
                Ok(page) => {
                    if let Some(w) = weak_self.upgrade() {
                        let imp = w.imp();
                        let mut all = imp.all_commits.borrow_mut();
                        all.extend(page);
                        let count = all.len();
                        drop(all);
                        imp.window_title.set_subtitle(
                            &format!("{} {}", count, gettext("commits"))
                        );
                    }
                    glib::ControlFlow::Continue
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    if let Some(w) = weak_self.upgrade() {
                        let imp = w.imp();
                        imp.loading_commits.set(false);
                        imp.commit_search_entry.set_sensitive(true);
                        imp.commit_search_entry.set_tooltip_text(None);
                        let count = imp.all_commits.borrow().len();
                        imp.window_title.set_subtitle(
                            &format!("{} {}", count, gettext("commits"))
                        );
                        w.populate_year_list();
                    }
                    glib::ControlFlow::Break
                }
            }
        });
    }

    // ── Commit list ──────────────────────────────────────────────────────────────────────────────

    fn populate_commit_list(&self, commits: &[CommitInfo]) {
        commit_controller::populate_commit_list(&self.imp().commit_list, commits);
    }

    // ── Search ─────────────────────────────────────────────────────────────────────────────────

    fn on_search_changed(&self, query: &str) {
        let imp = self.imp();
        if imp.loading_commits.get() { return; }
        { let last = imp.last_query.borrow(); if *last == query { return; } }
        *imp.last_query.borrow_mut() = query.to_owned();

        if let Some(old_cancel) = imp.search_cancel.borrow().as_ref() {
            old_cancel.store(true, Ordering::Relaxed);
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *imp.search_cancel.borrow_mut() = Some(Arc::clone(&cancel));

        let all: Vec<CommitInfo> = imp.all_commits.borrow().clone();
        let year = imp.selected_year.get();
        let query_owned = query.to_owned();

        if query_owned.is_empty() {
            self.populate_commit_list(&all);
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel::<Vec<CommitInfo>>();
        std::thread::spawn(move || {
            let q = query_owned.to_lowercase();
            let mut results = Vec::new();
            let iter: Box<dyn Iterator<Item = &CommitInfo> + Send> = if year == 0 {
                Box::new(all.iter())
            } else {
                Box::new(all.iter().filter(move |c| {
                    matches!(
                        glib::DateTime::from_unix_local(c.timestamp)
                            .ok()
                            .map(|dt| dt.year()),
                        Some(y) if y == year
                    )
                }))
            };
            for commit in iter {
                if cancel.load(Ordering::Relaxed) { return; }
                if commit.summary.to_lowercase().contains(&q)
                    || commit.hash.starts_with(&q)
                    || commit.author.to_lowercase().contains(&q)
                {
                    results.push(commit.clone());
                }
            }
            let _ = tx.send(results);
        });

        let weak_self = self.downgrade();
        glib::idle_add_local(move || {
            match rx.try_recv() {
                Ok(results) => {
                    if let Some(w) = weak_self.upgrade() { w.populate_commit_list(&results); }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    // ── Commit selected ───────────────────────────────────────────────────────────────────────────

    fn on_commit_selected(&self, row: Option<&gtk::ListBoxRow>) {
        let imp = self.imp();
        let row = match row {
            Some(r) => r,
            None => { imp.commit_info_bar.set_revealed(false); self.show_empty_state(); return; }
        };
        let hash = row.widget_name().to_string();
        let commit = { let all = imp.all_commits.borrow(); all.iter().find(|c| c.hash == hash).cloned() };
        let commit = match commit { Some(c) => c, None => return };
        imp.commit_hash_label.set_label(&commit.hash[..commit.hash.len().min(12)]);
        imp.commit_hash_label.add_css_class("commit-hash");
        imp.commit_message_label.set_label(&commit.summary);
        imp.commit_date_label.set_label(&Self::format_timestamp(commit.timestamp));
        imp.commit_info_bar.set_revealed(true);
        *imp.current_hash.borrow_mut() = Some(hash.clone());
        *imp.current_dir.borrow_mut()  = PathBuf::new();
        imp.history_back.borrow_mut().clear();
        imp.history_forward.borrow_mut().clear();
        self.browse_dir(&hash, &PathBuf::new());
    }

    // ── Navigation ──────────────────────────────────────────────────────────────────────────────

    fn enter_dir(&self, dir: PathBuf) {
        let imp = self.imp();
        let hash = match imp.current_hash.borrow().clone() { Some(h) => h, None => return };
        let prev = imp.current_dir.borrow().clone();
        imp.history_back.borrow_mut().push(prev);
        imp.history_forward.borrow_mut().clear();
        *imp.current_dir.borrow_mut() = dir.clone();
        self.browse_dir_inner(&hash, &dir);
        self.update_nav_buttons();
    }

    fn navigate_back(&self) {
        let imp = self.imp();
        let prev = imp.history_back.borrow_mut().pop();
        if let Some(dir) = prev {
            let cur = imp.current_dir.borrow().clone();
            imp.history_forward.borrow_mut().push(cur);
            *imp.current_dir.borrow_mut() = dir.clone();
            let maybe_hash = imp.current_hash.borrow().clone();
            if let Some(hash) = maybe_hash { self.browse_dir_inner(&hash, &dir); }
            self.update_nav_buttons();
        }
    }

    fn navigate_forward(&self) {
        let imp = self.imp();
        let next = imp.history_forward.borrow_mut().pop();
        if let Some(dir) = next {
            let cur = imp.current_dir.borrow().clone();
            imp.history_back.borrow_mut().push(cur);
            *imp.current_dir.borrow_mut() = dir.clone();
            let maybe_hash = imp.current_hash.borrow().clone();
            if let Some(hash) = maybe_hash { self.browse_dir_inner(&hash, &dir); }
            self.update_nav_buttons();
        }
    }

    fn update_nav_buttons(&self) {
        let imp = self.imp();
        imp.nav_back_button.set_sensitive(!imp.history_back.borrow().is_empty());
        imp.nav_forward_button.set_sensitive(!imp.history_forward.borrow().is_empty());
    }

    fn browse_dir(&self, hash: &str, dir: &PathBuf) {
        self.browse_dir_inner(hash, dir);
        self.update_nav_buttons();
    }

    fn browse_dir_inner(&self, hash: &str, dir: &PathBuf) {
        let imp = self.imp();

        let cached: Option<Vec<TreeNode>> = imp
            .dir_cache
            .borrow_mut()
            .get(hash, dir.as_path())
            .map(|arc| (*arc).clone());

        let mut children: Vec<TreeNode> = if let Some(nodes) = cached {
            nodes
        } else {
            let repo_ref = imp.repository.borrow();
            let repo = match repo_ref.as_ref() {
                Some(r) => r,
                None => { self.show_error_toast(&gettext("No repository open.")); return; }
            };
            let resolver = SnapshotResolver::new(repo);
            match resolver.resolve_dir(hash, dir.as_path()) {
                Ok(n) => {
                    drop(repo_ref);
                    imp.dir_cache.borrow_mut().insert(hash.to_owned(), dir.clone(), n.clone());
                    n
                }
                Err(e) => {
                    drop(repo_ref);
                    self.show_error_toast(&format!("{}: {e}", gettext("Cannot resolve snapshot")));
                    return;
                }
            }
        };

        let name = |n: &TreeNode| n.path().file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        children.sort_by(|a, b| {
            match (a.is_dir(), b.is_dir()) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => name(a).cmp(&name(b)),
            }
        });

        let subtitle = commit_controller::item_count_subtitle(children.len());
        imp.window_title.set_subtitle(&subtitle);

        {
            let repo_name = imp.repo_name.borrow().clone();
            let bar = imp.address_bar.clone();
            let toolbar_switcher = imp.toolbar_switcher.clone();
            address_bar::rebuild_address_bar(
                &bar,
                &repo_name,
                dir,
                glib::clone!(
                    #[weak(rename_to = w)] self,
                    move |target| w.enter_dir(target)
                ),
                glib::clone!(
                    #[weak] toolbar_switcher,
                    #[weak] bar,
                    #[weak(rename_to = w)] self,
                    move || {
                        let dir = w.imp().current_dir.borrow().clone();
                        address_bar::switch_to_location_entry(
                            &toolbar_switcher,
                            &w.imp().location_entry,
                            &dir,
                        );
                        let _ = bar;
                    }
                ),
            );
        }
        self.show_pathbar();

        let view_mode = *imp.view_mode.borrow();
        let widget: gtk::Widget = match view_mode {
            ViewMode::List => list_view::build_list_view(
                &children, hash,
                Box::new(glib::clone!(#[weak(rename_to = w)] self, move |dir| w.enter_dir(dir))),
                Box::new(glib::clone!(#[weak(rename_to = w)] self, move |path, hash| w.open_file_preview(path, hash))),
            ),
            ViewMode::Grid => grid_view::build_grid_view(
                &children, hash,
                Box::new(glib::clone!(#[weak(rename_to = w)] self, move |dir| w.enter_dir(dir))),
                Box::new(glib::clone!(#[weak(rename_to = w)] self, move |path, hash| w.open_file_preview(path, hash))),
            ),
        };
        self.replace_right_panel(widget);
    }

    // ── File preview ────────────────────────────────────────────────────────────────────────────

    fn open_file_preview(&self, path: &std::path::Path, hash: &str) {
        let imp = self.imp();
        let repo_ref = imp.repository.borrow();
        let Some(repo) = repo_ref.as_ref() else {
            self.show_error_toast(&gettext("No repository open."));
            return;
        };
        file_preview::show_file_preview(self, repo, hash, path);
    }

    // ── Panel helpers ─────────────────────────────────────────────────────────────────────────────

    fn show_empty_state(&self) {
        let imp = self.imp();
        imp.toolbar_switcher.set_visible_child_name("pathbar");
        clear_box(&imp.address_bar);
        imp.window_title.set_visible(true);
        imp.nav_back_button.set_sensitive(false);
        imp.nav_forward_button.set_sensitive(false);
        // Clear the content box and flip to the "empty" page.
        // The empty_state StatusPage is never moved — it always lives inside
        // the "empty" StackPage, so its parent pointer stays valid.
        clear_box(&imp.right_panel_content);
        imp.right_panel_stack.set_visible_child_name("empty");
    }

    fn replace_right_panel(&self, widget: gtk::Widget) {
        // Instead of calling set_content() on content_toolbar_view (which
        // would require the widget to have no parent), we append into the
        // permanent right_panel_content Box and flip the stack to "content".
        // This never re-parents any template child and never triggers the
        // adw_toolbar_view_set_content parent assertion.
        let imp = self.imp();
        clear_box(&imp.right_panel_content);
        imp.right_panel_content.append(&widget);
        imp.right_panel_stack.set_visible_child_name("content");
    }

    // ── Utilities ─────────────────────────────────────────────────────────────────────────────

    fn show_error_toast(&self, message: &str) {
        let toast = adw::Toast::builder().title(message).timeout(4).build();
        if let Some(overlay) = self.content().and_then(|w| w.downcast::<adw::ToastOverlay>().ok()) {
            overlay.add_toast(toast);
        } else {
            eprintln!("[temporal-explorer] {message}");
        }
    }

    fn format_timestamp(ts: i64) -> String {
        if let Ok(dt) = glib::DateTime::from_unix_local(ts) {
            dt.format("%Y-%m-%d %H:%M").unwrap_or_default().to_string()
        } else { ts.to_string() }
    }
}
