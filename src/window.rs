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
//
// git2::Repository does not implement Debug, but the imp struct derives it.
// pub(super) matches the effective visibility of the imp fields and silences
// the private_interfaces warning.

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
        pub window_title: TemplateChild<adw::WindowTitle>,

        // Left panel
        #[template_child]
        pub commit_search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub commit_list: TemplateChild<gtk::ListBox>,

        // Right panel
        #[template_child]
        pub empty_state: TemplateChild<adw::StatusPage>,

        // Main paned — held as TemplateChild to avoid fragile widget walking
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
        /// All commits currently loaded from the open repository.
        pub all_commits: RefCell<Vec<CommitInfo>>,
        /// Path to the currently open repository.
        pub repo_path: RefCell<Option<PathBuf>>,
        /// The open Repository wrapped in DebugRepository so #[derive(Debug)]
        /// compiles.  Reused across commit selections to avoid reopening the
        /// repo on every click.
        pub repository: RefCell<Option<DebugRepository>>,
        /// Last search query — used to skip redundant repopulations.
        pub last_query: RefCell<String>,
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
            #[weak(rename_to = window)]
            self,
            move |_| window.open_repository_dialog()
        ));

        imp.commit_list.connect_row_selected(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, row| window.on_commit_selected(row)
        ));

        imp.commit_search_entry.connect_search_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
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
                #[weak(rename_to = window)]
                self,
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
                imp.window_title
                    .set_subtitle(&format!("{} commits", commits.len()));

                *imp.repo_path.borrow_mut() = Some(path);
                *imp.repository.borrow_mut() = Some(DebugRepository(reader.repo));
                *imp.all_commits.borrow_mut() = commits.clone();
                *imp.last_query.borrow_mut() = String::new();

                self.populate_commit_list(&commits);
                imp.commit_info_bar.set_revealed(false);
                self.show_empty_state();
            }
        }
    }

    // ── Commit list helpers ───────────────────────────────────────────────────

    fn populate_commit_list(&self, commits: &[CommitInfo]) {
        let imp = self.imp();
        while let Some(child) = imp.commit_list.first_child() {
            imp.commit_list.remove(&child);
        }
        for commit in commits {
            let row = self.build_commit_row(commit);
            imp.commit_list.append(&row);
        }
    }

    fn build_commit_row(&self, commit: &CommitInfo) -> gtk::ListBoxRow {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
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
            if *last == query {
                return;
            }
        }
        *imp.last_query.borrow_mut() = query.to_owned();

        let all = imp.all_commits.borrow();

        if query.is_empty() {
            self.populate_commit_list(&all);
            return;
        }

        let q = query.to_lowercase();
        let filtered: Vec<CommitInfo> = all
            .iter()
            .filter(|c| {
                c.summary.to_lowercase().contains(&q)
                    || c.hash.starts_with(query)
                    || c.author.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();

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

        let commit = match commit {
            Some(c) => c,
            None => return,
        };

        imp.commit_hash_label.set_label(&commit.hash[..12]);
        imp.commit_message_label.set_label(&commit.summary);
        imp.commit_date_label
            .set_label(&Self::format_timestamp(commit.timestamp));
        imp.commit_info_bar.set_revealed(true);

        self.show_file_tree(&hash);
    }

    // ── File tree rendering ───────────────────────────────────────────────────

    fn show_file_tree(&self, hash: &str) {
        let imp = self.imp();

        let repo_ref = imp.repository.borrow();
        let repo = match repo_ref.as_ref() {
            Some(r) => r,
            None => {
                self.show_error_toast("No repository open.");
                return;
            }
        };

        let resolver = SnapshotResolver::new(repo);
        let nodes = match resolver.resolve_tree(hash) {
            Ok(n) => n,
            Err(e) => {
                self.show_error_toast(&format!("Cannot resolve snapshot: {e}"));
                return;
            }
        };

        let scrolled = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .hexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .build();

        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        list.add_css_class("boxed-list");

        for node in &nodes {
            list.append(&Self::build_tree_row(node));
        }

        if nodes.is_empty() {
            let placeholder = gtk::Label::builder()
                .label("Empty snapshot")
                .margin_top(24)
                .margin_bottom(24)
                .build();
            placeholder.add_css_class("dim-label");
            list.append(&gtk::ListBoxRow::builder().child(&placeholder).build());
        }

        scrolled.set_child(Some(&list));
        self.replace_right_panel(scrolled.upcast());
    }

    fn build_tree_row(node: &TreeNode) -> gtk::ListBoxRow {
        let hbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(8)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(12)
            .margin_end(12)
            .build();

        let (icon_name, depth) = match node {
            TreeNode::Dir(p) => ("folder-symbolic", p.components().count().saturating_sub(1)),
            TreeNode::File(p) => ("text-x-generic-symbolic", p.components().count().saturating_sub(1)),
        };

        let indent = gtk::Box::builder()
            .width_request((depth as i32) * 16)
            .build();
        hbox.append(&indent);
        hbox.append(&gtk::Image::from_icon_name(icon_name));

        let label_text = node
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        let label = gtk::Label::builder()
            .label(label_text)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .hexpand(true)
            .build();

        if node.is_dir() {
            label.add_css_class("heading");
        }

        hbox.append(&label);
        gtk::ListBoxRow::builder().child(&hbox).build()
    }

    // ── Panel helpers ─────────────────────────────────────────────────────────

    fn show_empty_state(&self) {
        self.replace_right_panel(self.imp().empty_state.clone().upcast());
    }

    /// Uses the `main_paned` TemplateChild directly — no fragile widget walking.
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
