/* navigation.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Directory and commit navigation, location entry, path bar and history controls.

use super::*;
use super::util::*;

impl TemporalExplorerWindow {
    pub(super) fn edit_location(&self) {
        self.enter_location_mode();
    }

    pub(super) fn prompt_root_location(&self) {
        self.imp().toolbar.set_location_mode(true);
        self.imp().toolbar.location_entry().set_text("/");
        self.imp().toolbar.location_entry().grab_focus();
        self.imp()
            .toolbar
            .location_entry()
            .set_position(self.imp().toolbar.location_entry().text_length() as i32);
    }

    pub(super) fn copy_current_location(&self) {
        if self.imp().current_hash.borrow().is_none() {
            self.show_toast(&gettext("No snapshot selected"));
            return;
        }

        let current_dir = self.imp().current_dir.borrow().clone();
        self.copy_snapshot_path_from_pathbar(&current_dir);
    }

    pub(super) fn copy_current_commit_sha(&self) {
        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_toast(&gettext("No commit selected"));
            return;
        };

        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&hash);
            self.show_toast(&gettext("Commit SHA copied"));
        }
    }

    pub(super) fn show_current_commit_details(&self) {
        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_toast(&gettext("No commit selected"));
            return;
        };

        let repo_ref = self.imp().repository.borrow();
        let Some(repo_wrapper) = repo_ref.as_ref() else {
            self.show_error(&gettext("No repository loaded"));
            return;
        };

        let commit = match git2::Oid::from_str(&hash)
            .ok()
            .and_then(|oid| repo_wrapper.0.find_commit(oid).ok())
        {
            Some(commit) => commit,
            None => {
                self.show_error(&gettext("Could not load commit details"));
                return;
            }
        };

        let parents = if commit.parent_count() == 0 {
            gettext("Root commit")
        } else {
            commit
                .parent_ids()
                .map(|oid| short_hash(&oid.to_string()).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };

        let details = CommitDetails {
            summary: commit.summary().unwrap_or("").to_string(),
            sha: commit.id().to_string(),
            parents,
            author: commit.author().name().unwrap_or("").to_string(),
            email: commit.author().email().unwrap_or("").to_string(),
            date: Self::format_timestamp(commit.time().seconds()),
            message: commit.message().unwrap_or("").trim().to_string(),
        };

        drop(commit);
        drop(repo_ref);

        let dialog = CommitDetailsDialog::new();
        dialog.set_details(&details);
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    pub(super) fn preview_selected_file_from_action(&self) {
        match self.selected_file_view_node() {
            Some(TreeNode::File(path)) => self.preview_file(&path),
            Some(_) => self.show_toast(&gettext("Selected item is not a file")),
            None => self.show_toast(&gettext("No file selected")),
        }
    }

    pub(super) fn reload_current_dir_if_possible(&self) {
        if self.imp().current_hash.borrow().is_some() {
            let dir = self.imp().current_dir.borrow().clone();
            self.navigate_to_dir(dir);
        }
    }

    pub(super) fn setup_history_controls(&self) {
        let win = self.clone();
        self.imp()
            .toolbar
            .history_controls()
            .connect_local("navigate-back", false, move |_| {
                win.navigate_back();
                None
            });

        let win = self.clone();
        self.imp()
            .toolbar
            .history_controls()
            .connect_local("navigate-forward", false, move |_| {
                win.navigate_forward();
                None
            });
    }

    /// Push `hash` onto the commit navigation back-stack and clear the forward stack.
    ///
    /// # Borrow safety
    ///
    /// `current_hash` is read into an owned `Option<String>` before
    /// `commit_nav_forward` is mutably borrowed, so the two RefCells are
    /// never borrowed simultaneously.
    pub(super) fn push_commit_nav(&self, hash: &str) {
        let imp = self.imp();

        // Read current_hash into an owned value; guard is dropped immediately.
        let prev_hash = imp.current_hash.borrow().clone();

        imp.commit_nav_forward.borrow_mut().clear();
        if let Some(prev) = prev_hash {
            if prev != hash {
                imp.commit_nav_back.borrow_mut().push(prev);
            }
        }
        self.update_commit_nav_buttons();
    }

    /// Navigate back one step in the commit history.
    ///
    /// # Borrow safety
    ///
    /// All RefCell values are extracted into owned locals inside a tightly-scoped
    /// block.  That block ends — and every borrow guard is dropped — **before**
    /// `jump_to_commit_hash` is called.  `jump_to_commit_hash` mutably borrows
    /// `current_hash` and `current_dir` internally; any live `imp` binding at the
    /// call-site would alias those borrows and cause a fatal RefCell panic inside
    /// the non-unwinding glib closure marshaller (SIGABRT).
    #[allow(dead_code)]
    pub(super) fn navigate_commit_back(&self) {
        // ── Extract all needed values; every borrow guard drops at end of block ──
        let (prev_hash, current) = {
            let imp = self.imp();
            let prev = imp.commit_nav_back.borrow_mut().pop();
            let cur = imp.current_hash.borrow().clone();
            (prev, cur)
            // imp, commit_nav_back borrow_mut, and current_hash borrow all dropped here
        };

        if let Some(prev) = prev_hash {
            if let Some(cur) = current {
                // No RefCell is borrowed at this point.
                self.imp().commit_nav_forward.borrow_mut().push(cur);
            }
            // All borrows are released here before jump_to_commit_hash writes
            // to current_hash, current_dir, history_back, and history_forward.
            self.jump_to_commit_hash(prev);
        }
    }

    /// Navigate forward one step in the commit history.
    ///
    /// # Borrow safety
    ///
    /// Mirrors `navigate_commit_back`: all RefCell values are extracted into
    /// owned locals before `jump_to_commit_hash` is called, avoiding any
    /// simultaneous borrow_mut aliasing.
    #[allow(dead_code)]
    pub(super) fn navigate_commit_forward(&self) {
        // ── Extract all needed values; every borrow guard drops at end of block ──
        let (next_hash, current) = {
            let imp = self.imp();
            let next = imp.commit_nav_forward.borrow_mut().pop();
            let cur = imp.current_hash.borrow().clone();
            (next, cur)
            // imp, commit_nav_forward borrow_mut, and current_hash borrow all dropped here
        };

        if let Some(next) = next_hash {
            if let Some(cur) = current {
                // No RefCell is borrowed at this point.
                self.imp().commit_nav_back.borrow_mut().push(cur);
            }
            // All borrows released before jump_to_commit_hash runs.
            self.jump_to_commit_hash(next);
        }
    }

    /// Navigate directly to `hash`, resetting dir history and updating the info bar.
    pub(super) fn jump_to_commit_hash(&self, hash: String) {
        let imp = self.imp();
        *imp.current_hash.borrow_mut() = Some(hash.clone());
        imp.current_oid.set(None);
        *imp.current_dir.borrow_mut() = PathBuf::new();
        imp.history_back.borrow_mut().clear();
        imp.history_forward.borrow_mut().clear();

        {
            let commits = imp.all_commits.borrow();
            let index = imp.commit_index.borrow();

            let commit = index
                .get(&hash)
                .and_then(|idx| commits.get(*idx))
                .or_else(|| {
                    commits
                        .iter()
                        .find(|c| c.hash.starts_with(short_hash(&hash)))
                });

            if let Some(commit) = commit {
                imp.current_oid.set(Some(commit.oid()));
                imp.commit_hash_label.set_label(display_hash(&commit.hash));
                imp.commit_message_label.set_label(&commit.summary);
                imp.commit_date_label
                    .set_label(&Self::format_timestamp(commit.timestamp));
                imp.commit_info_bar.set_revealed(true);
            }
        }
        self.update_commit_nav_buttons();
        self.navigate_to_dir(PathBuf::new());
    }

    pub(super) fn update_commit_nav_buttons(&self) {
        self.update_navigation_buttons();
    }

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
        let repo_name = imp.repo_name.borrow().clone();
        let current_oid = imp.current_oid.get();

        *imp.current_dir.borrow_mut() = dir.clone();

        let win_ab1 = self.clone();
        let win_ab2 = self.clone();
        let win_ab3 = self.clone();
        let win_ab4 = self.clone();
        address_bar::rebuild_address_bar_with_context(
            imp.toolbar.address_bar(),
            &repo_name,
            &dir,
            move |path: PathBuf| {
                win_ab1.push_dir(path);
            },
            move || {
                win_ab2.enter_location_mode();
            },
            move |path: PathBuf| {
                win_ab3.copy_snapshot_path_from_pathbar(&path);
            },
            move |path: PathBuf| {
                win_ab4.show_pathbar_properties(&path);
            },
        );
        let address_bar_scrolled = imp.toolbar.address_bar_scrolled().clone();
        glib::idle_add_local_once(move || {
            let adjustment = address_bar_scrolled.hadjustment();
            let max = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
            adjustment.set_value(max);
        });
        self.update_dir_nav_buttons();
        self.save_active_tab_state();

        // Cancellation token for directory loads.
        //
        // Each navigation invalidates the previous token before starting a new
        // worker.  The token is shared with the UI polling closure through Arc so
        // stale workers cannot update the right panel after the user has already
        // moved to another directory.
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(prev) = imp.load_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }
        *imp.load_cancel.borrow_mut() = Some(cancel.clone());

        let dir_clone = dir.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<TreeNode>, String>>(1);
        std::thread::spawn(move || {
            let result = git2::Repository::open(&repo_path)
                .map_err(|e| e.to_string())
                .and_then(|repo| {
                    let resolver = SnapshotResolver::new(&repo);
                    match current_oid {
                        Some(oid) => resolver
                            .resolve_dir_oid(oid, &dir_clone)
                            .map_err(|e| e.to_string()),
                        None => resolver
                            .resolve_dir(&hash, &dir_clone)
                            .map_err(|e| e.to_string()),
                    }
                });
            let _ = tx.send(result);
        });

        let win = self.clone();
        glib::idle_add_local(move || match rx.try_recv() {
            Ok(result) => {
                match result {
                    Ok(nodes) => win.render_dir(nodes),
                    Err(e) => win.show_error(&format!("{}: {e}", gettext("Error reading tree"))),
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(_) => glib::ControlFlow::Break,
        });
    }

    pub fn preview_file(&self, path: &std::path::Path) {
        let hash = match self.imp().current_hash.borrow().clone() {
            Some(h) => h,
            None => return,
        };
        let oid = self.imp().current_oid.get();
        let repo_path = match self.imp().repo_path.borrow().clone() {
            Some(p) => p,
            None => return,
        };
        let file_path = path.to_path_buf();

        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(String, String), String>>(1);
        let worker_path = file_path.clone();
        let worker_hash = hash.clone();
        let worker_oid = oid;

        std::thread::spawn(move || {
            let result = git2::Repository::open(&repo_path)
                .map_err(|e| e.to_string())
                .map(|repo| match worker_oid {
                    Some(oid) => file_preview::read_file_preview_oid(&repo, oid, &worker_path),
                    None => file_preview::read_file_preview(&repo, &worker_hash, &worker_path),
                });

            let _ = tx.send(result);
        });

        let win = self.clone();
        glib::idle_add_local(move || match rx.try_recv() {
            Ok(Ok((title, body))) => {
                file_preview::show_file_preview_text(&win, &title, &body, &hash, &file_path);
                glib::ControlFlow::Break
            }
            Ok(Err(e)) => {
                win.show_error(&format!("{}: {e}", gettext("Cannot open repository")));
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        });
    }

    pub fn push_dir(&self, dir: PathBuf) {
        let imp = self.imp();
        let prev = imp.current_dir.borrow().clone();
        imp.history_back.borrow_mut().push(prev);
        imp.history_forward.borrow_mut().clear();
        self.navigate_to_dir(dir);
    }

    /// Navigate back one step in the directory history of the current snapshot.
    ///
    /// # Borrow safety
    ///
    /// All RefCell guards (`history_back`, `current_dir`, `history_forward`) are
    /// resolved into owned values inside a tightly-scoped block.  That block ends
    /// — and every guard is dropped — **before** `navigate_to_dir` is called.
    /// `navigate_to_dir` itself borrows `current_dir` mutably, so any live guard
    /// on it at call-time would cause a "RefCell already mutably borrowed" panic
    /// inside the non-unwinding glib closure marshaller (fatal SIGABRT).
    pub(super) fn navigate_back(&self) {
        // ── Extract all needed values; every borrow guard drops at end of block ──
        let (dir_to_go, cur_dir) = {
            let imp = self.imp();
            let dir_to_go = imp.history_back.borrow_mut().pop();
            let cur_dir = imp.current_dir.borrow().clone();
            (dir_to_go, cur_dir)
            // imp, history_back borrow_mut, and current_dir borrow are all dropped here
        };

        if let Some(dir) = dir_to_go {
            self.imp().history_forward.borrow_mut().push(cur_dir);
            self.navigate_to_dir(dir);
        }
    }

    /// Navigate forward one step in the directory history of the current snapshot.
    ///
    /// # Borrow safety
    ///
    /// Mirrors `navigate_back`: all guards are resolved into owned locals before
    /// `navigate_to_dir` is called, preventing any live borrow aliasing.
    pub(super) fn navigate_forward(&self) {
        // ── Extract all needed values; every borrow guard drops at end of block ──
        let (dir_to_go, cur_dir) = {
            let imp = self.imp();
            let dir_to_go = imp.history_forward.borrow_mut().pop();
            let cur_dir = imp.current_dir.borrow().clone();
            (dir_to_go, cur_dir)
            // imp, history_forward borrow_mut, and current_dir borrow are all dropped here
        };

        if let Some(dir) = dir_to_go {
            self.imp().history_back.borrow_mut().push(cur_dir);
            self.navigate_to_dir(dir);
        }
    }

    pub(super) fn update_dir_nav_buttons(&self) {
        self.update_navigation_buttons();
    }

    pub(super) fn update_navigation_buttons(&self) {
        let imp = self.imp();
        let can_back = !imp.history_back.borrow().is_empty();
        let can_forward = !imp.history_forward.borrow().is_empty();
        imp.toolbar
            .history_controls()
            .set_sensitivity(can_back, can_forward);
    }

    pub(super) fn enter_location_mode(&self) {
        let imp = self.imp();
        let path_str = imp.current_dir.borrow().to_string_lossy().to_string();
        imp.toolbar.location_entry().set_text(&path_str);
        imp.toolbar.set_location_mode(true);
        imp.toolbar.location_entry().grab_focus();
    }

    pub(super) fn leave_location_mode(&self) {
        self.imp().toolbar.set_location_mode(false);
    }

    pub(super) fn navigate_to_typed_path(&self, text: &str) {
        self.imp().toolbar.set_location_mode(false);
        match Self::normalize_typed_snapshot_path(text) {
            Ok(path) => self.push_dir(path),
            Err(message) => {
                self.show_toast(&message);
                self.enter_location_mode();
            }
        }
    }

    pub(super) fn normalize_typed_snapshot_path(text: &str) -> Result<PathBuf, String> {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed == "/" || trimmed == "." {
            return Ok(PathBuf::new());
        }

        let mut normalized = PathBuf::new();
        let candidate = PathBuf::from(trimmed.trim_start_matches('/'));

        for component in candidate.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(gettext("Parent paths are not available in snapshots"));
                }
                Component::RootDir | Component::Prefix(_) => {}
            }
        }

        Ok(normalized)
    }

    pub(super) fn copy_snapshot_path_from_pathbar(&self, path: &std::path::Path) {
        let text = if path.as_os_str().is_empty() {
            self.imp()
                .repo_path
                .borrow()
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| self.imp().repo_name.borrow().clone())
        } else {
            path.to_string_lossy().to_string()
        };

        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&text);
            self.show_toast(&gettext("Path copied"));
        }
    }

    pub(super) fn show_pathbar_properties(&self, path: &std::path::Path) {
        self.show_node_properties(&TreeNode::Dir(path.to_path_buf()));
    }
}
