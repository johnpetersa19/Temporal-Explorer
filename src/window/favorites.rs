/* favorites.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Favorite snapshots shown from the main menu.

use super::*;
use super::util::*;

impl TemporalExplorerWindow {
    pub(super) fn show_favorites(&self) {
        let favorites = self
            .imp()
            .settings
            .get()
            .map(|settings| {
                settings
                    .strv("favorite-snapshot-items")
                    .iter()
                    .filter_map(|item| {
                        let mut parts = item.splitn(3, '\n');
                        Some((
                            PathBuf::from(parts.next()?),
                            parts.next()?.to_string(),
                            PathBuf::from(parts.next()?),
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let dialog = gtk::Window::builder()
            .title(gettext("Favorite Snapshots"))
            .transient_for(self)
            .modal(true)
            .default_width(520)
            .default_height(440)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);

        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");

        if favorites.is_empty() {
            let empty = gtk::Label::new(Some(&gettext(
                "No favorite snapshot items yet. Star an item in its properties.",
            )));
            empty.set_wrap(true);
            empty.set_margin_top(24);
            empty.set_margin_bottom(24);
            list.append(&empty);
        } else {
            for (repo_path, hash, item_path) in favorites {
                let repo_name = repo_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_else(|| repo_path.to_str().unwrap_or_default());
                let label = gtk::Label::new(Some(&format!(
                    "{}  ·  {}  ·  {}",
                    repo_name,
                    short_hash(&hash),
                    item_path.display()
                )));
                label.set_xalign(0.0);
                label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                label.set_margin_top(8);
                label.set_margin_bottom(8);
                label.set_margin_start(10);
                label.set_margin_end(10);

                let button = gtk::Button::new();
                button.set_child(Some(&label));
                button.add_css_class("flat");
                let row = gtk::ListBoxRow::new();
                row.set_child(Some(&button));
                list.append(&row);

                let window = self.clone();
                let dialog = dialog.clone();
                let favorite_repo = repo_path.clone();
                let favorite_hash = hash.clone();
                let favorite_path = item_path.clone();
                button.connect_clicked(move |_| {
                    dialog.close();
                    window.open_favorite(
                        favorite_repo.clone(),
                        favorite_hash.clone(),
                        favorite_path.clone(),
                    );
                });
            }
        }

        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&list)
            .build();
        content.append(&scroll);
        dialog.set_child(Some(&content));
        dialog.present();
    }

    pub(super) fn open_favorite(&self, repo: PathBuf, hash: String, path: PathBuf) {
        let loaded_repo = self.imp().repo_path.borrow().clone();
        if loaded_repo.as_deref() != Some(repo.as_path()) {
            *self.imp().pending_favorite.borrow_mut() = Some((repo.clone(), hash, path));
            self.load_repository(repo);
            return;
        }

        self.open_loaded_favorite(repo, hash, path);
    }

    pub(super) fn open_loaded_favorite(&self, repo: PathBuf, hash: String, path: PathBuf) {
        if self.imp().repo_path.borrow().as_deref() != Some(repo.as_path()) {
            return;
        }
        if !self.imp().commit_index.borrow().contains_key(&hash) {
            self.show_toast(&gettext("The favorite snapshot is no longer available"));
            return;
        }

        let is_directory = git2::Repository::open(&repo)
            .ok()
            .and_then(|repository| {
                let oid = git2::Oid::from_str(&hash).ok()?;
                let commit = repository.find_commit(oid).ok()?;
                let tree = commit.tree().ok()?;
                tree.get_path(&path)
                    .ok()
                    .map(|entry| entry.kind() == Some(git2::ObjectType::Tree))
            })
            .unwrap_or(false);
        let directory = if is_directory {
            path.clone()
        } else {
            path.parent().unwrap_or_else(|| Path::new("")).to_path_buf()
        };

        self.on_commit_selected(hash);
        self.navigate_to_dir(directory);
        self.show_toast(&gettext("Opened favorite snapshot item"));
    }
}
