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

// ── DebugRepository ───────────────────────────────────────────────────────────

pub(super) struct DebugRepository(git2::Repository);

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
        // Header bar
        #[template_child]
        pub open_repo_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub back_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,

        // Breadcrumb bar (populated dynamically)
        #[template_child]
        pub breadcrumb_bar: TemplateChild<gtk::Box>,

        // Left panel
        #[template_child]
        pub commit_search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub commit_list: TemplateChild<gtk::ListBox>,

        // Right panel
        #[template_child]
        pub empty_state: TemplateChild<adw::StatusPage>,

        // Main paned
        #[template_child]
        pub main_paned: TemplateChild<gtk::Paned>,

        // Bottom bar
        #[template_child]
        pub commit_info_bar: TemplateChild<gtk::ActionBar>,
        #[template_child]
        pub commit_hash_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub commit_message_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub commit_date_label: TemplateChild<gtk::Label>,

        // Runtime state
        pub all_commits: RefCell<Vec<CommitInfo>>,
        pub repo_path: RefCell<Option<PathBuf>>,
        pub repository: RefCell<Option<DebugRepository>>,
        pub last_query: RefCell<String>,
        /// Hash of the commit currently displayed in the right panel.
        pub current_hash: RefCell<Option<String>>,
        /// Directory being browsed inside that commit (relative to repo root).
        pub current_dir: RefCell<PathBuf>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TemporalExplorerWindow {
        const NAME: &'static str = "TemporalExplorerWindow";
        type Type = super::TemporalExplorerWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
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
            gio::ActionGroup,
            gio::ActionMap,
            gtk::Accessible,
            gtk::Buildable,
            gtk::ConstraintTarget,
            gtk::Native,
            gtk::Root,
            gtk::ShortcutManager;
}

impl TemporalExplorerWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    // ── Signal wiring ─────────────────────────────────────────────────────────

    fn setup_callbacks(&self) {
        let imp = self.imp();

        imp.open_repo_button.connect_clicked(glib::clone!(
            #[weak(rename_to = window)] self,
            move |_| window.open_repository_dialog()
        ));

        imp.back_button.connect_clicked(glib::clone!(
            #[weak(rename_to = window)] self,
            move |_| window.navigate_up()
        ));

        imp.commit_list.connect_row_selected(glib::clone!(
            #[weak(rename_to = window)] self,
            move |_, row| window.on_commit_selected(row)
        ));

        imp.commit_search_entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = window)] self,
            move |entry| window.on_search_changed(entry.text().as_str())
        ));
    }

    // ── Open repository ───────────────────────────────────────────────────────

    fn open_repository_dialog(&self) {
        let dialog = gtk::FileDialog::builder()
            .title("Open Git Repository")
            .modal(true)
            .build();

        dialog.select_folder(
            Some(self),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = window)] self,
                move |result| {
                    if let Ok(folder) = result {
                        if let Some(path) = folder.path() {
                            window.load_repository(path);
                        }
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
                    Err(e) => {
                        self.show_error_toast(&format!("Failed to read history: {e}"));
                        return;
                    }
                };

                let repo_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Repository");
                imp.window_title.set_title(repo_name);
                imp.window_title.set_subtitle(&format!("{} commits", commits.len()));

                *imp.repo_path.borrow_mut() = Some(path);
                *imp.repository.borrow_mut() = Some(DebugRepository(reader.repo));
                *imp.all_commits.borrow_mut() = commits.clone();
                *imp.last_query.borrow_mut() = String::new();
                *imp.current_hash.borrow_mut() = None;
                *imp.current_dir.borrow_mut() = PathBuf::new();

                self.populate_commit_list(&commits);
                imp.commit_info_bar.set_revealed(false);
                self.show_empty_state();
            }
        }
    }

    // ── Commit list ───────────────────────────────────────────────────────────

    fn populate_commit_list(&self, commits: &[CommitInfo]) {
        let imp = self.imp();
        while let Some(child) = imp.commit_list.first_child() {
            imp.commit_list.remove(&child);
        }
        for commit in commits {
            imp.commit_list.append(&self.build_commit_row(commit));
        }
    }

    fn build_commit_row(&self, commit: &CommitInfo) -> gtk::ListBoxRow {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .margin_top(6).margin_bottom(6)
            .margin_start(12).margin_end(12)
            .build();

        let summary = gtk::Label::builder()
            .label(&commit.summary)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();

        let meta = gtk::Label::builder()
            .label(&format!("{} · {}", &commit.hash[..8], commit.author))
            .xalign(0.0)
            .build();
        meta.add_css_class("caption");
        meta.add_css_class("dim-label");

        vbox.append(&summary);
        vbox.append(&meta);

        gtk::ListBoxRow::builder()
            .name(&commit.hash)
            .child(&vbox)
            .build()
    }

    // ── Search ────────────────────────────────────────────────────────────────

    fn on_search_changed(&self, query: &str) {
        let imp = self.imp();
        {
            let last = imp.last_query.borrow();
            if *last == query { return; }
        }
        *imp.last_query.borrow_mut() = query.to_owned();

        let all = imp.all_commits.borrow();
        if query.is_empty() {
            self.populate_commit_list(&all);
            return;
        }
        let q = query.to_lowercase();
        let filtered: Vec<CommitInfo> = all.iter().filter(|c| {
            c.summary.to_lowercase().contains(&q)
                || c.hash.starts_with(query)
                || c.author.to_lowercase().contains(&q)
        }).cloned().collect();
        drop(all);
        self.populate_commit_list(&filtered);
    }

    // ── Commit selected ───────────────────────────────────────────────────────

    fn on_commit_selected(&self, row: Option<&gtk::ListBoxRow>) {
        let imp = self.imp();

        let row = match row {
            Some(r) => r,
            None => {
                imp.commit_info_bar.set_revealed(false);
                self.show_empty_state();
                return;
            }
        };

        let hash = row.widget_name().to_string();
        let commit = {
            let all = imp.all_commits.borrow();
            all.iter().find(|c| c.hash == hash).cloned()
        };
        let commit = match commit { Some(c) => c, None => return };

        imp.commit_hash_label.set_label(&commit.hash[..12]);
        imp.commit_message_label.set_label(&commit.summary);
        imp.commit_date_label.set_label(&Self::format_timestamp(commit.timestamp));
        imp.commit_info_bar.set_revealed(true);

        // Reset to repo root when switching commits
        *imp.current_hash.borrow_mut() = Some(hash.clone());
        *imp.current_dir.borrow_mut() = PathBuf::new();

        self.browse_dir(&hash, &PathBuf::new());
    }

    // ── Directory navigation ──────────────────────────────────────────────────

    /// Navigate into `dir` for the current commit.
    fn enter_dir(&self, dir: PathBuf) {
        let hash = match self.imp().current_hash.borrow().clone() {
            Some(h) => h,
            None => return,
        };
        *self.imp().current_dir.borrow_mut() = dir.clone();
        self.browse_dir(&hash, &dir);
    }

    /// Navigate one level up (parent directory).
    fn navigate_up(&self) {
        let current = self.imp().current_dir.borrow().clone();
        let parent = current.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let hash = match self.imp().current_hash.borrow().clone() {
            Some(h) => h,
            None => return,
        };
        *self.imp().current_dir.borrow_mut() = parent.clone();
        self.browse_dir(&hash, &parent);
    }

    /// Core render: show only the direct children of `dir` inside `hash`.
    fn browse_dir(&self, hash: &str, dir: &PathBuf) {
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

        // Filter: only direct children of `dir`
        let children = Self::direct_children(&all_nodes, dir);

        // Update breadcrumb and back button
        self.update_breadcrumb(dir);
        imp.back_button.set_visible(!dir.as_os_str().is_empty());
        imp.breadcrumb_bar.set_visible(true);

        // Build the file list view
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
                .label("Empty directory")
                .margin_top(24).margin_bottom(24)
                .build();
            placeholder.add_css_class("dim-label");
            list.append(&gtk::ListBoxRow::builder().child(&placeholder).build());
        } else {
            for node in &children {
                list.append(&Self::build_file_row(node));
            }
        }

        // Activate row = enter directory (files: no-op for now)
        let children_clone = children.clone();
        list.connect_row_activated(glib::clone!(
            #[weak(rename_to = window)] self,
            move |_, row| {
                let idx = row.index() as usize;
                if let Some(node) = children_clone.get(idx) {
                    if node.is_dir() {
                        window.enter_dir(node.path().to_path_buf());
                    }
                }
            }
        ));

        scrolled.set_child(Some(&list));
        self.replace_right_panel(scrolled.upcast());
    }

    /// Returns only the direct children of `parent_dir` from a flat node list.
    /// Dirs come first, then files, both sorted alphabetically.
    fn direct_children(nodes: &[TreeNode], parent_dir: &PathBuf) -> Vec<TreeNode> {
        let depth = if parent_dir.as_os_str().is_empty() {
            1
        } else {
            parent_dir.components().count() + 1
        };

        let mut dirs: Vec<TreeNode> = Vec::new();
        let mut files: Vec<TreeNode> = Vec::new();

        for node in nodes {
            let p = node.path();
            let node_depth = p.components().count();
            if node_depth != depth { continue; }

            // Must be a direct child of parent_dir
            let is_child = if parent_dir.as_os_str().is_empty() {
                true
            } else {
                p.starts_with(parent_dir)
            };

            if !is_child { continue; }

            match node {
                TreeNode::Dir(_) => dirs.push(node.clone()),
                TreeNode::File(_) => files.push(node.clone()),
            }
        }

        let name = |n: &TreeNode| {
            n.path().file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_lowercase()
        };
        dirs.sort_by(|a, b| name(a).cmp(&name(b)));
        files.sort_by(|a, b| name(a).cmp(&name(b)));
        dirs.extend(files);
        dirs
    }

    /// Builds a Nautilus-style list row for one file/dir entry.
    fn build_file_row(node: &TreeNode) -> gtk::ListBoxRow {
        let hbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(10)
            .margin_top(6).margin_bottom(6)
            .margin_start(12).margin_end(12)
            .build();

        let icon_name = match node {
            TreeNode::Dir(_) => "folder-symbolic",
            TreeNode::File(p) => mime_icon(p),
        };

        let icon = gtk::Image::from_icon_name(icon_name);
        icon.set_pixel_size(16);
        hbox.append(&icon);

        let name = node.path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        let label = gtk::Label::builder()
            .label(name)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();

        if node.is_dir() {
            label.add_css_class("body");
        }

        hbox.append(&label);

        // Chevron for directories
        if node.is_dir() {
            let chevron = gtk::Image::from_icon_name("go-next-symbolic");
            chevron.add_css_class("dim-label");
            hbox.append(&chevron);
        }

        gtk::ListBoxRow::builder().child(&hbox).build()
    }

    // ── Breadcrumb ────────────────────────────────────────────────────────────

    /// Rebuilds the breadcrumb bar for the given `dir`.
    fn update_breadcrumb(&self, dir: &PathBuf) {
        let bar = &self.imp().breadcrumb_bar;

        // Clear existing crumbs
        while let Some(child) = bar.first_child() {
            bar.remove(&child);
        }

        // "Home" button always present
        let home_btn = gtk::Button::builder()
            .label("Home")
            .build();
        home_btn.add_css_class("flat");
        home_btn.connect_clicked(glib::clone!(
            #[weak(rename_to = window)] self,
            move |_| {
                let hash = match window.imp().current_hash.borrow().clone() {
                    Some(h) => h,
                    None => return,
                };
                *window.imp().current_dir.borrow_mut() = PathBuf::new();
                window.browse_dir(&hash, &PathBuf::new());
            }
        ));
        bar.append(&home_btn);

        // One button per path component
        let mut accumulated = PathBuf::new();
        for component in dir.components() {
            let seg = component.as_os_str().to_string_lossy().to_string();
            accumulated.push(&seg);

            let sep = gtk::Label::builder().label(" › ").build();
            sep.add_css_class("dim-label");
            bar.append(&sep);

            let btn = gtk::Button::builder().label(&seg).build();
            btn.add_css_class("flat");
            let target = accumulated.clone();
            btn.connect_clicked(glib::clone!(
                #[weak(rename_to = window)] self,
                move |_| window.enter_dir(target.clone())
            ));
            bar.append(&btn);
        }
    }

    // ── Panel helpers ─────────────────────────────────────────────────────────

    fn show_empty_state(&self) {
        let imp = self.imp();
        imp.breadcrumb_bar.set_visible(false);
        imp.back_button.set_visible(false);
        self.replace_right_panel(imp.empty_state.clone().upcast());
    }

    fn replace_right_panel(&self, widget: gtk::Widget) {
        self.imp().main_paned.set_end_child(Some(&widget));
    }

    // ── Utilities ─────────────────────────────────────────────────────────────

    fn show_error_toast(&self, message: &str) {
        let toast = adw::Toast::builder()
            .title(message)
            .timeout(4)
            .build();
        if let Some(overlay) = self
            .content()
            .and_then(|w| w.downcast::<adw::ToastOverlay>().ok())
        {
            overlay.add_toast(toast);
        } else {
            eprintln!("[temporal-explorer] {message}");
        }
    }

    fn format_timestamp(ts: i64) -> String {
        if let Ok(dt) = glib::DateTime::from_unix_local(ts) {
            dt.format("%Y-%m-%d %H:%M").unwrap_or_default().to_string()
        } else {
            ts.to_string()
        }
    }
}

// ── MIME icon helper ──────────────────────────────────────────────────────────

/// Returns a themed icon name based on the file extension.
fn mime_icon(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs")                        => "text-x-rust-symbolic",
        Some("toml") | Some("yaml") |
        Some("yml")  | Some("json")       => "text-x-script-symbolic",
        Some("md") | Some("txt")          => "text-x-generic-symbolic",
        Some("png") | Some("jpg") |
        Some("jpeg")| Some("svg") |
        Some("webp")                      => "image-x-generic-symbolic",
        Some("mp3") | Some("ogg") |
        Some("flac")| Some("wav")         => "audio-x-generic-symbolic",
        Some("sh") | Some("bash")         => "text-x-script-symbolic",
        Some("c") | Some("h") |
        Some("cpp") | Some("hpp")         => "text-x-csrc-symbolic",
        Some("py")                        => "text-x-python-symbolic",
        Some("html")| Some("css")         => "text-html-symbolic",
        _                                 => "text-x-generic-symbolic",
    }
}
