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
use std::cell::RefCell;
use std::path::PathBuf;

use crate::git_engine::{CommitInfo, HistoryReader, SnapshotResolver, TreeNode};

// ── ViewMode ──────────────────────────────────────────────────────────────────

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
        #[template_child] pub open_repo_button:    TemplateChild<gtk::Button>,
        #[template_child] pub nav_back_button:     TemplateChild<gtk::Button>,
        #[template_child] pub nav_forward_button:  TemplateChild<gtk::Button>,
        #[template_child] pub view_toggle_button:  TemplateChild<gtk::Button>,
        #[template_child] pub show_sidebar_button: TemplateChild<gtk::ToggleButton>,
        #[template_child] pub window_title:        TemplateChild<adw::WindowTitle>,
        #[template_child] pub address_bar:         TemplateChild<gtk::Box>,

        // Left panel
        #[template_child] pub commit_search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child] pub commit_list:         TemplateChild<gtk::ListBox>,

        // Right panel
        #[template_child] pub empty_state:         TemplateChild<adw::StatusPage>,
        #[template_child] pub split_view:          TemplateChild<adw::OverlaySplitView>,
        #[template_child] pub content_toolbar_view:TemplateChild<adw::ToolbarView>,

        // Bottom bar
        #[template_child] pub commit_info_bar:     TemplateChild<gtk::ActionBar>,
        #[template_child] pub commit_hash_label:   TemplateChild<gtk::Label>,
        #[template_child] pub commit_message_label:TemplateChild<gtk::Label>,
        #[template_child] pub commit_date_label:   TemplateChild<gtk::Label>,

        // Runtime state
        pub all_commits:     RefCell<Vec<CommitInfo>>,
        pub repo_path:       RefCell<Option<PathBuf>>,
        pub repository:      RefCell<Option<DebugRepository>>,
        pub last_query:      RefCell<String>,
        pub current_hash:    RefCell<Option<String>>,
        pub current_dir:     RefCell<PathBuf>,
        pub history_back:    RefCell<Vec<PathBuf>>,
        pub history_forward: RefCell<Vec<PathBuf>>,
        pub view_mode:       RefCell<ViewMode>,
        pub repo_name:       RefCell<String>,
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
            .nautilus-pathbar {
                background-color: color-mix(in srgb, currentColor 10%, transparent);
                border-radius: 9px;
                padding: 2px;
            }
            .nautilus-path-button {
                margin: 3px;
                min-width: 8px;
                border-radius: 7px;
                padding-top: 0px;
                padding-bottom: 0px;
            }
            .nautilus-path-button label {
                font-weight: bold;
            }
            .nautilus-path-button:not(:hover),
            .nautilus-path-button.current-dir {
                background: none;
                box-shadow: none;
            }
            .nautilus-path-button:not(.current-dir):not(:backdrop):hover label,
            .nautilus-path-button:not(.current-dir):not(:backdrop):hover image {
                opacity: 1;
            }
            .nautilus-view-cell {
                background-color: color-mix(in srgb, currentColor 4%, transparent);
                border: 1px solid color-mix(in srgb, currentColor 8%, transparent);
                border-radius: 12px;
                padding: 10px;
                transition: background-color 0.15s ease, border-color 0.15s ease;
            }
            .nautilus-view-cell:hover {
                background-color: color-mix(in srgb, currentColor 8%, transparent);
                border-color: color-mix(in srgb, currentColor 12%, transparent);
            }
            flowboxchild:selected .nautilus-view-cell {
                background-color: color-mix(in srgb, var(--accent-bg-color, #3584e4) 20%, transparent);
                border-color: var(--accent-bg-color, #3584e4);
            }
        ");
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
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
        imp.commit_search_entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = w)] self, move |e| w.on_search_changed(e.text().as_str())
        ));
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
            .title("Open Git Repository")
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
            Err(e) => self.show_error_toast(&format!("Failed to open repository: {e}")),
            Ok(reader) => {
                let commits = match reader.list_commits() {
                    Ok(c) => c,
                    Err(e) => { self.show_error_toast(&format!("Failed to read history: {e}")); return; }
                };
                let repo_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("Repository").to_string();
                imp.window_title.set_title(&repo_name);
                imp.window_title.set_subtitle(&format!("{} commits", commits.len()));
                *imp.repo_name.borrow_mut()    = repo_name;
                *imp.repo_path.borrow_mut()    = Some(path);
                *imp.repository.borrow_mut()   = Some(DebugRepository(reader.repo));
                *imp.all_commits.borrow_mut()  = commits.clone();
                *imp.last_query.borrow_mut()   = String::new();
                *imp.current_hash.borrow_mut() = None;
                *imp.current_dir.borrow_mut()  = PathBuf::new();
                imp.history_back.borrow_mut().clear();
                imp.history_forward.borrow_mut().clear();
                self.populate_commit_list(&commits);
                imp.commit_info_bar.set_revealed(false);
                self.show_empty_state();
            }
        }
    }

    // ── Commit list ───────────────────────────────────────────────────────────

    fn populate_commit_list(&self, commits: &[CommitInfo]) {
        let imp = self.imp();
        while let Some(child) = imp.commit_list.first_child() { imp.commit_list.remove(&child); }
        for commit in commits { imp.commit_list.append(&self.build_commit_row(commit)); }
    }

    fn build_commit_row(&self, commit: &CommitInfo) -> gtk::ListBoxRow {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical).spacing(2)
            .margin_top(6).margin_bottom(6).margin_start(12).margin_end(12)
            .build();
        let summary = gtk::Label::builder().label(&commit.summary).xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End).build();
        let meta = gtk::Label::builder()
            .label(&format!("{} · {}", &commit.hash[..8], commit.author)).xalign(0.0).build();
        meta.add_css_class("caption"); meta.add_css_class("dim-label");
        vbox.append(&summary); vbox.append(&meta);
        gtk::ListBoxRow::builder().name(&commit.hash).child(&vbox).build()
    }

    // ── Search ────────────────────────────────────────────────────────────────

    fn on_search_changed(&self, query: &str) {
        let imp = self.imp();
        { let last = imp.last_query.borrow(); if *last == query { return; } }
        *imp.last_query.borrow_mut() = query.to_owned();
        let all = imp.all_commits.borrow();
        if query.is_empty() { self.populate_commit_list(&all); return; }
        let q = query.to_lowercase();
        let filtered: Vec<CommitInfo> = all.iter().filter(|c| {
            c.summary.to_lowercase().contains(&q) || c.hash.starts_with(query) || c.author.to_lowercase().contains(&q)
        }).cloned().collect();
        drop(all);
        self.populate_commit_list(&filtered);
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
            if let Some(hash) = maybe_hash {
                self.browse_dir_inner(&hash, &dir);
            }
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
            if let Some(hash) = maybe_hash {
                self.browse_dir_inner(&hash, &dir);
            }
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

        let repo_ref = imp.repository.borrow();
        let repo = match repo_ref.as_ref() {
            Some(r) => r,
            None => { self.show_error_toast("No repository open."); return; }
        };

        let resolver = SnapshotResolver::new(repo);
        let all_nodes = match resolver.resolve_tree(hash) {
            Ok(n) => n,
            Err(e) => { self.show_error_toast(&format!("Cannot resolve snapshot: {e}")); return; }
        };

        let children = Self::direct_children(&all_nodes, dir);
        drop(repo_ref);

        self.update_address_bar(dir);

        let view_mode = *imp.view_mode.borrow();
        let widget: gtk::Widget = match view_mode {
            ViewMode::List => self.build_list_view(&children),
            ViewMode::Grid => self.build_grid_view(&children),
        };
        self.replace_right_panel(widget);
    }

    // ── List view ─────────────────────────────────────────────────────────────

    fn build_list_view(&self, children: &[TreeNode]) -> gtk::Widget {
        let scrolled = gtk::ScrolledWindow::builder()
            .vexpand(true).hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .build();
        list.add_css_class("boxed-list");

        if children.is_empty() {
            let placeholder = gtk::Label::builder().label("Empty directory")
                .margin_top(24).margin_bottom(24).build();
            placeholder.add_css_class("dim-label");
            list.append(&gtk::ListBoxRow::builder().child(&placeholder).build());
        } else {
            for node in children { list.append(&Self::build_file_row(node)); }
        }

        let children_clone = children.to_vec();
        list.connect_row_activated(glib::clone!(
            #[weak(rename_to = window)] self,
            move |_, row| {
                let idx = row.index() as usize;
                if let Some(node) = children_clone.get(idx) {
                    if node.is_dir() { window.enter_dir(node.path().to_path_buf()); }
                }
            }
        ));

        scrolled.set_child(Some(&list));
        scrolled.upcast()
    }

    fn build_file_row(node: &TreeNode) -> gtk::ListBoxRow {
        let hbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal).spacing(10)
            .margin_top(6).margin_bottom(6).margin_start(12).margin_end(12)
            .build();
        let icon_name = match node {
            TreeNode::Dir(p)  => {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                folder_icon_symbolic(name)
            }
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
            hbox.append(&chevron);
        }
        gtk::ListBoxRow::builder().child(&hbox).build()
    }

    // ── Grid view ─────────────────────────────────────────────────────────────

    fn build_grid_view(&self, children: &[TreeNode]) -> gtk::Widget {
        let scrolled = gtk::ScrolledWindow::builder()
            .vexpand(true).hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        // Match Nautilus: row/column spacing and padding
        let flow = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .homogeneous(true)
            .column_spacing(16)
            .row_spacing(16)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .max_children_per_line(64)
            .min_children_per_line(1)
            .build();
        flow.add_css_class("nautilus-grid-view");

        if children.is_empty() {
            let placeholder = gtk::Label::builder().label("Empty directory")
                .margin_top(24).margin_bottom(24).build();
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
        flow.connect_child_activated(glib::clone!(
            #[weak(rename_to = window)] self,
            move |_, child| {
                let idx = child.index() as usize;
                if let Some(node) = children_clone.get(idx) {
                    if node.is_dir() { window.enter_dir(node.path().to_path_buf()); }
                }
            }
        ));

        scrolled.set_child(Some(&flow));
        scrolled.upcast()
    }

    fn build_grid_cell(node: &TreeNode) -> gtk::Box {
        // Nautilus-style cell: 96px wide, 4px padding on each side,
        // 64px icon, label up to 3 lines with ellipsis.
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(4)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .width_request(96)
            .build();
        vbox.add_css_class("nautilus-view-cell");

        let icon_name = match node {
            TreeNode::Dir(p)  => {
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                folder_icon(name)
            }
            TreeNode::File(p) => mime_icon_full(p),
        };
        let icon = gtk::Image::from_icon_name(icon_name);
        icon.set_pixel_size(64);
        icon.set_halign(gtk::Align::Center);
        vbox.append(&icon);

        let name = node.path().file_name().and_then(|n| n.to_str()).unwrap_or("");
        let label = gtk::Label::builder()
            .label(name)
            .halign(gtk::Align::Center)
            .justify(gtk::Justification::Center)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::WordChar)
            .max_width_chars(12)
            .lines(3)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        label.add_css_class("caption");
        vbox.append(&label);
        vbox
    }

    // ── Direct children helper ────────────────────────────────────────────────

    fn direct_children(nodes: &[TreeNode], parent_dir: &PathBuf) -> Vec<TreeNode> {
        let depth = if parent_dir.as_os_str().is_empty() { 1 } else { parent_dir.components().count() + 1 };
        let mut dirs  = Vec::new();
        let mut files = Vec::new();
        for node in nodes {
            let p = node.path();
            if p.components().count() != depth { continue; }
            let is_child = parent_dir.as_os_str().is_empty() || p.starts_with(parent_dir);
            if !is_child { continue; }
            match node { TreeNode::Dir(_) => dirs.push(node.clone()), TreeNode::File(_) => files.push(node.clone()) }
        }
        let name = |n: &TreeNode| n.path().file_name().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        dirs.sort_by(|a, b| name(a).cmp(&name(b)));
        files.sort_by(|a, b| name(a).cmp(&name(b)));
        dirs.extend(files);
        dirs
    }

    // ── Address bar ───────────────────────────────────────────────────────────

    fn update_address_bar(&self, dir: &PathBuf) {
        let imp = self.imp();
        let bar = &imp.address_bar;

        while let Some(child) = bar.first_child() {
            bar.remove(&child);
        }

        // Get translation for "Home" (translated to e.g. "Pasta pessoal" in Portuguese)
        let get_home_label = || -> String {
            let gtk_translation = gettextrs::dgettext("gtk40", "Home");
            if gtk_translation != "Home" {
                return gtk_translation;
            }
            let lang = std::env::var("LANG").unwrap_or_default();
            if lang.starts_with("pt") {
                "Pasta pessoal".to_string()
            } else {
                "Home".to_string()
            }
        };

        struct PathComponent {
            label: String,
            icon: Option<String>,
            target_dir: PathBuf,
        }

        let mut components = Vec::new();
        let repo_path_opt = imp.repo_path.borrow().clone();

        if let Some(ref repo_path) = repo_path_opt {
            let absolute_path = repo_path.join(dir);
            let home_dir = glib::home_dir();

            if absolute_path.starts_with(&home_dir) {
                // Home directory component
                components.push(PathComponent {
                    label: get_home_label(),
                    icon: Some("user-home-symbolic".to_string()),
                    target_dir: PathBuf::new(),
                });

                if let Ok(relative_to_home) = absolute_path.strip_prefix(&home_dir) {
                    let mut current_absolute = home_dir.clone();
                    for seg in relative_to_home.components() {
                        let seg_str = seg.as_os_str().to_string_lossy().to_string();
                        current_absolute.push(&seg_str);

                        let target = if current_absolute.starts_with(repo_path) {
                            if let Ok(rel) = current_absolute.strip_prefix(repo_path) {
                                rel.to_path_buf()
                            } else {
                                PathBuf::new()
                            }
                        } else {
                            PathBuf::new()
                        };

                        components.push(PathComponent {
                            label: seg_str,
                            icon: None,
                            target_dir: target,
                        });
                    }
                }
            } else {
                // Fallback: Repository Root
                let repo_name = imp.repo_name.borrow().clone();
                components.push(PathComponent {
                    label: repo_name,
                    icon: Some("folder-symbolic".to_string()),
                    target_dir: PathBuf::new(),
                });

                let mut accumulated = PathBuf::new();
                for component in dir.components() {
                    let seg = component.as_os_str().to_string_lossy().to_string();
                    accumulated.push(&seg);
                    components.push(PathComponent {
                        label: seg,
                        icon: None,
                        target_dir: accumulated.clone(),
                    });
                }
            }
        } else {
            let repo_name = imp.repo_name.borrow().clone();
            components.push(PathComponent {
                label: repo_name,
                icon: Some("folder-symbolic".to_string()),
                target_dir: PathBuf::new(),
            });

            let mut accumulated = PathBuf::new();
            for component in dir.components() {
                let seg = component.as_os_str().to_string_lossy().to_string();
                accumulated.push(&seg);
                components.push(PathComponent {
                    label: seg,
                    icon: None,
                    target_dir: accumulated.clone(),
                });
            }
        }

        let total_components = components.len();
        for (idx, component) in components.iter().enumerate() {
            let is_current = idx == total_components - 1;

            if idx > 0 {
                let sep = gtk::Label::builder().label("/").build();
                sep.add_css_class("dim-label");
                sep.set_margin_start(2);
                sep.set_margin_end(2);
                bar.append(&sep);
            }

            let btn = gtk::Button::new();
            btn.add_css_class("flat");
            btn.add_css_class("nautilus-path-button");
            if is_current {
                btn.add_css_class("current-dir");
            }

            if component.icon.is_some() {
                let box_layout = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                let img = gtk::Image::from_icon_name(component.icon.as_ref().unwrap());
                if !is_current {
                    img.add_css_class("dim-label");
                }
                box_layout.append(&img);

                let lbl = gtk::Label::builder()
                    .label(&component.label)
                    .single_line_mode(true)
                    .build();
                if !is_current {
                    lbl.add_css_class("dim-label");
                }
                box_layout.append(&lbl);
                btn.set_child(Some(&box_layout));
            } else {
                let lbl = gtk::Label::builder()
                    .label(&component.label)
                    .single_line_mode(true)
                    .build();
                if !is_current {
                    lbl.add_css_class("dim-label");
                }
                btn.set_child(Some(&lbl));
            }

            let target = component.target_dir.clone();
            btn.connect_clicked(glib::clone!(
                #[weak(rename_to = window)] self,
                move |_| window.enter_dir(target.clone())
            ));

            bar.append(&btn);
        }

        imp.address_bar.set_visible(true);
        imp.window_title.set_visible(false);
    }

    // ── Panel helpers ─────────────────────────────────────────────────────────

    fn show_empty_state(&self) {
        let imp = self.imp();
        imp.address_bar.set_visible(false);
        imp.window_title.set_visible(true);
        imp.nav_back_button.set_sensitive(false);
        imp.nav_forward_button.set_sensitive(false);
        self.replace_right_panel(imp.empty_state.clone().upcast());
    }

    fn replace_right_panel(&self, widget: gtk::Widget) {
        self.imp().content_toolbar_view.set_content(Some(&widget));
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
        Some("rs")                                       => "text-x-rust-symbolic",
        Some("toml")|Some("yaml")|Some("yml")|Some("json") => "text-x-script-symbolic",
        Some("md")|Some("txt")                           => "text-x-generic-symbolic",
        Some("png")|Some("jpg")|Some("jpeg")|Some("svg")|Some("webp") => "image-x-generic-symbolic",
        Some("mp3")|Some("ogg")|Some("flac")|Some("wav") => "audio-x-generic-symbolic",
        Some("sh")|Some("bash")                          => "text-x-script-symbolic",
        Some("c")|Some("h")|Some("cpp")|Some("hpp")      => "text-x-csrc-symbolic",
        Some("py")                                       => "text-x-python-symbolic",
        Some("html")|Some("css")                         => "text-html-symbolic",
        _                                                => "text-x-generic-symbolic",
    }
}

fn mime_icon_full(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs")                                       => "text-x-rust",
        Some("toml")|Some("yaml")|Some("yml")|Some("json") => "text-x-script",
        Some("md")|Some("txt")                           => "text-x-generic",
        Some("png")|Some("jpg")|Some("jpeg")|Some("svg")|Some("webp") => "image-x-generic",
        Some("mp3")|Some("ogg")|Some("flac")|Some("wav") => "audio-x-generic",
        Some("sh")|Some("bash")                          => "text-x-script",
        Some("c")|Some("h")|Some("cpp")|Some("hpp")      => "text-x-csrc",
        Some("py")                                       => "text-x-python",
        Some("html")|Some("css")                         => "text-html",
        _                                                => "text-x-generic",
    }
}

// ── Folder icon helpers ──────────────────────────────────────────────────────

fn folder_icon(name: &str) -> &'static str {
    match name.to_lowercase().as_str() {
        "src" | "code" | "devel" | "development" | "projects" | "projetos" => "folder-development",
        "doc" | "docs" | "documents" | "documentos" => "folder-documents",
        "data" | "db" | "database" => "folder-documents",
        "test" | "tests" | "spec" | "specs" => "folder-remote",
        "images" | "img" | "pictures" | "imagens" => "folder-pictures",
        "videos" | "video" => "folder-videos",
        "music" | "audio" | "músicas" | "musicas" => "folder-music",
        "download" | "downloads" => "folder-download",
        _ => "folder",
    }
}

fn folder_icon_symbolic(name: &str) -> &'static str {
    match name.to_lowercase().as_str() {
        "src" | "code" | "devel" | "development" | "projects" | "projetos" => "folder-development-symbolic",
        "doc" | "docs" | "documents" | "documentos" => "folder-documents-symbolic",
        "images" | "img" | "pictures" | "imagens" => "folder-pictures-symbolic",
        "videos" | "video" => "folder-videos-symbolic",
        "music" | "audio" | "músicas" | "musicas" => "folder-music-symbolic",
        "download" | "downloads" => "folder-download-symbolic",
        _ => "folder-symbolic",
    }
}
