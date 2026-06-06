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
        /// All commits currently displayed (may be a filtered subset).
        pub all_commits: RefCell<Vec<CommitInfo>>,
        /// Path to the currently open repository.
        pub repo_path: RefCell<Option<PathBuf>>,
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

        // "Open Repository" button
        imp.open_repo_button.connect_clicked(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| window.open_repository_dialog()
        ));

        // Commit row selected
        imp.commit_list.connect_row_selected(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, row| window.on_commit_selected(row)
        ));

        // Search entry changed
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

    /// Opens the repository at `path`, populates the commit list and updates
    /// the window title.
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

                // Update window title with the folder name
                let repo_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("Repository");
                imp.window_title.set_title(repo_name);
                imp.window_title
                    .set_subtitle(&format!("{} commits", commits.len()));

                // Store state
                *imp.repo_path.borrow_mut() = Some(path);
                *imp.all_commits.borrow_mut() = commits.clone();

                // Populate the list
                self.populate_commit_list(&commits);

                // Reset right panel
                imp.commit_info_bar.set_revealed(false);
                self.show_empty_state();
            }
        }
    }

    // ── Commit list helpers ───────────────────────────────────────────────────

    /// Clears and repopulates `commit_list` from `commits`.
    fn populate_commit_list(&self, commits: &[CommitInfo]) {
        let imp = self.imp();

        // Remove all existing rows
        while let Some(child) = imp.commit_list.first_child() {
            imp.commit_list.remove(&child);
        }

        for commit in commits {
            let row = self.build_commit_row(commit);
            imp.commit_list.append(&row);
        }
    }

    /// Builds a single `gtk::ListBoxRow` for one commit.
    fn build_commit_row(&self, commit: &CommitInfo) -> gtk::ListBoxRow {
        let vbox = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(12)
            .margin_end(12)
            .build();

        // Summary line
        let summary = gtk::Label::builder()
            .label(&commit.summary)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();

        // Hash + author line
        let meta = gtk::Label::builder()
            .label(&format!("{} · {}", &commit.hash[..8], commit.author))
            .xalign(0.0)
            .build();
        meta.add_css_class("caption");
        meta.add_css_class("dim-label");

        vbox.append(&summary);
        vbox.append(&meta);

        // Store the full hash as widget name for retrieval on selection
        gtk::ListBoxRow::builder()
            .name(&commit.hash)
            .child(&vbox)
            .build()
    }

    // ── Search ────────────────────────────────────────────────────────────────

    fn on_search_changed(&self, query: &str) {
        let imp = self.imp();
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
                    || c.hash.starts_with(&q)
                    || c.author.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();

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

        // Retrieve the full hash stored as the widget name
        let hash = row.widget_name().to_string();

        // Look up the CommitInfo for this hash
        let commit = {
            let all = imp.all_commits.borrow();
            all.iter().find(|c| c.hash == hash).cloned()
        };

        let commit = match commit {
            Some(c) => c,
            None => return,
        };

        // Update the bottom action bar
        imp.commit_hash_label.set_label(&commit.hash[..12]);
        imp.commit_message_label.set_label(&commit.summary);
        imp.commit_date_label
            .set_label(&Self::format_timestamp(commit.timestamp));
        imp.commit_info_bar.set_revealed(true);

        // Materialize the file tree for this revision
        let repo_path = imp.repo_path.borrow().clone();
        if let Some(path) = repo_path {
            self.show_file_tree(&path, &hash);
        }
    }

    // ── File tree rendering ───────────────────────────────────────────────────

    /// Resolves `hash` against the repository at `repo_path` and renders the
    /// resulting file tree in the right panel.
    fn show_file_tree(&self, repo_path: &std::path::Path, hash: &str) {
        let reader = match HistoryReader::open(repo_path) {
            Ok(r) => r,
            Err(e) => {
                self.show_error_toast(&format!("Cannot open repo: {e}"));
                return;
            }
        };

        let resolver = SnapshotResolver::new(&reader.repo);
        let nodes = match resolver.resolve_tree(hash) {
            Ok(n) => n,
            Err(e) => {
                self.show_error_toast(&format!("Cannot resolve snapshot: {e}"));
                return;
            }
        };

        // Build a scrollable tree view in the right panel
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
            let row = Self::build_tree_row(node);
            list.append(&row);
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

    /// Builds a single row for a [`TreeNode`].
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
            TreeNode::Dir(p) => (
                "folder-symbolic",
                p.components().count().saturating_sub(1),
            ),
            TreeNode::File(p) => (
                "text-x-generic-symbolic",
                p.components().count().saturating_sub(1),
            ),
        };

        // Indent based on depth
        let indent = gtk::Box::builder()
            .width_request((depth as i32) * 16)
            .build();
        hbox.append(&indent);

        let icon = gtk::Image::from_icon_name(icon_name);
        hbox.append(&icon);

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

    /// Shows the empty state placeholder in the right panel.
    fn show_empty_state(&self) {
        self.replace_right_panel(self.imp().empty_state.clone().upcast());
    }

    /// Replaces the [end] child of the main GtkPaned with `widget`.
    fn replace_right_panel(&self, widget: gtk::Widget) {
        // Walk: AdwApplicationWindow → ToolbarView → content → Paned
        if let Some(toolbar_view) = self
            .content()
            .and_then(|w| w.downcast::<adw::ToolbarView>().ok())
        {
            if let Some(paned) = toolbar_view
                .content()
                .and_then(|w| w.downcast::<gtk::Paned>().ok())
            {
                paned.set_end_child(Some(&widget));
            }
        }
    }

    // ── Utilities ─────────────────────────────────────────────────────────────

    /// Shows a brief error toast at the top of the window.
    fn show_error_toast(&self, message: &str) {
        let toast = adw::Toast::builder()
            .title(message)
            .timeout(4)
            .build();

        // Walk up to the AdwToastOverlay if present; fall back to stderr.
        if let Some(overlay) = self
            .content()
            .and_then(|w| w.downcast::<adw::ToastOverlay>().ok())
        {
            overlay.add_toast(toast);
        } else {
            eprintln!("[temporal-explorer] {message}");
        }
    }

    /// Formats a Unix timestamp into a human-readable date string.
    fn format_timestamp(ts: i64) -> String {
        if let Ok(dt) = glib::DateTime::from_unix_local(ts) {
            dt.format("%Y-%m-%d %H:%M").unwrap_or_default().to_string()
        } else {
            ts.to_string()
        }
    }
}
