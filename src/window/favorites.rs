/* favorites.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Favorite snapshots shown from the main menu.

use super::*;

impl TemporalExplorerWindow {
    pub(super) fn current_favorite_id(&self) -> Option<String> {
        let imp = self.imp();
        let repo = imp.repo_path.borrow().clone()?;
        let hash = imp.current_hash.borrow().clone()?;
        let path = self
            .selected_file_view_node()
            .map(|node| node.path().to_path_buf())
            .unwrap_or_else(|| imp.current_dir.borrow().clone());
        Some(format!("{}\n{}\n{}", repo.display(), hash, path.display()))
    }

    pub(super) fn is_favorite_id(&self, favorite_id: &str) -> bool {
        self.imp()
            .settings
            .get()
            .map(|settings| {
                settings
                    .strv("favorite-snapshot-items")
                    .iter()
                    .any(|item| item.as_str() == favorite_id)
            })
            .unwrap_or(false)
    }

    pub(super) fn update_star_button(&self) {
        let starred = self
            .current_favorite_id()
            .is_some_and(|id| self.is_favorite_id(&id));
        let button = &self.imp().star_button;
        button.set_icon_name(if starred {
            "starred-symbolic"
        } else {
            "non-starred-symbolic"
        });
        button.set_tooltip_text(Some(&if starred {
            gettext("Remove from Favorites")
        } else {
            gettext("Add to Favorites")
        }));
    }

    pub(super) fn toggle_star_current_snapshot(&self) {
        let Some(favorite_id) = self.current_favorite_id() else {
            self.show_toast(&gettext("No snapshot selected"));
            return;
        };

        let Some(settings) = self.imp().settings.get() else {
            return;
        };

        let mut favorites: Vec<String> = settings
            .strv("favorite-snapshot-items")
            .iter()
            .map(|item| item.to_string())
            .collect();
        let was_starred = favorites.iter().any(|item| item == &favorite_id);
        favorites.retain(|item| item != &favorite_id);
        if !was_starred {
            favorites.push(favorite_id);
        }
        settings
            .set_value("favorite-snapshot-items", &favorites.to_variant())
            .ok();

        self.update_star_button();
        let msg = if was_starred {
            gettext("Removed from favorites")
        } else {
            gettext("Marked as favorite")
        };
        self.show_toast(&msg);
    }

    pub(super) fn show_favorites(&self) {
        let entries = self
            .imp()
            .settings
            .get()
            .map(|settings| {
                settings
                    .strv("favorite-snapshot-items")
                    .iter()
                    .filter_map(|item| {
                        let raw_id = item.to_string();
                        let mut parts = item.splitn(3, '\n');
                        Some(crate::favorites_dialog::FavoriteEntry {
                            raw_id,
                            repo_path: PathBuf::from(parts.next()?),
                            hash: parts.next()?.to_string(),
                            item_path: PathBuf::from(parts.next()?),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        let dialog = crate::favorites_dialog::FavoritesDialog::new();
        let win_open = self.clone();
        let win_remove = self.clone();
        dialog.set_callbacks(
            Box::new(move |repo, hash, path| {
                win_open.open_favorite(repo, hash, path);
            }),
            Box::new(move |raw_id| {
                if let Some(settings) = win_remove.imp().settings.get() {
                    let mut favorites: Vec<String> = settings
                        .strv("favorite-snapshot-items")
                        .iter()
                        .map(|item| item.to_string())
                        .collect();
                    favorites.retain(|item| item != &raw_id);
                    settings
                        .set_value("favorite-snapshot-items", &favorites.to_variant())
                        .ok();
                }
                win_remove.update_star_button();
                win_remove.show_toast(&gettext("Removed from favorites"));
            }),
        );
        dialog.set_entries(entries);
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
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
