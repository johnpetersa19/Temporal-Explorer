/* files.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Snapshot item operations: open, export, copy, preview, properties and launch.

use super::*;
use super::util::*;

enum SnapshotWriteMessage {
    Progress { fraction: f64, status: String },
    Done(Result<PathBuf, String>),
}

impl TemporalExplorerWindow {
    pub(super) fn show_current_folder_properties(&self) {
        if self.imp().current_hash.borrow().is_none() {
            self.show_toast(&gettext("No snapshot selected"));
            return;
        }

        let current_dir = self.imp().current_dir.borrow().clone();
        self.show_node_properties(&TreeNode::Dir(current_dir));
    }

    pub(super) fn open_snapshot_node_in_new_tab(&self, node: &TreeNode) {
        if node.is_dir() || node.is_submodule() {
            let path = node.path().to_path_buf();
            self.new_tab();
            self.push_dir(path);
        } else {
            self.open_snapshot_node_with_default_app(node);
        }
    }

    pub(super) fn open_snapshot_node_in_new_window(&self, node: &TreeNode) {
        if !(node.is_dir() || node.is_submodule()) {
            self.open_snapshot_node_with_default_app(node);
            return;
        }

        let Some(application) = self.application() else {
            self.show_error(&gettext("Could not open a new window"));
            return;
        };
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };
        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_toast(&gettext("No snapshot selected"));
            return;
        };

        let target_dir = node.path().to_path_buf();
        let new_window = TemporalExplorerWindow::new(&application);
        new_window.present();
        new_window.load_repository(repo_path);
        new_window.open_snapshot_dir_when_ready(hash, target_dir);
    }

    pub(super) fn open_snapshot_dir_when_ready(&self, hash: String, dir: PathBuf) {
        const MAX_ATTEMPTS: u32 = 240;

        let win = self.clone();
        let mut attempts = 0u32;
        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            attempts += 1;

            if win.imp().commit_index.borrow().contains_key(&hash) {
                win.jump_to_commit_hash(hash.clone());
                win.navigate_to_dir(dir.clone());
                return glib::ControlFlow::Break;
            }

            if attempts >= MAX_ATTEMPTS || !win.imp().loading_commits.get() {
                win.show_error(&gettext("Could not open the snapshot in a new window"));
                return glib::ControlFlow::Break;
            }

            glib::ControlFlow::Continue
        });
    }

    pub(super) fn write_snapshot_node_to_path_with_progress<F>(
        &self,
        node: &TreeNode,
        target: PathBuf,
        title: &str,
        initial_status: &str,
        on_done: F,
    ) where
        F: FnOnce(&Self, PathBuf) + 'static,
    {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_error(&gettext("No repository loaded"));
            return;
        };

        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_error(&gettext("No snapshot selected"));
            return;
        };
        let current_oid = self.imp().current_oid.get();

        let node_path = node.path().to_path_buf();
        let cancel = Arc::new(AtomicBool::new(false));

        if !self.begin_operation(&format!("{}: {}", title, initial_status), cancel.clone()) {
            return;
        }

        let (tx, rx) = std::sync::mpsc::sync_channel::<SnapshotWriteMessage>(16);
        let worker_cancel = cancel.clone();

        std::thread::spawn(move || {
            let result = (|| -> Result<PathBuf, String> {
                let repo = git2::Repository::open(&repo_path)
                    .map_err(|e| format!("Cannot open repository: {e}"))?;

                let commit = match current_oid {
                    Some(oid) => repo
                        .find_commit(oid)
                        .map_err(|e| format!("Cannot read snapshot: {e}"))?,
                    None => repo
                        .revparse_single(&hash)
                        .map_err(|e| format!("Cannot read snapshot: {e}"))?
                        .peel_to_commit()
                        .map_err(|e| format!("Cannot peel commit: {e}"))?,
                };

                let tree = commit
                    .tree()
                    .map_err(|e| format!("Cannot read tree: {e}"))?;

                let entry = tree
                    .get_path(&node_path)
                    .map_err(|e| format!("Cannot find item in snapshot: {e}"))?;

                fn is_safe_entry_name(name: &str) -> bool {
                    !name.is_empty()
                        && name != "."
                        && name != ".."
                        && !name.contains('/')
                        && !name.contains('\\')
                        && !name.contains('\0')
                }

                fn refuse_symlink(path: &std::path::Path) -> Result<(), String> {
                    if let Ok(meta) = std::fs::symlink_metadata(path) {
                        if meta.file_type().is_symlink() {
                            return Err(format!(
                                "Refusing to write through symbolic link: {}",
                                path.display()
                            ));
                        }
                    }
                    Ok(())
                }

                fn materialize_tree(
                    repo: &git2::Repository,
                    tree: &git2::Tree<'_>,
                    destination: &std::path::Path,
                    cancel: &AtomicBool,
                    depth: usize,
                ) -> Result<(), String> {
                    const MAX_MATERIALIZE_DEPTH: usize = 64;
                    if depth > MAX_MATERIALIZE_DEPTH {
                        return Err("Snapshot directory nesting exceeds safety limit".to_string());
                    }
                    refuse_symlink(destination)?;
                    std::fs::create_dir_all(destination)
                        .map_err(|e| format!("Cannot create destination directory: {e}"))?;

                    for child in tree.iter() {
                        if cancel.load(Ordering::Relaxed) {
                            return Err("Operation cancelled".to_string());
                        }
                        let Some(name) = child.name() else { continue };
                        if !is_safe_entry_name(name) {
                            return Err(format!("Unsafe path component in Git tree: {name}"));
                        }
                        let child_target = destination.join(name);
                        refuse_symlink(&child_target)?;
                        match child.kind() {
                            Some(git2::ObjectType::Tree) => {
                                let child_tree = repo
                                    .find_tree(child.id())
                                    .map_err(|e| format!("Cannot read snapshot directory: {e}"))?;
                                materialize_tree(
                                    repo,
                                    &child_tree,
                                    &child_target,
                                    cancel,
                                    depth + 1,
                                )?;
                            }
                            Some(git2::ObjectType::Blob) => {
                                let blob = repo
                                    .find_blob(child.id())
                                    .map_err(|e| format!("Cannot read snapshot file: {e}"))?;
                                if let Some(parent) = child_target.parent() {
                                    std::fs::create_dir_all(parent).map_err(|e| {
                                        format!("Cannot create destination directory: {e}")
                                    })?;
                                }
                                std::fs::write(&child_target, blob.content())
                                    .map_err(|e| format!("Cannot write snapshot file: {e}"))?;
                            }
                            // A gitlink has no content in the containing repository,
                            // but exposing it as a folder is useful to the file manager.
                            Some(git2::ObjectType::Commit) => {
                                std::fs::create_dir_all(&child_target).map_err(|e| {
                                    format!("Cannot create submodule directory: {e}")
                                })?;
                            }
                            _ => {}
                        }
                    }
                    Ok(())
                }

                match entry.kind() {
                    Some(git2::ObjectType::Tree) => {
                        let selected_tree = repo
                            .find_tree(entry.id())
                            .map_err(|e| format!("Cannot read snapshot directory: {e}"))?;
                        materialize_tree(&repo, &selected_tree, &target, &worker_cancel, 0)?;
                    }
                    Some(git2::ObjectType::Blob) => {
                        let blob = repo
                            .find_blob(entry.id())
                            .map_err(|e| format!("Cannot read snapshot file: {e}"))?;
                        if let Some(parent) = target.parent() {
                            std::fs::create_dir_all(parent)
                                .map_err(|e| format!("Cannot create destination directory: {e}"))?;
                        }
                        let content = blob.content();
                        let total = content.len().max(1);
                        let mut file = std::fs::File::create(&target)
                            .map_err(|e| format!("Cannot create destination file: {e}"))?;
                        for (index, chunk) in content.chunks(64 * 1024).enumerate() {
                            if worker_cancel.load(Ordering::Relaxed) {
                                return Err("Operation cancelled".to_string());
                            }
                            file.write_all(chunk)
                                .map_err(|e| format!("Cannot write destination file: {e}"))?;
                            let written = ((index + 1) * 64 * 1024).min(content.len());
                            let _ = tx.send(SnapshotWriteMessage::Progress {
                                fraction: written as f64 / total as f64,
                                status: format!("Writing {} / {} bytes", written, total),
                            });
                        }
                        file.flush()
                            .map_err(|e| format!("Cannot flush destination file: {e}"))?;
                    }
                    Some(git2::ObjectType::Commit) => {
                        std::fs::create_dir_all(&target)
                            .map_err(|e| format!("Cannot create submodule directory: {e}"))?;
                    }
                    _ => return Err(gettext("Unsupported snapshot item type")),
                }

                Ok(target)
            })();

            let _ = tx.send(SnapshotWriteMessage::Done(result));
        });

        let win = self.clone();
        let callback = std::rc::Rc::new(std::cell::RefCell::new(Some(on_done)));
        let callback_ref = callback.clone();

        glib::idle_add_local(move || loop {
            match rx.try_recv() {
                Ok(SnapshotWriteMessage::Progress { fraction, status }) => {
                    win.update_operation(fraction, &status);
                }
                Ok(SnapshotWriteMessage::Done(result)) => {
                    match result {
                        Ok(path) => {
                            win.complete_operation(
                                &format!("{}: {}", gettext("Completed"), path.display()),
                                true,
                            );
                            if let Some(callback) = callback_ref.borrow_mut().take() {
                                callback(&win, path);
                            }
                        }
                        Err(msg) => {
                            if msg != "Operation cancelled" {
                                win.complete_operation(&msg, false);
                                win.show_error(&msg);
                            } else {
                                win.complete_operation(&gettext("Operation cancelled"), false);
                                win.show_toast(&gettext("Operation cancelled"));
                            }
                        }
                    }

                    return glib::ControlFlow::Break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    return glib::ControlFlow::Continue;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    win.complete_operation(
                        &gettext("Snapshot operation stopped unexpectedly"),
                        false,
                    );
                    return glib::ControlFlow::Break;
                }
            }
        });
    }

    pub(super) fn temp_target_for_snapshot_node(&self, node: &TreeNode) -> Option<PathBuf> {
        let hash = self.imp().current_hash.borrow().clone()?;
        // Prefer user-private directories (0700) over shared /tmp to avoid
        // predictable-path symlink attacks (CWE-377) on multi-user systems.
        let cache_root = if Path::new("/.flatpak-info").exists() {
            glib::home_dir().join(".cache").join("temporal-explorer")
        } else if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
            PathBuf::from(runtime_dir).join("temporal-explorer")
        } else {
            glib::user_cache_dir().join("temporal-explorer").join("preview")
        };
        Some(cache_root.join(short_hash(&hash)).join(node.path()))
    }

    #[allow(dead_code)]
    pub(super) fn materialize_snapshot_node_to_temp(&self, node: &TreeNode) -> Option<PathBuf> {
        if node.is_dir() || node.is_submodule() {
            self.show_toast(&gettext(
                "Only files can be opened or exported from a snapshot",
            ));
            return None;
        }

        let hash = self.imp().current_hash.borrow().clone()?;
        let current_oid = self.imp().current_oid.get();
        let repo_ref = self.imp().repository.borrow();
        let repo = &repo_ref.as_ref()?.0;

        let commit = match current_oid {
            Some(oid) => repo.find_commit(oid).ok()?,
            None => repo.revparse_single(&hash).ok()?.peel_to_commit().ok()?,
        };
        let tree = commit.tree().ok()?;
        let entry = tree.get_path(node.path()).ok()?;
        let blob = repo.find_blob(entry.id()).ok()?;

        let file_name = node
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("snapshot-file");

        let short = short_hash(&hash);
        let target_dir = std::env::temp_dir().join("temporal-explorer").join(short);

        std::fs::create_dir_all(&target_dir).ok()?;

        let target = target_dir.join(file_name);
        std::fs::write(&target, blob.content()).ok()?;

        Some(target)
    }

    pub(super) fn open_snapshot_node_with_default_app(&self, node: &TreeNode) {
        let Some(target) = self.temp_target_for_snapshot_node(node) else {
            return;
        };

        self.write_snapshot_node_to_path_with_progress(
            node,
            target,
            &gettext("Opening File"),
            &gettext("Materializing snapshot file…"),
            |win, path| {
                if Path::new("/.flatpak-info").exists() {
                    if let Err(err) = Self::launch_path_with_host_default(&path) {
                        win.show_error(&format!(
                            "{}: {err}",
                            gettext("Could not open the item with the default application")
                        ));
                    }
                } else if let Err(err) = Self::launch_path_with_default_app(&path) {
                    win.show_error(&format!(
                        "{}\n\n{}: {}\n{}: {err}",
                        gettext("Could not open the item with the default application."),
                        gettext("Temporary item"),
                        path.display(),
                        gettext("System error")
                    ));
                }
            },
        );
    }

    pub(super) fn launch_path_through_portal(&self, path: PathBuf, always_ask: bool) {
        let file = gio::File::for_path(&path);
        let launcher = gtk::FileLauncher::new(Some(&file));
        launcher.set_always_ask(always_ask);
        let win = self.clone();
        launcher.launch(Some(self), None::<&gio::Cancellable>, move |result| {
            if let Err(error) = result {
                let dismissed = error.message().to_ascii_lowercase().contains("dismissed");
                if !error.matches(gio::IOErrorEnum::Cancelled) && !dismissed {
                    let action = if always_ask {
                        gettext("Could not open the item with the selected application")
                    } else {
                        gettext("Could not open the item with the default application")
                    };
                    win.show_error(&format!("{action}: {error}"));
                }
            }
        });
    }

    pub(super) fn launch_path_with_host_default(path: &Path) -> Result<(), String> {
        Command::new("flatpak-spawn")
            .arg("--host")
            .arg("gio")
            .arg("open")
            .arg("--")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub(super) fn launch_path_with_default_app(path: &Path) -> Result<(), String> {
        let file = gio::File::for_path(path);
        let info = file
            .query_info(
                gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
                gio::FileQueryInfoFlags::NONE,
                None::<&gio::Cancellable>,
            )
            .map_err(|error| error.to_string())?;
        let content_type = info
            .content_type()
            .ok_or_else(|| gettext("The system could not determine the item type"))?;
        if let Some(app) = gio::AppInfo::default_for_type(&content_type, false)
            .or_else(|| {
                gio::AppInfo::recommended_for_type(&content_type)
                    .into_iter()
                    .next()
            })
            .or_else(|| gio::AppInfo::all_for_type(&content_type).into_iter().next())
        {
            if app.launch(&[file], None::<&gio::AppLaunchContext>).is_ok() {
                return Ok(());
            }
        }

        // Some desktop environments expose associations to `gio open` while
        // g_app_info_get_default_for_type() returns NULL to the application.
        // Delegate to the system dispatcher in that case.
        Command::new("gio")
            .arg("open")
            .arg("--")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| {
                format!(
                    "{}: {} ({error})",
                    gettext("No application is available for this item type"),
                    content_type
                )
            })
    }

    #[allow(deprecated)] // GtkAppChooserDialog is the reliable non-portal fallback on GTK 4.
    pub(super) fn open_snapshot_node_with_app_chooser(&self, node: &TreeNode) {
        let Some(target) = self.temp_target_for_snapshot_node(node) else {
            return;
        };

        self.write_snapshot_node_to_path_with_progress(
            node,
            target,
            &gettext("Preparing File"),
            &gettext("Materializing snapshot file…"),
            |win, path| {
                if Path::new("/.flatpak-info").exists() {
                    win.launch_path_through_portal(path, true);
                } else {
                    win.show_native_app_chooser(path);
                }
            },
        );
    }

    pub(super) fn show_native_app_chooser(&self, path: PathBuf) {
        let file = gio::File::for_path(&path);
        let content_type = file
            .query_info(
                gio::FILE_ATTRIBUTE_STANDARD_CONTENT_TYPE,
                gio::FileQueryInfoFlags::NONE,
                None::<&gio::Cancellable>,
            )
            .ok()
            .and_then(|info| info.content_type())
            .map(|s| s.to_string());

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();

        let dialog = crate::app_chooser_dialog::AppChooserDialog::new();
        let win = self.clone();
        let launch_path = path.clone();
        dialog.configure(
            &file_name,
            content_type.as_deref(),
            Box::new(move |app| {
                let file = gio::File::for_path(&launch_path);
                if let Err(err) = app.launch(&[file], None::<&gio::AppLaunchContext>) {
                    win.show_error(&format!(
                        "{}: {err}",
                        gettext("Could not open the item with the selected application")
                    ));
                }
            }),
        );
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    pub(super) fn show_export_snapshot_archive_dialog(&self) {
        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_toast(&gettext("No snapshot selected"));
            return;
        };
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };

        let repo_name = self.imp().repo_name.borrow().clone();
        let current_dir = self.imp().current_dir.borrow().clone();
        let default_name = format!(
            "{}-{}",
            if repo_name.is_empty() {
                "snapshot"
            } else {
                repo_name.as_str()
            },
            short_hash(&hash)
        );

        let dialog = crate::compress_dialog::CompressDialog::new();
        let win = self.clone();
        dialog.configure(
            &default_name,
            Box::new(move |archive_name, format| {
                let file_name = if archive_name.ends_with(format.extension()) {
                    archive_name
                } else {
                    format!("{archive_name}{}", format.extension())
                };

                let chooser = gtk::FileDialog::builder()
                    .title(gettext("Save Snapshot Archive"))
                    .initial_name(&file_name)
                    .build();

                let win_save = win.clone();
                let repo_for_worker = repo_path.clone();
                let hash_for_worker = hash.clone();
                let dir_for_worker = current_dir.clone();
                chooser.save(Some(&win), gio::Cancellable::NONE, move |res| {
                    let Ok(dest_file) = res else { return };
                    let Some(dest_path) = dest_file.path() else {
                        return;
                    };

                    let cancel = Arc::new(AtomicBool::new(false));
                    win_save.begin_operation(
                        &format!("{}: {}", gettext("Exporting Archive"), dest_path.display()),
                        cancel,
                    );

                    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<PathBuf, String>>(1);
                    let dest_worker = dest_path.clone();
                    std::thread::spawn(move || {
                        if git2::Oid::from_str(&hash_for_worker).is_err() {
                            let _ = tx.send(Err("Invalid commit SHA".to_string()));
                            return;
                        }
                        let mut cmd = Command::new("git");
                        cmd.arg("-C")
                            .arg(&repo_for_worker)
                            .arg("archive")
                            .arg(format!("--format={}", format.git_format()))
                            .arg("--output")
                            .arg(&dest_worker)
                            .arg(&hash_for_worker);
                        if !dir_for_worker.as_os_str().is_empty() {
                            cmd.arg("--").arg(&dir_for_worker);
                        }
                        let result = match cmd.output() {
                            Ok(output) if output.status.success() => Ok(dest_worker),
                            Ok(output) => {
                                let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
                                Err(if err.is_empty() {
                                    "git archive failed".to_string()
                                } else {
                                    err
                                })
                            }
                            Err(e) => Err(e.to_string()),
                        };
                        let _ = tx.send(result);
                    });

                    let win_done = win_save.clone();
                    glib::idle_add_local(move || match rx.try_recv() {
                        Ok(Ok(path)) => {
                            win_done.complete_operation(
                                &format!("{}: {}", gettext("Exported"), path.display()),
                                true,
                            );
                            win_done.show_toast(&format!(
                                "{}: {}",
                                gettext("Archive exported"),
                                path.file_name()
                                    .and_then(|n| n.to_str())
                                    .unwrap_or_default()
                            ));
                            glib::ControlFlow::Break
                        }
                        Ok(Err(msg)) => {
                            win_done.complete_operation(&msg, false);
                            win_done.show_error(&msg);
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    });
                });
            }),
        );
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    pub(super) fn open_snapshot_node_location(&self, node: &TreeNode) {
        let Some(target) = self.temp_target_for_snapshot_node(node) else {
            return;
        };

        self.write_snapshot_node_to_path_with_progress(
            node,
            target,
            &gettext("Preparing Item"),
            &gettext("Materializing snapshot item…"),
            |win, path| {
                let location = if path.is_dir() {
                    path.clone()
                } else {
                    path.parent().map(PathBuf::from).unwrap_or(path)
                };
                if Path::new("/.flatpak-info").exists() {
                    if let Err(err) = Self::launch_path_with_host_default(&location) {
                        win.show_error(&format!(
                            "{}: {err}",
                            gettext("Could not open the item location")
                        ));
                    }
                } else if let Err(err) = Self::launch_path_with_default_app(&location) {
                    win.show_error(&format!(
                        "{}\n\n{}: {}\n{}: {err}",
                        gettext("Could not open the item location."),
                        gettext("Temporary location"),
                        location.display(),
                        gettext("System error")
                    ));
                }
            },
        );
    }

    pub(super) fn export_snapshot_node(&self, node: &TreeNode) {
        if node.is_dir() || node.is_submodule() {
            self.show_toast(&gettext("Only files can be exported from a snapshot"));
            return;
        }

        let file_name = node
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("snapshot-file")
            .to_string();

        let dialog = gtk::FileDialog::builder()
            .title(gettext("Export File"))
            .modal(true)
            .accept_label(gettext("Export"))
            .initial_name(file_name)
            .build();

        let win = self.clone();
        let node = node.clone();

        dialog.save(Some(self), None::<&gio::Cancellable>, move |result| {
            let Ok(file) = result else {
                return;
            };
            let Some(target) = file.path() else {
                return;
            };

            win.write_snapshot_node_to_path_with_progress(
                &node,
                target,
                &gettext("Exporting File"),
                &gettext("Exporting snapshot file…"),
                move |win, path| {
                    win.show_toast(&format!("{}: {}", gettext("Exported"), path.display()));
                },
            );
        });
    }

    pub(super) fn copy_repository_path(&self, node: &TreeNode) {
        let text = node.path().to_string_lossy().to_string();

        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&text);
            self.show_toast(&gettext("Snapshot path copied"));
        }
    }

    pub(super) fn copy_snapshot_node_content(&self, node: &TreeNode) {
        if node.is_dir() || node.is_submodule() {
            self.show_toast(&gettext("Only file content can be copied"));
            return;
        }

        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            return;
        };
        let current_oid = self.imp().current_oid.get();

        let repo_ref = self.imp().repository.borrow();
        let Some(repo_wrapper) = repo_ref.as_ref() else {
            return;
        };
        let repo = &repo_wrapper.0;

        let content = match current_oid {
            Some(oid) => repo.find_commit(oid).ok(),
            None => repo
                .revparse_single(&hash)
                .ok()
                .and_then(|obj| obj.peel_to_commit().ok()),
        }
        .and_then(|commit| commit.tree().ok())
        .and_then(|tree| tree.get_path(node.path()).ok())
        .and_then(|entry| repo.find_blob(entry.id()).ok())
        .and_then(|blob| {
            std::str::from_utf8(blob.content())
                .ok()
                .map(ToOwned::to_owned)
        });

        if let Some(text) = content {
            if let Some(display) = gtk::gdk::Display::default() {
                display.clipboard().set_text(&text);
                self.show_toast(&gettext("File content copied"));
            }
        } else {
            self.show_toast(&gettext(
                "This file is binary or could not be copied as text",
            ));
        }
    }

    pub(super) fn show_node_in_system(&self, node: &TreeNode) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            return;
        };

        let working_path = repo_path.join(node.path());

        if !working_path.exists() {
            self.show_toast(&gettext(
                "This file does not exist in the current working tree",
            ));
            return;
        }

        let file = gio::File::for_path(&working_path);
        let uri = file.uri();

        if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>).is_err() {
            self.show_error(&gettext("Could not show file in the system"));
        }
    }

    pub(super) fn show_node_properties(&self, node: &TreeNode) {
        let hash = self.imp().current_hash.borrow().clone().unwrap_or_default();
        let short = short_hash(&hash).to_string();
        let repo_name = self.imp().repo_name.borrow().clone();

        let snapshot_date = {
            let commits = self.imp().all_commits.borrow();
            let index = self.imp().commit_index.borrow();

            index
                .get(&hash)
                .and_then(|idx| commits.get(*idx))
                .map(|commit| Self::format_timestamp(commit.timestamp))
                .unwrap_or_else(|| gettext("Unknown snapshot date"))
        };

        let name = node
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        let repository_path = node.path().display().to_string();

        let parent_folder = node
            .path()
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.display().to_string())
            .unwrap_or_else(|| {
                self.imp()
                    .repo_path
                    .borrow()
                    .clone()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| gettext("Repository root"))
            });

        let kind = if node.is_dir() {
            gettext("Folder")
        } else if node.is_submodule() {
            gettext("Submodule")
        } else {
            gettext("File")
        };

        let icon_name = if node.is_dir() {
            "folder-symbolic".to_string()
        } else if node.is_submodule() {
            "network-computer-symbolic".to_string()
        } else {
            "text-x-generic-symbolic".to_string()
        };

        let mut git_object = gettext("Not available");
        let mut git_mode = gettext("Not available");
        let mut size = gettext("Not available");

        if let Some(hash) = self.imp().current_hash.borrow().clone() {
            let current_oid = self.imp().current_oid.get();
            let repo_ref = self.imp().repository.borrow();

            if let Some(repo_wrapper) = repo_ref.as_ref() {
                let repo = &repo_wrapper.0;

                let commit = match current_oid {
                    Some(oid) => repo.find_commit(oid).ok(),
                    None => repo
                        .revparse_single(&hash)
                        .ok()
                        .and_then(|obj| obj.peel_to_commit().ok()),
                };

                if let Some((object_id, object_mode, object_size)) = commit
                    .and_then(|commit| commit.tree().ok())
                    .and_then(|tree| tree.get_path(node.path()).ok())
                    .and_then(|entry| {
                        let oid = entry.id();
                        let mode = entry.filemode();

                        if matches!(node, TreeNode::File(_)) {
                            repo.find_blob(oid)
                                .ok()
                                .map(|blob| (oid.to_string(), mode, Some(blob.size() as u64)))
                        } else {
                            Some((oid.to_string(), mode, None))
                        }
                    })
                {
                    git_object = object_id;
                    git_mode = format_git_mode(object_mode);

                    if let Some(object_size) = object_size {
                        size = format_file_size(object_size);
                    }
                }
            }
        }

        let working_tree_status = self
            .imp()
            .repo_path
            .borrow()
            .clone()
            .map(|repo_path| repo_path.join(node.path()).exists())
            .unwrap_or(false);

        let system_status = if working_tree_status {
            gettext("Exists in working tree")
        } else {
            gettext("Only available in selected snapshot")
        };

        let repository_root = self.imp().repo_path.borrow().clone().unwrap_or_default();
        let favorite_id = format!(
            "{}\n{}\n{}",
            repository_root.display(),
            hash,
            node.path().display()
        );
        let favorite = self
            .imp()
            .settings
            .get()
            .map(|settings| {
                settings
                    .strv("favorite-snapshot-items")
                    .iter()
                    .any(|item| item.as_str() == favorite_id)
            })
            .unwrap_or(false);

        let props = NodeProperties {
            name,
            kind,
            icon_name,
            repository: repo_name,
            repository_path,
            parent_folder,
            snapshot_commit: short,
            full_commit: hash,
            snapshot_date,
            git_object,
            git_mode,
            size,
            system_status,
            favorite,
        };

        let dialog = NodePropertiesDialog::new();
        dialog.set_properties(&props);

        let win = self.clone();
        let favorite_id_for_signal = favorite_id.clone();
        dialog.connect_favorite_toggled(move |_, active| {
            if let Some(settings) = win.imp().settings.get() {
                let mut favorites = settings
                    .strv("favorite-snapshot-items")
                    .iter()
                    .map(|item| item.to_string())
                    .collect::<Vec<_>>();
                favorites.retain(|item| item != &favorite_id_for_signal);
                if active {
                    favorites.push(favorite_id_for_signal.clone());
                }
                settings
                    .set_value("favorite-snapshot-items", &favorites.to_variant())
                    .ok();
            }
            let msg = if active {
                gettext("Marked as favorite")
            } else {
                gettext("Removed from favorites")
            };
            win.show_toast(&msg);
        });

        let win = self.clone();
        let node_for_location = node.clone();
        dialog.connect_open_location_requested(move |_, _| {
            win.show_node_in_system(&node_for_location);
        });

        let win = self.clone();
        dialog.connect_icon_edit_requested(move |_| {
            win.show_toast(&gettext("Snapshot item icons cannot be edited"));
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }
}
