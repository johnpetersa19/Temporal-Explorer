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

use adw::prelude::AdwApplicationWindowExt;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use gtk::prelude::*;
use glib::object::ObjectExt;
use gettextrs::gettext;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::{Arc, atomic::{AtomicBool, Ordering}};

use crate::git_engine::{CommitInfo, DirCache, HistoryReader, SnapshotResolver, TreeNode};
use crate::commit_controller;
use crate::file_preview;

// ── ViewMode ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum ViewMode { #[default] List, Grid }

// ── DebugRepository ───────────────────────────────────────────────────────────

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

// ── Private implementation ────────────────────────────────────────────────────

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
        // "pathbar" page
        #[template_child] pub address_bar:          TemplateChild<gtk::Box>,
        // "location" page
        #[template_child] pub location_entry:       TemplateChild<gtk::Entry>,
        #[template_child] pub location_cancel_btn:  TemplateChild<gtk::Button>,

        // Left panel
        #[template_child] pub commit_search_entry:  TemplateChild<gtk::SearchEntry>,
        #[template_child] pub commit_list:          TemplateChild<gtk::ListBox>,

        // Right panel
        #[template_child] pub empty_state:          TemplateChild<adw::StatusPage>,
        #[template_child] pub split_view:           TemplateChild<adw::OverlaySplitView>,
        #[template_child] pub content_toolbar_view: TemplateChild<adw::ToolbarView>,

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

        // PERF: LRU cache for (hash, dir) → Arc<Vec<TreeNode>>.
        pub dir_cache:        RefCell<DirCache>,

        // PERF: search debounce timer handle.
        pub search_debounce:  RefCell<Option<glib::SourceId>>,

        // PERF: cancellation token for the in-flight search background task.
        pub search_cancel:    RefCell<Option<Arc<AtomicBool>>>,
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

// ── Public wrapper ────────────────────────────────────────────────────────────

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
        glib::Object::builder().property("application", application).build()
    }

    // ── Styles ────────────────────────────────────────────────────────────────

    fn setup_styles(&self) {
        let provider = gtk::CssProvider::new();
        provider.load_from_string("
            /* ── Path bar pill container ─────────────────────────────── */
            .nautilus-pathbar {
                background-color: color-mix(in srgb, currentColor 8%, transparent);
                border-radius: 9999px;
                padding: 2px 4px;
                min-height: 32px;
            }

            /* ── Individual path segment buttons ──────────────────────── */
            .nautilus-path-button {
                min-width: 8px;
                border-radius: 9999px;
                padding: 0 8px;
                min-height: 28px;
            }
            .nautilus-path-button label {
                font-weight: 600;
            }
            .nautilus-path-button:not(.current-dir) label,
            .nautilus-path-button:not(.current-dir) image {
                opacity: 0.55;
            }
            .nautilus-path-button:not(.current-dir):hover label,
            .nautilus-path-button:not(.current-dir):hover image {
                opacity: 0.85;
            }
            .nautilus-path-button.current-dir {
                background: none;
                box-shadow: none;
            }

            /* ── Chevron separators ───────────────────────────────── */
            .nautilus-path-separator {
                opacity: 0.35;
                margin: 0 1px;
                -gtk-icon-size: 12px;
            }

            /* ── Location entry ───────────────────────────────────────── */
            .location-bar {
                min-width: 320px;
            }
            .location-bar entry {
                border-radius: 9999px 0 0 9999px;
            }
            .location-bar button {
                border-radius: 0 9999px 9999px 0;
            }

            /* ── Grid view cells ─────────────────────────────────────── */
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

            /* ── List view rows ───────────────────────────────────────────── */
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

            /* ── Commit info bar ─────────────────────────────────────────── */
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

    // ── Signal wiring ─────────────────────────────────────────────────────────

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
        imp.commit_list.connect_row_selected(glib::clone!(
            #[weak(rename_to = w)] self, move |_, row| w.on_commit_selected(row)
        ));

        // PERF: debounce search — fire only after 200 ms of inactivity.
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
                if let Some(w) = weak_self.upgrade() {
                    w.show_pathbar();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        imp.location_entry.add_controller(key_ctrl);

        imp.location_cancel_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = w)] self, move |_| w.show_pathbar()
        ));
    }

    // ── toolbar_switcher helpers ───────────────────────────────────────────

    fn show_pathbar(&self) {
        self.imp().toolbar_switcher.set_visible_child_name("pathbar");
    }

    fn show_location_entry(&self) {
        let imp = self.imp();
        let dir = imp.current_dir.borrow().clone();
        let path_text = if dir.as_os_str().is_empty() {
            String::new()
        } else {
            dir.to_string_lossy().to_string()
        };
        imp.location_entry.set_text(&path_text);
        imp.toolbar_switcher.set_visible_child_name("location");
        imp.location_entry.grab_focus();
        imp.location_entry.select_region(0, -1);
    }

    fn navigate_to_location_text(&self, text: &str) {
        let trimmed = text.trim().trim_matches('/');
        let target = if trimmed.is_empty() {
            PathBuf::new()
        } else {
            PathBuf::from(trimmed)
        };
        self.enter_dir(target);
    }

    // ── View mode toggle ───────────────────────────────────────────────────────

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

    // ── Open repository ───────────────────────────────────────────────────────

    fn open_repository_dialog(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Open Git Repository"))
            .modal(true)
            .build();
        dialog.select_folder(
            Some(self), gio::Cancellable::NONE,
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
        match HistoryReader::open(&path) {
            Err(e) => self.show_error_toast(&format!("{}: {e}", gettext("Failed to open repository"))),
            Ok(reader) => {
                let repo_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(&gettext("Repository"))
                    .to_string();
                imp.window_title.set_title(&repo_name);
                imp.window_title.set_subtitle(&gettext("Loading…"));
                *imp.repo_name.borrow_mut()    = repo_name;
                *imp.repo_path.borrow_mut()    = Some(path.clone());
                *imp.repository.borrow_mut()   = Some(DebugRepository(reader.repo));
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

                // PERF: load commits in a background thread via std::sync::mpsc
                // (glib::MainContext::channel was removed in glib 0.20).
                // Pages are forwarded to the GTK main loop using
                // glib::idle_add_local_once so widget mutations always happen
                // on the main thread.
                let (tx, rx) = std::sync::mpsc::channel::<Vec<CommitInfo>>();

                std::thread::spawn(move || {
                    if let Ok(bg_reader) = HistoryReader::open(&path) {
                        let _ = bg_reader.list_commits_paginated(500, |page| {
                            let _ = tx.send(page);
                        });
                    }
                });

                // Poll the channel from the main loop until the sender is dropped.
                let weak_self = self.downgrade();
                glib::idle_add_local(move || {
                    match rx.try_recv() {
                        Ok(page) => {
                            if let Some(w) = weak_self.upgrade() {
                                let imp = w.imp();
                                commit_controller::append_commit_batch(&imp.commit_list, &page);
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
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            // No page ready yet — yield and come back next idle.
                            glib::ControlFlow::Continue
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            // Sender dropped: background thread finished.
                            glib::ControlFlow::Break
                        }
                    }
                });
            }
        }
    }

    // ── Commit list ───────────────────────────────────────────────────────────

    fn populate_commit_list(&self, commits: &[CommitInfo]) {
        commit_controller::populate_commit_list(&self.imp().commit_list, commits);
    }

    // ── Search — off-thread filtering with per-query cancellation ─────────────

    fn on_search_changed(&self, query: &str) {
        let imp = self.imp();
        { let last = imp.last_query.borrow(); if *last == query { return; } }
        *imp.last_query.borrow_mut() = query.to_owned();

        // Cancel any in-flight search.
        if let Some(old_cancel) = imp.search_cancel.borrow().as_ref() {
            old_cancel.store(true, Ordering::Relaxed);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        *imp.search_cancel.borrow_mut() = Some(Arc::clone(&cancel));

        let all: Vec<CommitInfo> = imp.all_commits.borrow().clone();
        let query_owned = query.to_owned();

        // Fast-path: empty query — no thread needed.
        if query_owned.is_empty() {
            self.populate_commit_list(&all);
            return;
        }

        let (tx, rx) = std::sync::mpsc::channel::<Vec<CommitInfo>>();

        std::thread::spawn(move || {
            let q = query_owned.to_lowercase();
            let mut results = Vec::new();
            for commit in &all {
                if cancel.load(Ordering::Relaxed) { return; }
                if commit.summary.to_lowercase().contains(&q)
                    || commit.hash.starts_with(&query_owned)
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
                    if let Some(w) = weak_self.upgrade() {
                        w.populate_commit_list(&results);
                    }
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
    }

    // ── Commit selected ───────────────────────────────────────────────────────

    fn on_commit_selected(&self, row: Option<&gtk::ListBoxRow>) {
        let imp = self.imp();
        let row = match row {
            Some(r) => r,
            None => { imp.commit_info_bar.set_revealed(false); self.show_empty_state(); return; }
        };
        let hash = row.widget_name().to_string();
        let commit = { let all = imp.all_commits.borrow(); all.iter().find(|c| c.hash == hash).cloned() };
        let commit = match commit { Some(c) => c, None => return };
        imp.commit_hash_label.set_label(&commit.hash[..12]);
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

    // ── Navigation ────────────────────────────────────────────────────────────

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

        // PERF: check LRU cache before hitting git2.
        // DirCache::get returns Arc<Vec<TreeNode>>; deref to get a plain Vec
        // so the type matches the else-branch below.
        let mut children: Vec<TreeNode> = if let Some(arc) = imp.dir_cache.borrow_mut().get(hash, dir.as_path()) {
            (*arc).clone()
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
                    imp.dir_cache.borrow_mut().insert(
                        hash.to_owned(),
                        dir.clone(),
                        n.clone(),
                    );
                    n
                }
                Err(e) => {
                    drop(repo_ref);
                    self.show_error_toast(&format!("{}: {e}", gettext("Cannot resolve snapshot")));
                    return;
                }
            }
        };

        // Sort: dirs first, then files, both alphabetically.
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

        self.update_address_bar(dir);
        self.show_pathbar();

        let view_mode = *imp.view_mode.borrow();
        let widget: gtk::Widget = match view_mode {
            ViewMode::List => self.build_list_view(&children, hash),
            ViewMode::Grid => self.build_grid_view(&children, hash),
        };
        self.replace_right_panel(widget);
    }

    // ── List view ─────────────────────────────────────────────────────────────

    fn build_list_view(&self, children: &[TreeNode], hash: &str) -> gtk::Widget {
        let scrolled = gtk::ScrolledWindow::builder()
            .vexpand(true).hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        list.add_css_class("boxed-list");

        if children.is_empty() {
            let placeholder = gtk::Label::builder()
                .label(gettext("Empty directory"))
                .margin_top(24).margin_bottom(24)
                .build();
            placeholder.add_css_class("dim-label");
            list.append(&gtk::ListBoxRow::builder().child(&placeholder).build());
        } else {
            for node in children { list.append(&Self::build_file_row(node)); }
        }

        let children_clone = children.to_vec();
        let hash_clone = hash.to_owned();
        list.connect_row_activated(glib::clone!(
            #[weak(rename_to = window)] self,
            move |_, row| {
                let idx = row.index() as usize;
                if let Some(node) = children_clone.get(idx) {
                    if node.is_dir() {
                        window.enter_dir(node.path().to_path_buf());
                    } else {
                        window.open_file_preview(node.path(), &hash_clone);
                    }
                }
            }
        ));

        scrolled.set_child(Some(&list));
        scrolled.upcast()
    }

    fn build_file_row(node: &TreeNode) -> gtk::ListBoxRow {
        let hbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal).spacing(10)
            .margin_top(5).margin_bottom(5).margin_start(12).margin_end(12)
            .build();
        hbox.add_css_class("nautilus-list-row");

        let icon_name = match node {
            TreeNode::Dir(p) => folder_icon_symbolic(p.file_name().and_then(|n| n.to_str()).unwrap_or("")),
            TreeNode::File(p) => mime_icon(p),
        };
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.set_pixel_size(16);
        hbox.append(&icon);

        let name = node.path().file_name().and_then(|n| n.to_str()).unwrap_or("");
        let label = gtk::Label::builder().label(name).xalign(0.0).hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End).build();
        hbox.append(&label);

        if node.is_dir() {
            let chevron = gtk::Image::from_icon_name("go-next-symbolic");
            chevron.add_css_class("dim-label");
            chevron.set_pixel_size(12);
            hbox.append(&chevron);
        } else if let Some(ext) = node.path().extension().and_then(|e| e.to_str()) {
            let type_label = gtk::Label::builder().label(&ext.to_uppercase()).build();
            type_label.add_css_class("caption");
            type_label.add_css_class("dim-label");
            hbox.append(&type_label);
        }

        gtk::ListBoxRow::builder().child(&hbox).build()
    }

    // ── Grid view ─────────────────────────────────────────────────────────────

    fn build_grid_view(&self, children: &[TreeNode], hash: &str) -> gtk::Widget {
        let scrolled = gtk::ScrolledWindow::builder()
            .vexpand(true).hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .homogeneous(true)
            .column_spacing(6).row_spacing(6)
            .margin_top(16).margin_bottom(16).margin_start(16).margin_end(16)
            .max_children_per_line(64)
            .min_children_per_line(1)
            .build();

        if children.is_empty() {
            let placeholder = gtk::Label::builder()
                .label(gettext("Empty directory"))
                .margin_top(24).margin_bottom(24)
                .build();
            placeholder.add_css_class("dim-label");
            flow.insert(&placeholder, -1);
        } else {
            for node in children {
                let cell = Self::build_grid_cell(node);
                let child = gtk::FlowBoxChild::builder()
                    .child(&cell)
                    .valign(gtk::Align::Start)
                    .halign(gtk::Align::Center)
                    .build();
                flow.insert(&child, -1);
            }
        }

        let children_clone = children.to_vec();
        let hash_clone = hash.to_owned();
        flow.connect_child_activated(glib::clone!(
            #[weak(rename_to = window)] self,
            move |_, child| {
                let idx = child.index() as usize;
                if let Some(node) = children_clone.get(idx) {
                    if node.is_dir() {
                        window.enter_dir(node.path().to_path_buf());
                    } else {
                        window.open_file_preview(node.path(), &hash_clone);
                    }
                }
            }
        ));

        scrolled.set_child(Some(&flow));
        scrolled.upcast()
    }

    fn build_grid_cell(node: &TreeNode) -> gtk::Box {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_top(6).margin_bottom(6).margin_start(6).margin_end(6)
            .width_request(96)
            .build();
        vbox.add_css_class("nautilus-view-cell");

        let icon_name = match node {
            TreeNode::Dir(p) => folder_icon(p.file_name().and_then(|n| n.to_str()).unwrap_or("")),
            TreeNode::File(p) => mime_icon_full(p),
        };
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.set_pixel_size(64);
        icon.set_halign(gtk::Align::Center);
        vbox.append(&icon);

        let name = node.path().file_name().and_then(|n| n.to_str()).unwrap_or("");
        let label = gtk::Label::builder()
            .label(name).halign(gtk::Align::Center)
            .justify(gtk::Justification::Center)
            .wrap(true).wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(12).lines(3)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        label.add_css_class("caption");
        vbox.append(&label);
        vbox
    }

    // ── File preview ──────────────────────────────────────────────────────────

    fn open_file_preview(&self, path: &std::path::Path, hash: &str) {
        let imp = self.imp();
        let repo_ref = imp.repository.borrow();
        let Some(repo) = repo_ref.as_ref() else {
            self.show_error_toast(&gettext("No repository open."));
            return;
        };
        file_preview::show_file_preview(self, repo, hash, path);
    }

    // ── Address bar ────────────────────────────────────────────────────────

    fn update_address_bar(&self, dir: &PathBuf) {
        let imp = self.imp();
        let bar = &imp.address_bar;

        while let Some(child) = bar.first_child() { child.unparent(); }

        let repo_name = imp.repo_name.borrow().clone();

        struct Seg { label: String, icon: Option<&'static str>, target: PathBuf }
        let mut segs: Vec<Seg> = Vec::new();

        segs.push(Seg { label: repo_name, icon: Some("folder-symbolic"), target: PathBuf::new() });

        let mut acc = PathBuf::new();
        for comp in dir.components() {
            let s = comp.as_os_str().to_string_lossy().to_string();
            acc.push(&s);
            segs.push(Seg { label: s, icon: None, target: acc.clone() });
        }

        let total = segs.len();
        for (idx, seg) in segs.iter().enumerate() {
            let is_current = idx == total - 1;

            if idx > 0 {
                let sep = gtk::Image::from_icon_name("go-next-symbolic");
                sep.add_css_class("nautilus-path-separator");
                bar.append(&sep);
            }

            let btn = gtk::Button::new();
            btn.add_css_class("flat");
            btn.add_css_class("nautilus-path-button");
            if is_current { btn.add_css_class("current-dir"); }

            let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            if let Some(ic) = seg.icon {
                let img = gtk::Image::from_icon_name(ic);
                img.set_pixel_size(16);
                row.append(&img);
            }
            let lbl = gtk::Label::builder().label(&seg.label).single_line_mode(true).build();
            row.append(&lbl);
            btn.set_child(Some(&row));

            let target = seg.target.clone();
            if is_current {
                btn.connect_clicked(glib::clone!(
                    #[weak(rename_to = window)] self,
                    move |_| window.show_location_entry()
                ));
            } else {
                btn.connect_clicked(glib::clone!(
                    #[weak(rename_to = window)] self,
                    move |_| window.enter_dir(target.clone())
                ));
            }

            bar.append(&btn);
        }
    }

    // ── Panel helpers ─────────────────────────────────────────────────────────

    fn show_empty_state(&self) {
        let imp = self.imp();
        imp.toolbar_switcher.set_visible_child_name("pathbar");
        while let Some(child) = imp.address_bar.first_child() { child.unparent(); }
        imp.window_title.set_visible(true);
        imp.nav_back_button.set_sensitive(false);
        imp.nav_forward_button.set_sensitive(false);
        self.replace_right_panel(imp.empty_state.clone().upcast());
    }

    fn replace_right_panel(&self, widget: gtk::Widget) {
        let tv = &self.imp().content_toolbar_view;
        tv.set_content(gtk::Widget::NONE);
        tv.set_content(Some(&widget));
    }

    // ── Utilities ─────────────────────────────────────────────────────────────

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

// ── MIME icon helpers ─────────────────────────────────────────────────────────

fn mime_icon(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs")                                              => "text-x-rust-symbolic",
        Some("toml") | Some("yaml") | Some("yml")              => "text-x-script-symbolic",
        Some("json")                                            => "text-x-script-symbolic",
        Some("xml") | Some("blp") | Some("ui")                 => "text-xml-symbolic",
        Some("md") | Some("rst") | Some("txt")                 => "text-x-generic-symbolic",
        Some("png") | Some("jpg") | Some("jpeg")               => "image-x-generic-symbolic",
        Some("svg") | Some("webp") | Some("gif")               => "image-x-generic-symbolic",
        Some("mp3") | Some("ogg") | Some("flac") | Some("wav") => "audio-x-generic-symbolic",
        Some("mp4") | Some("mkv") | Some("webm") | Some("avi") => "video-x-generic-symbolic",
        Some("sh") | Some("bash") | Some("zsh") | Some("fish") => "text-x-script-symbolic",
        Some("c") | Some("h") | Some("cpp") | Some("hpp")      => "text-x-csrc-symbolic",
        Some("py")                                             => "text-x-python-symbolic",
        Some("js") | Some("ts") | Some("jsx") | Some("tsx")    => "text-x-javascript-symbolic",
        Some("html") | Some("css")                             => "text-html-symbolic",
        Some("pdf")                                            => "application-pdf-symbolic",
        Some("zip") | Some("tar") | Some("gz") | Some("xz")    => "application-zip-symbolic",
        Some("lock")                                           => "text-x-generic-symbolic",
        Some("in")                                             => "text-x-makefile-symbolic",
        _ => match path.file_name().and_then(|n| n.to_str()) {
            Some(".gitignore") | Some(".gitattributes") | Some(".gitmodules") => "text-x-generic-symbolic",
            Some("Makefile") | Some("makefile") | Some("GNUmakefile")         => "text-x-makefile-symbolic",
            Some("LICENSE") | Some("COPYING") | Some("NOTICE")               => "text-x-generic-symbolic",
            Some("Dockerfile") | Some("Containerfile")                       => "application-x-executable-symbolic",
            _                                                                => "text-x-generic-symbolic",
        },
    }
}

fn mime_icon_full(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs")                                              => "text-x-rust",
        Some("toml") | Some("yaml") | Some("yml")              => "text-x-script",
        Some("json")                                            => "text-x-script",
        Some("xml") | Some("blp") | Some("ui")                 => "text-xml",
        Some("md") | Some("rst") | Some("txt")                 => "text-x-generic",
        Some("png") | Some("jpg") | Some("jpeg")               => "image-x-generic",
        Some("svg") | Some("webp") | Some("gif")               => "image-x-generic",
        Some("mp3") | Some("ogg") | Some("flac") | Some("wav") => "audio-x-generic",
        Some("mp4") | Some("mkv") | Some("webm") | Some("avi") => "video-x-generic",
        Some("sh") | Some("bash") | Some("zsh") | Some("fish") => "text-x-script",
        Some("c") | Some("h") | Some("cpp") | Some("hpp")      => "text-x-csrc",
        Some("py")                                             => "text-x-python",
        Some("js") | Some("ts") | Some("jsx") | Some("tsx")    => "text-x-javascript",
        Some("html") | Some("css")                             => "text-html",
        Some("pdf")                                            => "application-pdf",
        Some("zip") | Some("tar") | Some("gz") | Some("xz")    => "application-zip",
        Some("lock")                                           => "text-x-generic",
        Some("in")                                             => "text-x-makefile",
        _ => match path.file_name().and_then(|n| n.to_str()) {
            Some(".gitignore") | Some(".gitattributes") | Some(".gitmodules") => "text-x-generic",
            Some("Makefile") | Some("makefile") | Some("GNUmakefile")         => "text-x-makefile",
            Some("LICENSE") | Some("COPYING") | Some("NOTICE")               => "text-x-generic",
            Some("Dockerfile") | Some("Containerfile")                       => "application-x-executable",
            _                                                                => "text-x-generic",
        },
    }
}

// ── Folder icon helpers ──────────────────────────────────────────────────────

fn folder_icon(name: &str) -> &'static str {
    match name.to_lowercase().as_str() {
        "src" | "source" | "lib" | "crates"                              => "folder-development",
        "code" | "devel" | "development" | "projects" | "projetos"       => "folder-development",
        "doc" | "docs" | "documents" | "documentos" | "documentation"    => "folder-documents",
        "data" | "db" | "database" | "datasets"                          => "folder-documents",
        "test" | "tests" | "spec" | "specs" | "testing"                  => "folder-remote",
        "images" | "img" | "pictures" | "imagens" | "assets" | "media"   => "folder-pictures",
        "icons" | "pixmaps"                                              => "folder-pictures",
        "videos" | "video"                                               => "folder-videos",
        "music" | "audio" | "músicas" | "musicas" | "sounds"             => "folder-music",
        "download" | "downloads"                                         => "folder-download",
        "build" | "target" | "dist" | "out" | "output"                   => "folder-remote",
        "config" | "cfg" | "settings" | "conf"                           => "folder-documents",
        "scripts" | "bin" | "tools"                                      => "folder-development",
        "po" | "i18n" | "l10n" | "locale"                               => "folder-documents",
        "themes" | "theme" | "skins"                                     => "folder-pictures",
        _                                                                => "folder",
    }
}

fn folder_icon_symbolic(name: &str) -> &'static str {
    match name.to_lowercase().as_str() {
        "src" | "source" | "lib" | "crates"                              => "folder-development-symbolic",
        "code" | "devel" | "development" | "projects" | "projetos"       => "folder-development-symbolic",
        "doc" | "docs" | "documents" | "documentos" | "documentation"    => "folder-documents-symbolic",
        "data" | "db" | "database" | "datasets"                          => "folder-documents-symbolic",
        "test" | "tests" | "spec" | "specs" | "testing"                  => "folder-remote-symbolic",
        "images" | "img" | "pictures" | "imagens" | "assets" | "media"   => "folder-pictures-symbolic",
        "icons" | "pixmaps"                                              => "folder-pictures-symbolic",
        "videos" | "video"                                               => "folder-videos-symbolic",
        "music" | "audio" | "músicas" | "musicas" | "sounds"             => "folder-music-symbolic",
        "download" | "downloads"                                         => "folder-download-symbolic",
        "build" | "target" | "dist" | "out" | "output"                   => "folder-remote-symbolic",
        "config" | "cfg" | "settings" | "conf"                           => "folder-documents-symbolic",
        "scripts" | "bin" | "tools"                                      => "folder-development-symbolic",
        "po" | "i18n" | "l10n" | "locale"                               => "folder-documents-symbolic",
        "themes" | "theme" | "skins"                                     => "folder-pictures-symbolic",
        _                                                                => "folder-symbolic",
    }
}
