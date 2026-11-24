/* operations.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Repository lifecycle, background operations, branches, clone, batch and merge flows.

use super::*;
use super::util::*;
use super::git_ops::*;

impl TemporalExplorerWindow {
    pub(super) fn begin_operation(&self, status: &str, cancel: Arc<AtomicBool>) -> bool {
        let imp = self.imp();
        if imp.operation_cancel.borrow().is_some() {
            self.show_toast(&gettext("An operation is already in progress"));
            return false;
        }
        *imp.operation_cancel.borrow_mut() = Some(cancel);
        imp.operation_status_label.set_label(status);
        imp.operation_progress_bar.set_fraction(0.0);
        imp.operation_progress_bar.set_visible(true);
        imp.operation_cancel_button.set_sensitive(true);
        imp.progress_indicator_icon
            .set_icon_name(Some("process-working-symbolic"));
        imp.sidebar_bottom_bar.set_revealed(true);
        self.add_operation_row(status);
        true
    }

    pub(super) fn update_operation(&self, fraction: f64, status: &str) {
        let imp = self.imp();
        imp.operation_status_label.set_label(status);
        imp.operation_progress_bar
            .set_fraction(fraction.clamp(0.0, 1.0));
        self.update_operation_row(status, Some(fraction.clamp(0.0, 1.0)));
    }

    pub(super) fn complete_operation(&self, summary: &str, success: bool) {
        let imp = self.imp();
        imp.operation_cancel.borrow_mut().take();
        imp.operation_status_label.set_label(summary);
        imp.operation_progress_bar
            .set_fraction(if success { 1.0 } else { 0.0 });
        imp.operation_progress_bar.set_visible(false);
        imp.operation_cancel_button.set_sensitive(false);
        imp.progress_indicator_icon.set_icon_name(Some(if success {
            "file-operation-finished-symbolic"
        } else {
            "file-operation-cancelled-symbolic"
        }));
        self.update_operation_row(summary, Some(if success { 1.0 } else { 0.0 }));
        self.flag_operation_needs_attention();

        let mut history = imp.operation_history.borrow_mut();
        history.push(summary.to_string());
        if history.len() > 20 {
            history.remove(0);
        }
    }

    /// Pulses the operations indicator when an operation finishes.
    ///
    /// Equivalent to the Nautilus `needs-attention` indicator state: the
    /// sidebar button pulses once to draw the user's attention.
    fn flag_operation_needs_attention(&self) {
        let imp = self.imp();
        let button = imp.progress_indicator_button.get();
        button.add_css_class("needs-attention");

        let button = button.clone();
        glib::timeout_add_local_once(std::time::Duration::from_secs(3), move || {
            button.remove_css_class("needs-attention");
        });
    }

    /// Append a row to the sidebar operations popover.
    fn add_operation_row(&self, status: &str) {
        let imp = self.imp();
        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.set_selectable(false);

        let bx = gtk::Box::new(gtk::Orientation::Vertical, 6);
        bx.set_margin_top(6);
        bx.set_margin_bottom(6);
        bx.set_margin_start(6);
        bx.set_margin_end(6);

        let label = gtk::Label::new(Some(status));
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let bar = gtk::ProgressBar::new();
        bar.set_show_text(false);
        bar.set_fraction(0.0);

        bx.append(&label);
        bx.append(&bar);
        row.set_child(Some(&bx));
        imp.operations_list.prepend(&row);

        // Keep the popover history bounded.
        while let Some(last) = imp.operations_list.last_child() {
            if imp.operations_list.observe_children().n_items() <= 20 {
                break;
            }
            imp.operations_list.remove(&last);
        }
    }

    /// Update the most recent operations row (the one currently running).
    fn update_operation_row(&self, status: &str, fraction: Option<f64>) {
        let imp = self.imp();
        let Some(row) = imp.operations_list.first_child().and_downcast::<gtk::ListBoxRow>() else {
            return;
        };
        let Some(bx) = row.child().and_downcast::<gtk::Box>() else {
            return;
        };
        if let Some(label) = bx.first_child().and_downcast::<gtk::Label>() {
            label.set_label(status);
        }
        if let Some(bar) = bx.last_child().and_downcast::<gtk::ProgressBar>() {
            if let Some(fraction) = fraction {
                bar.set_fraction(fraction);
            }
        }
    }

    pub(super) fn cancel_operation(&self) {
        if let Some(cancel) = self.imp().operation_cancel.borrow().as_ref() {
            cancel.store(true, Ordering::Relaxed);
            self.imp()
                .operation_status_label
                .set_label(&gettext("Cancelling operation…"));
            self.imp().operation_cancel_button.set_sensitive(false);
        }
    }

    pub(super) fn close_operation_panel(&self) {
        self.imp().sidebar_bottom_bar.set_revealed(false);
    }

    pub(super) fn show_operation_panel(&self) {
        let imp = self.imp();
        imp.sidebar_bottom_bar.set_revealed(true);
        imp.operations_popover.popup();
    }

    pub(super) fn show_operation_history(&self) {
        let history = self.imp().operation_history.borrow().clone();
        let dialog = gtk::Window::builder()
            .title(gettext("Recent Operations"))
            .transient_for(self)
            .modal(true)
            .default_width(480)
            .default_height(360)
            .build();
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::None);
        list.add_css_class("boxed-list");

        if history.is_empty() {
            let empty = gtk::Label::new(Some(&gettext("No operations have completed yet.")));
            empty.set_margin_top(24);
            empty.set_margin_bottom(24);
            list.append(&empty);
        } else {
            for entry in history.iter().rev() {
                let label = gtk::Label::new(Some(entry));
                label.set_xalign(0.0);
                label.set_wrap(true);
                label.set_margin_top(8);
                label.set_margin_bottom(8);
                label.set_margin_start(10);
                label.set_margin_end(10);
                list.append(&label);
            }
        }

        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&list)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);
        content.append(&scroll);
        dialog.set_child(Some(&content));
        dialog.present();
    }

    pub(super) fn reload_repository(&self) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };

        self.load_repository(repo_path);
    }

    pub(super) fn confirm_restore_project_snapshot(&self) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };
        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_toast(&gettext("No snapshot selected"));
            return;
        };

        let body = format!(
            "{}\n\n{}: {}",
            gettext("The current branch and HEAD will not change. The restored files will appear as local, unstaged changes."),
            gettext("Snapshot"),
            display_hash(&hash)
        );
        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Restore Project Files?"))
            .body(body)
            .build();
        dialog.add_response("cancel", &gettext("Cancel"));
        dialog.add_response("restore", &gettext("Restore Files"));
        dialog.set_response_appearance("restore", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("restore"));
        dialog.set_close_response("cancel");

        let win = self.clone();
        dialog.connect_response(Some("restore"), move |_, _| {
            match restore_project_files(&repo_path, &hash) {
                Ok(()) => {
                    win.show_toast(&gettext("Project files restored as local changes"));
                    win.reload_current_dir_if_possible();
                }
                Err(error) => win.show_error(&format!(
                    "{}: {error}",
                    gettext("Could not restore project files")
                )),
            }
        });
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    pub(super) fn open_repository_in_system(&self) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };

        let file = gio::File::for_path(&repo_path);
        let uri = file.uri();

        if gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>).is_err() {
            self.show_error(&gettext("Could not open repository in the system"));
        }
    }

    pub(super) fn open_repository_in_console(&self) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };

        let attempts: &[(&str, &[&str])] = &[
            ("kgx", &["--working-directory"]),
            ("gnome-terminal", &["--working-directory"]),
            ("konsole", &["--workdir"]),
            ("xfce4-terminal", &["--working-directory"]),
        ];

        for (program, args) in attempts {
            let mut command = Command::new(program);
            command.args(*args).arg(&repo_path);

            if command.spawn().is_ok() {
                return;
            }
        }

        self.show_error(&gettext("Could not open a console for this repository"));
    }

    pub(super) fn copy_current_repository_path(&self) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };

        if let Some(display) = gtk::gdk::Display::default() {
            display.clipboard().set_text(&repo_path.to_string_lossy());
            self.show_toast(&gettext("Repository path copied"));
        }
    }

    pub(super) fn show_merge_conflicts_for_current(&self) {
        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_toast(&gettext("No commit selected"));
            return;
        };

        if !self.try_show_merge_conflict_dialog(&hash) {
            self.show_toast(&gettext("Selected commit is not a merge commit"));
        }
    }

    pub(super) fn show_new_branch_dialog(&self) {
        let dialog = NewBranchDialog::new();
        let win = self.clone();
        dialog.connect_branch_created(move |_, name| {
            win.create_branch(name);
        });
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    /// Create a local branch at HEAD of the currently loaded repository.
    ///
    /// # Borrow-checker / lifetime rationale
    ///
    /// `git2::Branch<'repo>` borrows from the `Repository` it was created on.
    /// When `repo` is a local variable owned by a closure passed to `and_then`,
    /// returning the `Branch` from that closure is illegal (E0515) because the
    /// borrow of `repo` would escape the scope in which `repo` is owned.
    ///
    /// ## Fix
    ///
    /// Two complementary techniques are used here:
    ///
    /// 1. **Resolve HEAD OID in a tightly-scoped block** — the `RefCell` borrow
    ///    on `self.imp().repository` is released before we do anything else,
    ///    so no live borrow is held across later fallible operations.
    ///
    /// 2. **Map `Branch` to `()` inside the closure** — `repo.branch(…)` returns
    ///    `Result<Branch<'_>, git2::Error>`.  Calling `.map(|_| ())` immediately
    ///    drops the `Branch` (and its borrow of `repo`) *within* the closure,
    ///    so the closure returns `Result<(), String>` — a type with no lifetime
    ///    parameters — which can be safely returned to the outer scope.
    pub fn create_branch(&self, name: &str) {
        // ── 1. Resolve repo_path and HEAD OID — borrow ends here ─────────────
        let (repo_path, head_oid) = {
            let guard = self.imp().repository.borrow();
            let Some(ref repo) = *guard else {
                self.show_error(&gettext("No repository loaded."));
                return;
            };

            let head = match repo.head() {
                Ok(h) => h,
                Err(e) => {
                    self.show_error(&format!("{}: {e}", gettext("Cannot read HEAD")));
                    return;
                }
            };

            let oid = match head.peel_to_commit() {
                Ok(c) => c.id(),
                Err(e) => {
                    self.show_error(&format!("{}: {e}", gettext("Cannot peel HEAD to commit")));
                    return;
                }
            };

            // Clone the path so the borrow on `self.imp().repository` can end.
            let path = self.imp().repo_path.borrow().clone();
            (path, oid)
        };
        // repo_guard is dropped here; no borrows of `self.imp()` are live.

        // ── 2. Resolve the repo_path ──────────────────────────────────────────
        let Some(path) = repo_path else {
            self.show_error(&gettext("No repository path stored."));
            return;
        };

        // ── 3. Open a short-lived handle and create the branch ────────────────
        //
        // A secondary `Repository` handle is opened solely for this operation.
        // Its lifetime — and therefore the lifetime of `Branch<'_>` — is
        // entirely contained within this block, satisfying the borrow checker.
        //
        // `.map(|_| ())` drops the `Branch` (and its borrow of `repo`) inside
        // the closure so `Result<(), String>` — with no lifetime parameter —
        // is what gets returned to the outer scope.  Without this `.map`, the
        // compiler would emit E0515 because the `Branch` borrow would escape
        // the closure that owns `repo`.
        let result = git2::Repository::open(&path)
            .map_err(|e| format!("{}: {e}", gettext("Cannot open repository")))
            .and_then(|repo| {
                let commit = repo
                    .find_commit(head_oid)
                    .map_err(|e| format!("{}: {e}", gettext("Cannot find HEAD commit")))?;
                repo.branch(name, &commit, false)
                    .map(|_| ()) // ← drop Branch<'_> here; borrow of `repo` ends
                    .map_err(|e| {
                        format!(
                            "{} \u{2018}{}\u{2019}: {e}",
                            gettext("Failed to create branch"),
                            name
                        )
                    })
            });

        // ── 4. Report outcome ─────────────────────────────────────────────────
        match result {
            Ok(()) => {
                let toast = adw::Toast::new(&format!(
                    "{} \u{2018}{}\u{2019}",
                    gettext("Created branch"),
                    name,
                ));
                if let Some(overlay) = self
                    .imp()
                    .content_toolbar_view
                    .parent()
                    .and_then(|w| w.downcast::<adw::ToastOverlay>().ok())
                {
                    overlay.add_toast(toast);
                }
            }
            Err(msg) => self.show_error(&msg),
        }
    }

    pub fn show_batch_operations_dialog(&self) {
        let mut selected_hashes = HashSet::new();
        let mut child = self.imp().commit_list.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Ok(row) = widget.downcast::<gtk::ListBoxRow>() {
                if row.has_css_class("pattern-match") && !row.widget_name().is_empty() {
                    selected_hashes.insert(row.widget_name().to_string());
                }
            }
        }

        if selected_hashes.is_empty() {
            if let Some(hash) = self.imp().current_hash.borrow().clone() {
                selected_hashes.insert(hash);
            }
        }

        let selected_commits = self
            .imp()
            .all_commits
            .borrow()
            .iter()
            .filter(|commit| selected_hashes.contains(&commit.hash))
            .cloned()
            .collect::<Vec<_>>();

        if selected_commits.is_empty() {
            self.show_toast(&gettext("Select at least one commit"));
            return;
        }

        let dialog = BatchOperationsDialog::new();
        dialog.set_commits(&selected_commits);

        let win = self.clone();
        dialog.connect_operation_requested(move |dlg, op, shas| match op {
            BatchOp::CherryPick { signoff } => {
                let Some(repo_path) = win.imp().repo_path.borrow().clone() else {
                    win.show_error(&gettext("No repository loaded"));
                    return;
                };
                let dlg_ref = dlg.clone();
                let win_ref = win.clone();
                dlg.set_progress_visible(true);
                let (tx, rx) = std::sync::mpsc::sync_channel(1);

                std::thread::spawn(move || {
                    let _ = tx.send(cherry_pick_commits(&repo_path, &shas, signoff));
                });

                glib::idle_add_local(move || match rx.try_recv() {
                    Ok(Ok(count)) => {
                        dlg_ref.mark_done();
                        win_ref.show_toast(&format!(
                            "{} {} commit(s)",
                            gettext("Cherry-picked"),
                            count
                        ));
                        win_ref.reload_repository();
                        glib::ControlFlow::Break
                    }
                    Ok(Err(error)) => {
                        dlg_ref.mark_failed();
                        win_ref.show_error(&format!("{}: {error}", gettext("Cherry-pick failed")));
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(_) => {
                        dlg_ref.mark_failed();
                        win_ref.show_error(&gettext("Cherry-pick worker stopped unexpectedly"));
                        glib::ControlFlow::Break
                    }
                });
            }
            BatchOp::ExportPatches { dest_dir } => {
                let shas_clone = shas.clone();
                let repo_path = win.imp().repo_path.borrow().clone();
                let dlg_ref = dlg.clone();
                dlg.set_progress_visible(true);

                let (tx, rx) = std::sync::mpsc::sync_channel::<()>(1);

                std::thread::spawn(move || {
                    if let Some(repo_path) = repo_path {
                        let _ = std::fs::create_dir_all(&dest_dir);

                        if let Ok(repo) = git2::Repository::open(&repo_path) {
                            for (i, sha) in shas_clone.iter().enumerate() {
                                let patch_path = dest_dir.join(format!(
                                    "{:04}-{}.patch",
                                    i + 1,
                                    short_hash(sha)
                                ));

                                if let Ok(oid) = git2::Oid::from_str(sha) {
                                    if let Ok(commit) = repo.find_commit(oid) {
                                        if let Ok(tree) = commit.tree() {
                                            let parent_tree =
                                                commit.parent(0).ok().and_then(|p| p.tree().ok());
                                            if let Ok(diff) = repo.diff_tree_to_tree(
                                                parent_tree.as_ref(),
                                                Some(&tree),
                                                None,
                                            ) {
                                                let mut buf = Vec::new();
                                                let _ = diff.print(
                                                    git2::DiffFormat::Patch,
                                                    |_d, _h, line| {
                                                        buf.extend_from_slice(line.content());
                                                        true
                                                    },
                                                );
                                                let _ = std::fs::write(&patch_path, &buf);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let _ = tx.send(());
                });

                glib::idle_add_local(move || match rx.try_recv() {
                    Ok(()) => {
                        dlg_ref.mark_done();
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                    Err(_) => glib::ControlFlow::Break,
                });
            }
            BatchOp::CopyShas { short } => {
                let text = shas
                    .iter()
                    .map(|s| {
                        if short {
                            short_hash(s).to_string()
                        } else {
                            s.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Some(display) = gtk::gdk::Display::default() {
                    display.clipboard().set_text(&text);
                }
                win.show_toast(&format!("{} {} SHA(s)", gettext("Copied"), shas.len()));
                dlg.mark_done();
            }
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    pub fn show_select_by_pattern_dialog(&self) {
        let dialog = SelectCommitsByPattern::new();
        dialog.set_commits(&self.imp().all_commits.borrow());

        let win = self.clone();
        dialog.connect_pattern_selected(move |_, pattern, mode, icase| {
            let all = win.imp().all_commits.borrow();
            let matching: std::collections::HashSet<String> = all
                .iter()
                .filter(|c| commit_matches_pattern(c, pattern, mode, icase))
                .map(|c| short_hash(&c.hash).to_string())
                .collect();

            let list = win.imp().commit_list.clone();

            // Mark matching rows with CSS class "pattern-match".
            let mut row = list.first_child();
            while let Some(r) = row {
                if let Some(list_row) = r.downcast_ref::<gtk::ListBoxRow>() {
                    let hash = list_row.widget_name().to_string();

                    if !hash.is_empty() {
                        let is_match = matching.contains(short_hash(&hash));

                        if is_match {
                            list_row.add_css_class("pattern-match");
                        } else {
                            list_row.remove_css_class("pattern-match");
                        }
                    }
                    row = list_row.next_sibling();
                } else {
                    row = r.next_sibling();
                }
            }

            // Scroll to first match.
            let mut row2 = list.first_child();
            while let Some(r) = row2 {
                if let Some(list_row) = r.downcast_ref::<gtk::ListBoxRow>() {
                    if list_row.has_css_class("pattern-match") {
                        list_row.grab_focus();
                        break;
                    }
                    row2 = list_row.next_sibling();
                } else {
                    row2 = r.next_sibling();
                }
            }

            win.show_toast(&format!(
                "{} {} commit(s)",
                gettext("Selected"),
                matching.len()
            ));
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    /// Show the merge-conflict inspector only for merge commits (≥ 2 parents).
    pub(super) fn try_show_merge_conflict_dialog(&self, hash: &str) -> bool {
        let repo_path = match self.imp().repo_path.borrow().clone() {
            Some(p) => p,
            None => return false,
        };

        let repo = match git2::Repository::open(&repo_path) {
            Ok(r) => r,
            Err(_) => return false,
        };

        let oid = match git2::Oid::from_str(hash) {
            Ok(o) => o,
            Err(_) => return false,
        };

        let commit = match repo.find_commit(oid) {
            Ok(c) => c,
            Err(_) => return false,
        };

        if commit.parent_count() < 2 {
            return false;
        }

        let ours = commit.parent(0).ok();
        let theirs = commit.parent(1).ok();

        let conflict_file = ours
            .as_ref()
            .zip(theirs.as_ref())
            .and_then(|(o, t)| {
                let ot = o.tree().ok()?;
                let tt = t.tree().ok()?;
                let diff = repo.diff_tree_to_tree(Some(&ot), Some(&tt), None).ok()?;
                let mut first_path = None;
                diff.foreach(
                    &mut |delta, _| {
                        if first_path.is_none() {
                            if let Some(p) = delta.new_file().path() {
                                first_path = Some(p.to_string_lossy().to_string());
                            }
                        }
                        true
                    },
                    None,
                    None,
                    None,
                )
                .ok()?;
                first_path
            })
            .unwrap_or_else(|| "(unknown)".to_string());

        let diff_text = ours
            .as_ref()
            .zip(theirs.as_ref())
            .and_then(|(o, t)| {
                let ot = o.tree().ok()?;
                let tt = t.tree().ok()?;
                let mut opts = git2::DiffOptions::new();
                opts.pathspec(&conflict_file);
                let diff = repo
                    .diff_tree_to_tree(Some(&ot), Some(&tt), Some(&mut opts))
                    .ok()?;
                let mut buf = Vec::new();
                diff.print(git2::DiffFormat::Patch, |_d, _h, line| {
                    buf.extend_from_slice(line.content());
                    true
                })
                .ok()?;
                Some(String::from_utf8_lossy(&buf).to_string())
            })
            .unwrap_or_default();

        let fmt_commit = |c: Option<&git2::Commit>| -> (String, String, String) {
            match c {
                Some(commit) => (
                    commit.id().to_string(),
                    commit.author().name().unwrap_or("").to_string(),
                    glib::DateTime::from_unix_local(commit.time().seconds())
                        .and_then(|d| d.format("%Y-%m-%d %H:%M"))
                        .map(|s| s.to_string())
                        .unwrap_or_default(),
                ),
                None => (String::new(), String::new(), String::new()),
            }
        };

        let (ours_sha, ours_author, ours_date) = fmt_commit(ours.as_ref());
        let (theirs_sha, theirs_author, theirs_date) = fmt_commit(theirs.as_ref());

        let info = ConflictInfo {
            file_path: conflict_file,
            ours_sha,
            ours_author,
            ours_date,
            theirs_sha,
            theirs_author,
            theirs_date,
            diff_text,
        };

        let dialog = MergeConflictDialog::new();
        dialog.load_conflict(&info);

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
        true
    }

    pub(super) fn open_repo_dialog(&self) {
        let dialog = gtk::FileDialog::builder()
            .title(gettext("Open Repository"))
            .modal(true)
            .build();
        let win = self.clone();
        dialog.select_folder(Some(self), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    if git2::Repository::open(&path).is_ok() {
                        win.load_repository(path);
                    } else {
                        win.show_invalid_repository_and_retry(&path);
                    }
                }
            }
        });
    }

    pub(super) fn show_invalid_repository_and_retry(&self, path: &Path) {
        let folder = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_else(|| path.to_str().unwrap_or(""));
        let message = format!(
            "{}\n\n{}: {}",
            gettext(
                "No Git repository was found in the selected folder. Choose a folder that contains a Git project."
            ),
            gettext("Selected folder"),
            folder
        );

        let dialog = adw::AlertDialog::builder()
            .heading(gettext("Repository Not Found"))
            .body(message)
            .build();
        dialog.add_response("cancel", &gettext("Cancel"));
        dialog.add_response("choose-again", &gettext("Choose Another Folder"));
        dialog.set_default_response(Some("choose-again"));
        dialog.set_close_response("cancel");

        let win = self.clone();
        dialog.connect_response(Some("choose-again"), move |_, _| {
            win.open_repo_dialog();
        });
        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    pub(super) fn show_clone_repository_dialog(&self) {
        let dialog = CloneRepositoryDialog::new();

        let win = self.clone();
        dialog.connect_clone_requested(move |_, url, destination, save, include_files| {
            let destination = if save {
                PathBuf::from(destination)
            } else {
                win.temporary_clone_destination(url)
            };
            win.clone_repository(url.to_string(), destination, save, include_files);
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    pub(super) fn temporary_clone_destination(&self, url: &str) -> PathBuf {
        let repository_name = url
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("repository")
            .trim_end_matches(".git");
        let repository_name = repository_name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                    character
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        );
        std::env::temp_dir()
            .join("temporal-explorer")
            .join("repositories")
            .join(unique)
            .join(repository_name)
    }

    pub(super) fn clone_repository(&self, url: String, destination: PathBuf, save: bool, include_files: bool) {
        let url = url.trim().to_string();

        if url.is_empty() {
            self.show_error(&gettext("Repository URL is required"));
            return;
        }

        if destination.exists() {
            if !destination.is_dir() {
                self.show_error(&gettext("Clone destination must be a folder"));
                return;
            }

            match std::fs::read_dir(&destination) {
                Ok(entries) => {
                    if entries.into_iter().next().is_some() {
                        self.show_error(&gettext("Clone destination must be empty"));
                        return;
                    }
                }
                Err(e) => {
                    self.show_error(&format!(
                        "{}: {e}",
                        gettext("Cannot read clone destination")
                    ));
                    return;
                }
            }
        } else if let Some(parent) = destination.parent() {
            if !parent.exists() && save {
                self.show_error(&gettext("Parent folder does not exist"));
                return;
            }
            if !parent.exists() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    self.show_error(&format!(
                        "{}: {error}",
                        gettext("Cannot create temporary clone directory")
                    ));
                    return;
                }
            }
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let clone_progress = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        if !self.begin_operation(&gettext("Connecting to remote repository…"), cancel.clone()) {
            return;
        }

        let worker_url = url.clone();
        let worker_destination = destination.clone();
        let worker_cancel = cancel.clone();
        let worker_progress = clone_progress.clone();
        let saved_clone = save;
        let (tx, rx) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            let result = clone_git_repository(
                worker_url,
                worker_destination,
                worker_cancel,
                worker_progress,
                !saved_clone || include_files,
            );
            tx.send(result).ok();
        });

        let win = self.clone();
        glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
            match rx.try_recv() {
                Ok(Ok(path)) => {
                    let message = if saved_clone {
                        gettext("Repository saved and opened")
                    } else {
                        gettext("Repository opened temporarily")
                    };
                    win.complete_operation(&message, true);
                    win.show_toast(&message);
                    win.load_repository(path);
                    glib::ControlFlow::Break
                }
                Ok(Err(message)) => {
                    win.complete_operation(&message, false);
                    win.show_error(&message);
                    glib::ControlFlow::Break
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    let status = if cancel.load(Ordering::Relaxed) {
                        gettext("Cancelling clone…")
                    } else {
                        gettext("Cloning repository…")
                    };
                    let fraction = clone_progress.load(Ordering::Relaxed) as f64 / 100.0;
                    win.update_operation(fraction, &status);
                    glib::ControlFlow::Continue
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    win.complete_operation(&gettext("Clone operation stopped unexpectedly"), false);
                    win.show_error(&gettext("Clone operation stopped unexpectedly"));
                    glib::ControlFlow::Break
                }
            }
        });
    }

    pub fn load_repository(&self, path: PathBuf) {
        let discard_pending = self
            .imp()
            .pending_favorite
            .borrow()
            .as_ref()
            .is_some_and(|(pending_repo, _, _)| pending_repo != &path);
        if discard_pending {
            self.imp().pending_favorite.borrow_mut().take();
        }

        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(prev) = self.imp().load_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }
        *self.imp().load_cancel.borrow_mut() = Some(cancel.clone());

        match git2::Repository::open(&path) {
            Ok(repo) => {
                let repo_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("repository")
                    .to_string();

                let imp = self.imp();
                *imp.repo_path.borrow_mut() = Some(path.clone());
                *imp.repository.borrow_mut() = Some(DebugRepository(repo));
                *imp.file_index_store.borrow_mut() = Some(FileIndexStore::open(&path));
                *imp.repo_name.borrow_mut() = repo_name.clone();
                *imp.current_hash.borrow_mut() = None;
                imp.current_oid.set(None);
                *imp.current_dir.borrow_mut() = PathBuf::new();
                imp.history_back.borrow_mut().clear();
                imp.history_forward.borrow_mut().clear();
                imp.commit_nav_back.borrow_mut().clear();
                imp.commit_nav_forward.borrow_mut().clear();
                imp.toolbar.history_controls().reset();
                if let Some(page) = imp.tab_view.selected_page() {
                    imp.tab_view.close_other_pages(&page);
                    imp.tab_states.borrow_mut().clear();
                    imp.tab_states
                        .borrow_mut()
                        .insert(Self::tab_key(&page), TabState::default());
                    self.update_tab_title(&page);
                }
                imp.dir_cache.borrow_mut().clear();
                imp.toolbar.new_branch_button().set_sensitive(true);

                // Populate branch chips immediately (O(branches), non-blocking),
                // then compute the branch -> commit-hash index in the background
                // so giant repositories (e.g. Linux kernel) never freeze the UI.
                let branches = imp
                    .repository
                    .borrow()
                    .as_ref()
                    .map(|repo| collect_local_branch_names(&repo.0))
                    .unwrap_or_default();
                imp.branch_commit_index.borrow_mut().clear();

                if let Some(ref pop) = *imp.filter_popover.borrow() {
                    pop.populate_branch_chips(&branches);
                }

                let (btx, brx) =
                    std::sync::mpsc::sync_channel::<HashMap<String, HashSet<String>>>(1);
                let branch_repo_path = path.clone();
                let branch_cancel = cancel.clone();
                std::thread::spawn(move || {
                    const MAX_COMMITS_PER_BRANCH_INDEX: usize = 50_000;
                    if let Ok(repo) = git2::Repository::open(&branch_repo_path) {
                        let idx = collect_branch_commit_index_cancelable(
                            &repo,
                            &branch_cancel,
                            MAX_COMMITS_PER_BRANCH_INDEX,
                        );
                        if !branch_cancel.load(Ordering::Relaxed) {
                            let _ = btx.send(idx);
                        }
                    }
                });
                let win_branch = self.clone();
                let branch_poll_cancel = cancel.clone();
                glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
                    if branch_poll_cancel.load(Ordering::Relaxed) {
                        return glib::ControlFlow::Break;
                    }
                    match brx.try_recv() {
                        Ok(branch_index) => {
                            *win_branch.imp().branch_commit_index.borrow_mut() = branch_index;
                            glib::ControlFlow::Break
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                        Err(_) => glib::ControlFlow::Break,
                    }
                });

                self.load_timeline(cancel);
            }
            Err(e) => self.show_error(&format!("{}: {e}", gettext("Failed to open repository"))),
        }
    }

    /// Displays an empty-repository state in the timeline sidebar.
    ///
    /// Called when `load_timeline` receives the EOS sentinel but `all_commits`
    /// is still empty — meaning the repository has no commits reachable from
    /// HEAD (freshly initialised repo, unborn branch, or detached HEAD with no
    /// history).  Showing an explicit message is far better than leaving the
    /// sidebar blank and making the user wonder whether loading is still in
    /// progress.
    pub(super) fn show_empty_repository_state(&self) {
        let imp = self.imp();
        imp.year_list.remove_all();
        *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
        imp.selected_year.set(0);
        imp.selected_month.set(0);
        imp.timeline_stack.set_visible_child_name("years");
        imp.timeline_back_button.set_visible(false);
        imp.timeline_header_title.set_title(&gettext("Timeline"));
        imp.timeline_header_title
            .set_subtitle(&gettext("No commits found"));
        imp.split_view.set_show_sidebar(true);
    }
}
