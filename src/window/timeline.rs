/* timeline.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Commit timeline, drill-down (years/months/commits), filters and snapshot search.

use super::*;
use super::util::*;

#[inline]
pub(super) fn year_from_timestamp(ts: i64) -> Option<i32> {
    glib::DateTime::from_unix_local(ts).ok().map(|dt| dt.year())
}

#[inline]
pub(super) fn year_month_from_timestamp(ts: i64) -> Option<(i32, u32)> {
    let dt = glib::DateTime::from_unix_local(ts).ok()?;
    Some((dt.year(), dt.month() as u32))
}

pub(super) fn bump_year_count(year_counts: &mut Vec<(i32, usize)>, year: i32) {
    if let Some((_, count)) = year_counts.iter_mut().find(|(y, _)| *y == year) {
        *count += 1;
    } else {
        year_counts.push((year, 1));
        year_counts.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    }
}

/// Returns `true` if `query` matches any date-related representation of `ts`.
///
/// Recognised formats: ISO date (`2024-03-15`), year-month (`2024-03`),
/// bare year (`2024`), full/abbreviated English month name, and the
/// locale-translated month name returned by [`timeline_filter::month_name`].
pub(super) fn matches_calendar(ts: i64, q: &str) -> bool {
    // Fast reject for ordinary text queries. Calendar matches are short:
    // years, ISO dates, year-month values, timestamps, and month names.
    if q.len() < 2 || q.len() > 16 {
        return false;
    }

    let Ok(dt) = glib::DateTime::from_unix_local(ts) else {
        return false;
    };

    let year = dt.year();
    let month = dt.month() as u32;
    let day = dt.day_of_month();

    // Common numeric filters first, before allocating formatted strings.
    if q.len() == 4 {
        if let Ok(q_year) = q.parse::<i32>() {
            return year == q_year;
        }
    }

    let month_full = match month {
        1 => "january",
        2 => "february",
        3 => "march",
        4 => "april",
        5 => "may",
        6 => "june",
        7 => "july",
        8 => "august",
        9 => "september",
        10 => "october",
        11 => "november",
        12 => "december",
        _ => "",
    };
    let month_abbr = &month_full[..3.min(month_full.len())];

    // Month-only queries do not need ISO/timestamp allocation.
    if q.chars().all(|c| c.is_ascii_alphabetic()) {
        return month_full.contains(q)
            || month_abbr == q
            || timeline_filter::month_name(month)
                .to_lowercase()
                .contains(q);
    }

    let iso_date = format!("{:04}-{:02}-{:02}", year, month, day);
    let year_month = format!("{:04}-{:02}", year, month);
    let human = dt
        .format("%Y-%m-%d %H:%M")
        .map(|s| s.to_string())
        .unwrap_or_default();

    iso_date.contains(q) || year_month.contains(q) || human.contains(q)
}

pub(super) fn commit_matches_lightweight_filters(
    commit: &CommitInfo,
    filter: &FilterState,
    branch_hashes: Option<&HashSet<String>>,
    q: &str,
) -> bool {
    if let Some(ref author) = filter.author {
        let wanted_author = author.to_lowercase();
        if !commit.author_matches_query(&wanted_author) {
            return false;
        }
    }

    if let Some(hashes) = branch_hashes {
        if !hashes.contains(&commit.hash) {
            return false;
        }
    }

    if let Some(since) = filter.date.from {
        if commit.timestamp < since {
            return false;
        }
    }

    if let Some(until) = filter.date.to {
        if commit.timestamp > until {
            return false;
        }
    }

    if !q.is_empty() && !commit.matches_text_query(q) && !matches_calendar(commit.timestamp, q) {
        return false;
    }

    true
}

pub(super) fn search_snapshot_files(
    repo_path: &Path,
    hash: &str,
    query: &str,
) -> Result<SnapshotSearchResults, String> {
    fn walk_tree(
        repo: &git2::Repository,
        tree: &git2::Tree<'_>,
        prefix: &Path,
        query: &str,
        depth: usize,
        scanned: &mut usize,
        matches: &mut Vec<(PathBuf, String)>,
        truncated: &mut bool,
    ) -> Result<(), git2::Error> {
        if depth > SNAPSHOT_SEARCH_MAX_DEPTH
            || *scanned >= SNAPSHOT_SEARCH_MAX_FILES
            || matches.len() >= SNAPSHOT_SEARCH_MAX_MATCHES
        {
            *truncated = true;
            return Ok(());
        }

        for entry in tree.iter() {
            if *scanned >= SNAPSHOT_SEARCH_MAX_FILES || matches.len() >= SNAPSHOT_SEARCH_MAX_MATCHES
            {
                *truncated = true;
                break;
            }
            let Some(name) = entry.name().filter(|name| !name.is_empty()) else {
                continue;
            };
            *scanned += 1;
            let path = prefix.join(name);

            match entry.kind() {
                Some(git2::ObjectType::Tree) => {
                    let subtree = repo.find_tree(entry.id())?;
                    walk_tree(
                        repo,
                        &subtree,
                        &path,
                        query,
                        depth + 1,
                        scanned,
                        matches,
                        truncated,
                    )?;
                }
                Some(git2::ObjectType::Blob) => {
                    let name_matches = path.to_string_lossy().to_lowercase().contains(query);
                    let blob = repo.find_blob(entry.id())?;
                    let content_match = if blob.size() <= SNAPSHOT_SEARCH_MAX_FILE_SIZE
                        && !blob.content().contains(&0)
                    {
                        std::str::from_utf8(blob.content())
                            .ok()
                            .and_then(|contents| {
                                contents
                                    .lines()
                                    .find(|line| line.to_lowercase().contains(query))
                            })
                            .map(|line| line.chars().take(180).collect::<String>())
                    } else {
                        None
                    };

                    if name_matches || content_match.is_some() {
                        let description =
                            content_match.unwrap_or_else(|| gettext("File name matches"));
                        matches.push((path, description));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    let repo = git2::Repository::open(repo_path).map_err(|error| error.to_string())?;
    let oid = git2::Oid::from_str(hash).map_err(|error| error.to_string())?;
    let commit = repo.find_commit(oid).map_err(|error| error.to_string())?;
    let tree = commit.tree().map_err(|error| error.to_string())?;
    let mut matches = Vec::new();
    let mut scanned = 0;
    let mut truncated = false;
    walk_tree(
        &repo,
        &tree,
        Path::new(""),
        &query.to_lowercase(),
        0,
        &mut scanned,
        &mut matches,
        &mut truncated,
    )
    .map_err(|error| error.to_string())?;

    Ok(SnapshotSearchResults { matches, truncated })
}

enum SearchProgressMessage {
    Progress {
        indexed: usize,
        total: usize,
        cache_hits: usize,
        matches: usize,
    },
    Done(SearchRunResult),
    Cancelled,
    Error(String),
}

enum BackgroundIndexMessage {
    Batch(Vec<(git2::Oid, CommitFileIndex)>),
    Done,
}

struct SearchRunResult {
    result_indices: Vec<usize>,
    newly_indexed_files: Vec<(git2::Oid, CommitFileIndex)>,
    file_match_counts: HashMap<git2::Oid, usize>,
    indexed: usize,
    cache_hits: usize,
    diff_errors: usize,
}

pub(super) struct SnapshotSearchResults {
    matches: Vec<(PathBuf, String)>,
    truncated: bool,
}

impl TemporalExplorerWindow {
    pub(super) fn search_snapshot_content(&self) {
        let Some(repo_path) = self.imp().repo_path.borrow().clone() else {
            self.show_toast(&gettext("No repository loaded"));
            return;
        };
        let Some(hash) = self.imp().current_hash.borrow().clone() else {
            self.show_toast(&gettext("No snapshot selected"));
            return;
        };

        let dialog = gtk::Window::builder()
            .title(gettext("Search Snapshot"))
            .transient_for(self)
            .modal(true)
            .default_width(620)
            .default_height(500)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        content.set_margin_top(18);
        content.set_margin_bottom(18);
        content.set_margin_start(18);
        content.set_margin_end(18);

        let query = gtk::Entry::builder()
            .placeholder_text(gettext("Search file names and contents"))
            .hexpand(true)
            .build();
        let search_button = gtk::Button::with_label(&gettext("Search"));
        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        search_row.append(&query);
        search_row.append(&search_button);
        content.append(&search_row);

        let status = gtk::Label::new(Some(&gettext("Enter a search term.")));
        status.set_xalign(0.0);
        content.append(&status);

        let results = gtk::ListBox::new();
        results.set_selection_mode(gtk::SelectionMode::None);
        results.add_css_class("boxed-list");
        let scroll = gtk::ScrolledWindow::builder()
            .vexpand(true)
            .child(&results)
            .build();
        content.append(&scroll);
        dialog.set_child(Some(&content));

        let win = self.clone();
        let dialog_for_search = dialog.clone();
        let query_for_search = query.clone();
        let status_for_search = status.clone();
        let results_for_search = results.clone();
        search_button.connect_clicked(move |_| {
            let term = query_for_search.text().trim().to_string();
            if term.is_empty() {
                status_for_search.set_label(&gettext("Enter a search term."));
                return;
            }

            results_for_search.remove_all();
            status_for_search.set_label(&gettext("Searching snapshot…"));
            let (tx, rx) = std::sync::mpsc::channel();
            let worker_repo = repo_path.clone();
            let worker_hash = hash.clone();
            let worker_term = term.clone();
            let result_hash = hash.clone();
            std::thread::spawn(move || {
                let result = search_snapshot_files(&worker_repo, &worker_hash, &worker_term);
                let _ = tx.send(result);
            });

            let win = win.clone();
            let dialog = dialog_for_search.clone();
            let status = status_for_search.clone();
            let results = results_for_search.clone();
            let term_for_results = term.clone();
            glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                match rx.try_recv() {
                    Ok(Ok(search_results)) => {
                        if search_results.matches.is_empty() {
                            status.set_label(&gettext("No matching files found."));
                        } else {
                            let message = if search_results.truncated {
                                format!(
                                    "{} {} {}",
                                    gettext("Search limited to the first"),
                                    search_results.matches.len(),
                                    gettext("matches")
                                )
                            } else {
                                format!(
                                    "{} {}",
                                    search_results.matches.len(),
                                    gettext("matching file(s)")
                                )
                            };
                            status.set_label(&message);
                        }

                        for (path, snippet) in search_results.matches {
                            let label =
                                gtk::Label::new(Some(&format!("{}\n{}", path.display(), snippet)));
                            label.set_xalign(0.0);
                            label.set_wrap(true);
                            label.set_margin_top(8);
                            label.set_margin_bottom(8);
                            label.set_margin_start(10);
                            label.set_margin_end(10);
                            let button = gtk::Button::new();
                            button.set_child(Some(&label));
                            button.add_css_class("flat");
                            let row = gtk::ListBoxRow::new();
                            row.set_child(Some(&button));
                            results.append(&row);

                            let win = win.clone();
                            let dialog = dialog.clone();
                            let path = path.clone();
                            let snapshot_hash = result_hash.clone();
                            let term = term_for_results.clone();
                            button.connect_clicked(move |_| {
                                dialog.close();
                                win.on_commit_selected(snapshot_hash.clone());
                                win.navigate_to_dir(
                                    path.parent().unwrap_or_else(|| Path::new("")).to_path_buf(),
                                );
                                win.show_toast(&format!("{}: {}", term, path.display()));
                            });
                        }
                        glib::ControlFlow::Break
                    }
                    Ok(Err(message)) => {
                        status.set_label(&gettext("Search failed"));
                        win.show_error(&message);
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        status.set_label(&gettext("Search stopped unexpectedly"));
                        glib::ControlFlow::Break
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                }
            });
        });
        let search_button_for_enter = search_button.clone();
        query.connect_activate(move |_| {
            search_button_for_enter.emit_clicked();
        });

        dialog.present();
        query.grab_focus();
    }

    pub(super) fn focus_commit_search(&self) {
        self.enter_search_mode();
    }

    pub(super) fn toggle_filter_popover(&self) {
        self.enter_search_mode();
        let button = &self.imp().filter_button;
        button.set_active(!button.is_active());
    }

    pub(super) fn open_date_range_dialog(&self) {
        self.enter_search_mode();
        if let Some(ref popover) = *self.imp().filter_popover.borrow() {
            popover.open_date_range_dialog();
        }
    }

    pub(super) fn clear_all_filters(&self) {
        if let Some(ref popover) = *self.imp().filter_popover.borrow() {
            popover.reset_all();
        }
        self.imp().commit_search_entry.set_text("");
        self.on_search_changed(String::new());
    }

    pub fn show_filter_types_dialog(&self) {
        let dialog = FilterTypesDialog::new();

        let win = self.clone();
        dialog.connect_file_type_selected(move |_, ext| {
            {
                let mut fs = win.imp().filter_state.borrow_mut();
                fs.files.other_ext = if ext.is_empty() {
                    None
                } else {
                    Some(ext.to_string())
                };
            }
            let q = win.imp().last_query.borrow().clone();
            win.run_search(q);

            if let Some(ref pop) = *win.imp().filter_popover.borrow() {
                pop.set_file_ext_filter(ext);
            }
        });

        AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
    }

    pub(super) fn enter_search_mode(&self) {
        let imp = self.imp();
        imp.split_view.set_show_sidebar(true);
        imp.sidebar_search_button.set_active(true);
        imp.sidebar_search_revealer.set_reveal_child(true);
        imp.commit_search_entry.grab_focus();
    }

    pub(super) fn leave_search_mode(&self) {
        let imp = self.imp();
        imp.commit_search_entry.set_text("");
        imp.filter_button.set_active(false);
        imp.sidebar_search_button.set_active(false);
        imp.sidebar_search_revealer.set_reveal_child(false);
    }

    pub(super) fn rebuild_author_chips_from_all_commits(&self) {
        const MAX_AUTHOR_CHIPS: usize = 60;
        let imp = self.imp();
        let pop_borrow = imp.filter_popover.borrow();
        let Some(ref pop) = *pop_borrow else { return };

        let commits = imp.all_commits.borrow();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for commit in commits.iter() {
            let author = commit.author.trim();
            if !author.is_empty() {
                *counts.entry(author.to_string()).or_default() += 1;
            }
        }

        // Keep the top contributors so repositories with tens of thousands of
        // authors (e.g. Linux kernel) never spawn 30k+ GtkToggleButtons.
        let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
        ranked.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ranked.truncate(MAX_AUTHOR_CHIPS);
        let authors: Vec<String> = ranked.into_iter().map(|(name, _)| name).collect();

        pop.populate_author_chips(&authors);
    }

    pub(super) fn load_timeline(&self, cancel: Arc<AtomicBool>) {
        // 500 commits per page: fast enough for incremental display,
        // large enough to avoid per-page GTK overhead on large repos.
        const TIMELINE_PAGE_SIZE: usize = 500;

        let repo_path = match self.imp().repo_path.borrow().clone() {
            Some(p) => p,
            None => return,
        };

        self.imp().loading_commits.set(true);
        self.imp().all_commits.borrow_mut().clear();
        self.imp().commit_index.borrow_mut().clear();
        self.imp().year_counts.borrow_mut().clear();
        self.imp().seen_authors.borrow_mut().clear();
        self.imp().changed_files_cache.borrow_mut().clear();
        self.imp().file_filter_results_cache.borrow_mut().take();

        // Show the empty state *before* the worker starts so the UI never
        // displays stale content from a previous repository during loading.
        self.show_empty_state();

        // Bounded channel (capacity 4) allows the worker to stay ahead of the
        // 60 FPS UI poller without buffering unbounded pages in RAM.
        let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Vec<CommitInfo>, String>>(4);

        let page_size = self
            .imp()
            .settings
            .get()
            .map(|s| s.int("commits-per-page").clamp(10, 500) as usize)
            .unwrap_or(TIMELINE_PAGE_SIZE);

        let cancel_worker = cancel.clone();
        std::thread::spawn(move || {
            let result = HistoryReader::open(&repo_path).and_then(|reader| {
                reader.list_commits_paginated_cancelable(page_size, &cancel_worker, |page| {
                    if cancel_worker.load(Ordering::Relaxed) {
                        return;
                    }
                    let _ = tx.send(Ok(page));
                })
            });

            if cancel_worker.load(Ordering::Relaxed) {
                return;
            }

            // Send the EOS sentinel (empty vec) on success, or the error string.
            // The UI callback distinguishes them by the is_empty() check.
            match result {
                Ok(()) => {
                    let _ = tx.send(Ok(Vec::new()));
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                }
            }
        });

        let win = self.clone();
        let mut last_rendered_years = 0usize;
        let mut pages_since_year_refresh = 0usize;
        // Poll at 60 FPS instead of `idle_add_local` busy-spinning at 100% CPU
        // while waiting for libgit2's revwalk on gigantic repositories.
        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            if cancel.load(Ordering::Relaxed) {
                return glib::ControlFlow::Break;
            }

            let mut received_any = false;
            let mut finished = false;
            let mut fatal_error: Option<String> = None;

            // Drain up to 8 pages per frame for fast ingestion while keeping UI smooth.
            for _ in 0..8 {
                match rx.try_recv() {
                    Ok(Ok(page)) if page.is_empty() => {
                        finished = true;
                        break;
                    }
                    Ok(Ok(page)) => {
                        received_any = true;
                        pages_since_year_refresh += 1;
                        let imp = win.imp();
                        let base = imp.all_commits.borrow().len();
                        {
                            let mut index = imp.commit_index.borrow_mut();
                            let mut year_counts = imp.year_counts.borrow_mut();
                            index.reserve(page.len());

                            for (offset, commit) in page.iter().enumerate() {
                                index.insert(commit.hash.clone(), base + offset);

                                if let Some(year) = year_from_timestamp(commit.timestamp) {
                                    bump_year_count(&mut year_counts, year);
                                }
                            }
                        }
                        imp.all_commits.borrow_mut().extend(page);
                    }
                    Ok(Err(e)) => {
                        fatal_error = Some(e);
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }

            if let Some(e) = fatal_error {
                win.imp().loading_commits.set(false);
                win.show_error(&format!("{}: {e}", gettext("Failed to read history")));
                return glib::ControlFlow::Break;
            }

            if finished {
                win.imp().loading_commits.set(false);

                let reachable = win
                    .imp()
                    .all_commits
                    .borrow()
                    .iter()
                    .enumerate()
                    .map(|(index, commit)| (commit.oid(), index))
                    .collect::<HashMap<_, _>>();
                if let Some(store) = win.imp().file_index_store.borrow_mut().as_mut() {
                    store.retain_oids(&reachable);
                    let store_to_save = store.clone();
                    std::thread::spawn(move || {
                        let _ = store_to_save.save();
                    });
                }
                win.start_background_file_index(cancel.clone());

                if win.imp().all_commits.borrow().is_empty() {
                    win.show_empty_repository_state();
                } else {
                    win.rebuild_author_chips_from_all_commits();
                    win.imp().split_view.set_show_sidebar(true);
                    win.populate_year_list();
                }
                if let Some((repo, hash, path)) = win.imp().pending_favorite.borrow_mut().take() {
                    win.open_loaded_favorite(repo, hash, path);
                }
                if let Some((state, title)) = win.imp().pending_tab_state.borrow_mut().take() {
                    if let Some(page) = win.imp().tab_view.selected_page() {
                        page.set_title(&title);
                        win.imp()
                            .tab_states
                            .borrow_mut()
                            .insert(Self::tab_key(&page), state);
                        win.restore_selected_tab_state();
                    }
                }

                return glib::ControlFlow::Break;
            }

            if received_any {
                let year_len = win.imp().year_counts.borrow().len();
                // Refresh year_list immediately when a new year appears, or
                // every 10 pages to avoid thousands of full GTK ListBox rebuilds.
                if year_len != last_rendered_years || pages_since_year_refresh >= 10 {
                    last_rendered_years = year_len;
                    pages_since_year_refresh = 0;
                    win.populate_year_list();
                    win.imp().split_view.set_show_sidebar(true);
                }
            }

            glib::ControlFlow::Continue
        });
    }

    pub(super) fn start_background_file_index(&self, cancel: Arc<AtomicBool>) {
        let repo_path = match self.imp().repo_path.borrow().clone() {
            Some(path) => path,
            None => return,
        };

        let existing = self
            .imp()
            .file_index_store
            .borrow()
            .as_ref()
            .map(FileIndexStore::snapshot)
            .unwrap_or_default();

        // Cap automatic background diff indexing to the newest 4,000 commits so
        // multi-million-commit repos (e.g. Linux kernel) don't saturate CPU/disk
        // for hours after opening.
        const MAX_BACKGROUND_INDEX_COMMITS: usize = 4_000;
        let missing_oids = {
            let commits = self.imp().all_commits.borrow();
            commits
                .iter()
                .take(MAX_BACKGROUND_INDEX_COMMITS)
                .map(|commit| commit.oid())
                .filter(|oid| !existing.contains_key(oid))
                .collect::<Vec<_>>()
        };

        if missing_oids.is_empty() {
            return;
        }

        // Index newest commits first so the first extension search becomes useful sooner.
        let mut missing_oids = missing_oids;
        missing_oids.reverse();

        let (tx, rx) = std::sync::mpsc::sync_channel::<BackgroundIndexMessage>(4);
        let worker_cancel = cancel.clone();

        std::thread::spawn(move || {
            let Ok(repo) = git2::Repository::open(&repo_path) else {
                let _ = tx.send(BackgroundIndexMessage::Done);
                return;
            };

            let mut batch = Vec::with_capacity(64);
            for oid in missing_oids {
                if worker_cancel.load(Ordering::Relaxed) {
                    break;
                }

                if let Ok(index) = build_commit_file_index(&repo, oid) {
                    batch.push((oid, index));
                }

                if batch.len() >= 64 {
                    let _ = tx.send(BackgroundIndexMessage::Batch(std::mem::take(&mut batch)));
                }

                if batch.len() % 16 == 0 {
                    std::thread::yield_now();
                }
            }

            if !batch.is_empty() {
                let _ = tx.send(BackgroundIndexMessage::Batch(batch));
            }
            let _ = tx.send(BackgroundIndexMessage::Done);
        });

        let win = self.clone();
        glib::idle_add_local(move || {
            if cancel.load(Ordering::Relaxed) {
                return glib::ControlFlow::Break;
            }

            loop {
                match rx.try_recv() {
                    Ok(BackgroundIndexMessage::Batch(batch)) => {
                        if let Some(store) = win.imp().file_index_store.borrow_mut().as_mut() {
                            store.insert_many(batch);
                        }
                    }
                    Ok(BackgroundIndexMessage::Done) => {
                        if let Some(store) = win.imp().file_index_store.borrow().as_ref() {
                            let store_to_save = store.clone();
                            std::thread::spawn(move || {
                                let _ = store_to_save.save();
                            });
                        }
                        return glib::ControlFlow::Break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return glib::ControlFlow::Continue;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        return glib::ControlFlow::Break;
                    }
                }
            }
        });
    }

    pub(super) fn visible_timeline_commits<'a>(&'a self, commits: &'a [CommitInfo]) -> Vec<&'a CommitInfo> {
        let filter = self.imp().filter_state.borrow().clone();

        commits
            .iter()
            .filter(|commit| {
                // Author filter
                if let Some(ref author) = filter.author {
                    let wanted_author = author.to_lowercase();
                    if !commit.author_matches_query(&wanted_author) {
                        return false;
                    }
                }

                // Date range filter
                if let Some(since) = filter.date.from {
                    if commit.timestamp < since {
                        return false;
                    }
                }

                if let Some(until) = filter.date.to {
                    if commit.timestamp >= until {
                        return false;
                    }
                }

                // Branch filter is intentionally not handled here because it depends
                // on branch_commit_index and is already handled in run_search().
                // The timeline sidebar remains commit-date/author/date driven.

                true
            })
            .collect()
    }

    pub(super) fn year_counts_for_visible_commits(&self, commits: &[CommitInfo]) -> Vec<(i32, usize)> {
        let visible = self.visible_timeline_commits(commits);
        let mut counts: Vec<(i32, usize)> = Vec::new();

        for commit in visible {
            if let Some(year) = year_from_timestamp(commit.timestamp) {
                bump_year_count(&mut counts, year);
            }
        }

        counts
    }

    pub(super) fn populate_year_list(&self) {
        let imp = self.imp();
        imp.year_list.remove_all();

        let commits = imp.all_commits.borrow();
        let year_counts = self.year_counts_for_visible_commits(&commits);

        for (year, count) in year_counts.iter() {
            imp.year_list
                .append(&commit_controller::build_year_row(*year, *count));
        }

        imp.selected_year.set(0);
        imp.selected_month.set(0);
        *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
        imp.timeline_stack.set_visible_child_name("years");
        imp.timeline_back_button.set_visible(false);
        imp.timeline_header_title.set_title(&gettext("Timeline"));

        if self.imp().filter_state.borrow().is_active() {
            imp.timeline_header_title
                .set_subtitle(&format!("{} year(s)", year_counts.len()));
        } else {
            imp.timeline_header_title.set_subtitle("");
        }
    }

    pub(super) fn reapply_active_file_filter(&self) {
        let file_filter_active = self.imp().filter_state.borrow().files.is_active();
        if file_filter_active {
            if !self.apply_cached_file_filter_for_current_scope() {
                let q = self.imp().last_query.borrow().clone();
                self.run_search(q);
            }
        }
    }

    pub(super) fn apply_cached_file_filter_for_current_scope(&self) -> bool {
        let imp = self.imp();
        let filter = imp.filter_state.borrow().clone();
        let query = imp.last_query.borrow().to_lowercase();
        let cache = imp.file_filter_results_cache.borrow().clone();
        let Some(cache) = cache.filter(|cache| cache.filter == filter && cache.query == query)
        else {
            return false;
        };

        let level = *imp.timeline_level.borrow();
        let selected_year = imp.selected_year.get();
        let selected_month = imp.selected_month.get();
        let requested_scope = match level {
            TimelineLevel::Commits if selected_year > 0 && selected_month > 0 => {
                Some((selected_year, selected_month))
            }
            TimelineLevel::Months | TimelineLevel::Commits if selected_year > 0 => {
                Some((selected_year, 0))
            }
            _ => None,
        };
        let cache_covers_scope = match (cache.scope, requested_scope) {
            (None, _) => true,
            (Some((cached_year, 0)), Some((year, _))) => cached_year == year,
            (Some(cached), Some(requested)) => cached == requested,
            (Some(_), None) => false,
        };
        if !cache_covers_scope {
            return false;
        }

        let commits = imp.all_commits.borrow();
        let scoped_indices = cache
            .result_indices
            .iter()
            .copied()
            .filter(|index| {
                commits.get(*index).is_some_and(|commit| match level {
                    TimelineLevel::Years => true,
                    TimelineLevel::Months if selected_year > 0 => {
                        year_from_timestamp(commit.timestamp) == Some(selected_year)
                    }
                    TimelineLevel::Commits if selected_year > 0 && selected_month > 0 => {
                        year_month_from_timestamp(commit.timestamp)
                            == Some((selected_year, selected_month))
                    }
                    TimelineLevel::Commits if selected_year > 0 => {
                        year_from_timestamp(commit.timestamp) == Some(selected_year)
                    }
                    _ => true,
                })
            })
            .collect::<Vec<_>>();

        let scoped_counts = scoped_indices
            .iter()
            .filter_map(|index| {
                let commit = commits.get(*index)?;
                cache
                    .file_match_counts
                    .get(&commit.oid())
                    .map(|count| (commit.oid(), *count))
            })
            .collect::<HashMap<_, _>>();

        if level == TimelineLevel::Commits {
            let refs = scoped_indices
                .iter()
                .filter_map(|index| commits.get(*index))
                .collect::<Vec<_>>();
            commit_controller::populate_commit_list_refs(&imp.commit_list, &refs);
        }
        drop(commits);
        self.update_file_filter_timeline(&scoped_indices, &scoped_counts);
        true
    }

    pub(super) fn update_file_filter_timeline(
        &self,
        result_indices: &[usize],
        file_match_counts: &HashMap<git2::Oid, usize>,
    ) {
        let imp = self.imp();
        let commits = imp.all_commits.borrow();
        let selected_year = imp.selected_year.get();
        let selected_month = imp.selected_month.get();
        let level = *imp.timeline_level.borrow();
        let total_files: usize = file_match_counts.values().sum();

        if selected_year > 0 && selected_month > 0 && level == TimelineLevel::Commits {
            imp.timeline_header_title.set_subtitle(&format!(
                "{} {} · {} file(s) · {} commit(s)",
                timeline_filter::month_name(selected_month),
                selected_year,
                total_files,
                result_indices.len(),
            ));
            return;
        }

        if selected_year > 0 {
            let mut month_counts: Vec<(u32, usize)> = Vec::new();

            for index in result_indices {
                let Some(commit) = commits.get(*index) else {
                    continue;
                };
                let Some((year, month)) = year_month_from_timestamp(commit.timestamp) else {
                    continue;
                };

                if year != selected_year {
                    continue;
                }

                let count = file_match_counts.get(&commit.oid()).copied().unwrap_or(0);
                if count == 0 {
                    continue;
                }

                if let Some((_, existing)) = month_counts.iter_mut().find(|(m, _)| *m == month) {
                    *existing += count;
                } else {
                    month_counts.push((month, count));
                }
            }

            month_counts.sort_unstable_by(|a, b| b.0.cmp(&a.0));
            imp.month_list.remove_all();
            for (month, count) in month_counts {
                imp.month_list
                    .append(&commit_controller::build_month_row(month, count));
            }

            imp.timeline_header_title.set_subtitle(&format!(
                "{} file(s) · {} commit(s)",
                total_files,
                result_indices.len()
            ));
            return;
        }

        let mut year_counts: Vec<(i32, usize)> = Vec::new();

        for index in result_indices {
            let Some(commit) = commits.get(*index) else {
                continue;
            };
            let Some(year) = year_from_timestamp(commit.timestamp) else {
                continue;
            };

            let count = file_match_counts.get(&commit.oid()).copied().unwrap_or(0);
            if count == 0 {
                continue;
            }

            if let Some((_, existing)) = year_counts.iter_mut().find(|(y, _)| *y == year) {
                *existing += count;
            } else {
                year_counts.push((year, count));
            }
        }

        year_counts.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        imp.year_list.remove_all();
        for (year, count) in year_counts.iter() {
            imp.year_list
                .append(&commit_controller::build_year_row(*year, *count));
        }

        *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
        imp.selected_year.set(0);
        imp.selected_month.set(0);
        imp.timeline_stack.set_visible_child_name("years");
        imp.timeline_back_button.set_visible(false);
        imp.timeline_header_title.set_title(&gettext("Timeline"));
        imp.timeline_header_title
            .set_subtitle(&format!("{} file(s)", total_files));
    }

    pub(super) fn on_year_selected(&self, year: i32) {
        let imp = self.imp();
        imp.selected_year.set(year);
        imp.selected_month.set(0);
        imp.month_list.remove_all();
        imp.commit_list.remove_all();

        let commits = imp.all_commits.borrow();
        let visible = self.visible_timeline_commits(&commits);
        let visible_owned: Vec<CommitInfo> = visible.iter().map(|c| (*c).clone()).collect();

        // Populate the month drill-down list for the selected year, respecting
        // the active author/date filters.
        for (month, count) in &timeline_filter::months_for_year(&visible_owned, year) {
            imp.month_list
                .append(&commit_controller::build_month_row(*month, *count));
        }

        // Also filter the visible commit list to the whole selected year.
        let filtered = timeline_filter::commits_for_year(&visible_owned, year);
        commit_controller::populate_commit_list_refs(&imp.commit_list, &filtered);

        *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
        imp.timeline_stack.set_visible_child_name("months");
        imp.timeline_back_button.set_visible(true);
        imp.timeline_header_title.set_title(&year.to_string());
        imp.timeline_header_title
            .set_subtitle(&format!("{} commit(s)", filtered.len()));

        drop(commits);
        self.reapply_active_file_filter();
    }

    pub(super) fn on_month_selected(&self, month: u32) {
        let imp = self.imp();
        let year = imp.selected_year.get();
        imp.selected_month.set(month);
        imp.commit_list.remove_all();

        let commits = imp.all_commits.borrow();
        let visible = self.visible_timeline_commits(&commits);
        let visible_owned: Vec<CommitInfo> = visible.iter().map(|c| (*c).clone()).collect();

        let filtered = timeline_filter::commits_for_month(&visible_owned, year, month);
        commit_controller::populate_commit_list_refs(&imp.commit_list, &filtered);

        *imp.timeline_level.borrow_mut() = TimelineLevel::Commits;
        imp.timeline_stack.set_visible_child_name("commits");
        imp.timeline_header_title.set_subtitle(&format!(
            "{} {} · {} commit(s)",
            timeline_filter::month_name(month),
            year,
            filtered.len(),
        ));

        drop(commits);
        self.reapply_active_file_filter();
    }

    pub(super) fn timeline_pop(&self) {
        let imp = self.imp();
        let level = *imp.timeline_level.borrow();
        match level {
            TimelineLevel::Commits => {
                if imp.selected_year.get() > 0 {
                    imp.selected_month.set(0);
                    *imp.timeline_level.borrow_mut() = TimelineLevel::Months;
                    imp.timeline_stack.set_visible_child_name("months");
                    imp.timeline_header_title.set_subtitle("");
                } else {
                    imp.selected_month.set(0);
                    *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
                    imp.timeline_stack.set_visible_child_name("years");
                    imp.timeline_back_button.set_visible(false);
                    imp.timeline_header_title.set_title(&gettext("Timeline"));
                    imp.timeline_header_title.set_subtitle("");
                }
            }
            TimelineLevel::Months => {
                imp.selected_month.set(0);
                *imp.timeline_level.borrow_mut() = TimelineLevel::Years;
                imp.timeline_stack.set_visible_child_name("years");
                imp.timeline_back_button.set_visible(false);
                imp.timeline_header_title.set_title(&gettext("Timeline"));
                imp.timeline_header_title.set_subtitle("");
                imp.selected_year.set(0);
            }
            TimelineLevel::Years => {}
        }
    }

    pub(super) fn on_commit_selected(&self, hash: String) {
        let imp = self.imp();

        self.push_commit_nav(&hash);

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

        // Lazily populate changed_files for the selected commit so that
        // FilterTypesDialog and any future diff-aware UI have the data ready.
        // load_changed_files is idempotent — this is a no-op if already loaded.
        {
            let repo_path = imp.repo_path.borrow().clone();
            if let Some(path) = repo_path {
                if let Ok(repo) = git2::Repository::open(&path) {
                    let mut commits = imp.all_commits.borrow_mut();
                    let index = imp.commit_index.borrow();

                    let commit = index.get(&hash).and_then(|idx| commits.get_mut(*idx));

                    if let Some(commit) = commit {
                        commit.load_changed_files(&repo);
                        imp.changed_files_cache
                            .borrow_mut()
                            .insert(commit.hash.clone(), commit.changed_files.clone());
                    } else if let Some(commit) = commits
                        .iter_mut()
                        .find(|c| c.hash.starts_with(short_hash(&hash)))
                    {
                        commit.load_changed_files(&repo);
                        imp.changed_files_cache
                            .borrow_mut()
                            .insert(commit.hash.clone(), commit.changed_files.clone());
                    }
                }
            }
        }

        self.try_show_merge_conflict_dialog(&hash);
        self.navigate_to_dir(PathBuf::new());
    }

    pub(super) fn refresh_timeline_after_filter_change(&self) {
        let imp = self.imp();
        let level = *imp.timeline_level.borrow();
        let selected_year = imp.selected_year.get();
        let selected_month = imp.selected_month.get();

        match level {
            TimelineLevel::Years => {
                self.populate_year_list();
            }
            TimelineLevel::Months | TimelineLevel::Commits => {
                if selected_year > 0 {
                    let commits = imp.all_commits.borrow();
                    let visible = self.visible_timeline_commits(&commits);
                    let visible_owned: Vec<CommitInfo> =
                        visible.iter().map(|c| (*c).clone()).collect();

                    let year_has_commits =
                        !timeline_filter::commits_for_year(&visible_owned, selected_year)
                            .is_empty();
                    let month_has_commits = level == TimelineLevel::Commits
                        && selected_month > 0
                        && !timeline_filter::commits_for_month(
                            &visible_owned,
                            selected_year,
                            selected_month,
                        )
                        .is_empty();

                    drop(commits);

                    if month_has_commits {
                        self.on_year_selected(selected_year);
                        self.on_month_selected(selected_month);
                    } else if year_has_commits {
                        self.on_year_selected(selected_year);
                    } else {
                        self.populate_year_list();
                    }
                } else {
                    self.populate_year_list();
                }
            }
        }
    }

    pub(super) fn setup_filter_popover(&self) {
        let imp = self.imp();
        let popover = SearchFilterPopover::new();

        // The popover refines the commit search in the timeline sidebar, so it
        // belongs to the filter button beside that SearchEntry.
        let filter_button = imp.filter_button.get();

        popover.set_parent(&filter_button);

        filter_button.connect_toggled({
            let pop = popover.clone();
            move |btn| {
                if btn.is_active() {
                    pop.popup();
                } else {
                    pop.popdown();
                }
            }
        });

        popover.connect_closed({
            let btn = filter_button.downgrade();
            move |_| {
                if let Some(b) = btn.upgrade() {
                    b.set_active(false);
                }
            }
        });

        let win = self.clone();
        popover.connect_filters_changed(move |popover_ref| {
            let state = popover_ref.filter_state();
            *win.imp().filter_state.borrow_mut() = state;
            win.update_search_filter_indicators();

            // Rebuild the timeline sidebar from the active FilterState.
            // Without this, changing Author/Date only updated the commit search
            // result list and left Years/Months stale.
            win.refresh_timeline_after_filter_change();

            let q = win.imp().last_query.borrow().clone();
            let file_filter_reapplied_by_timeline_selection = {
                let imp = win.imp();
                imp.filter_state.borrow().files.is_active() && imp.selected_year.get() > 0
            };

            if !file_filter_reapplied_by_timeline_selection {
                win.run_search(q);
            }
        });

        *imp.filter_popover.borrow_mut() = Some(popover);
    }

    pub(super) fn on_search_changed(&self, query: String) {
        *self.imp().last_query.borrow_mut() = query.clone();

        let trimmed = query.trim().to_string();
        let has_query = !trimmed.is_empty();
        if self.imp().toolbar.search_button().is_active() != has_query {
            self.imp().toolbar.search_button().set_active(has_query);
        }
        self.update_search_filter_indicators();

        if trimmed.is_empty() {
            // Search cleared: restore the normal timeline view using the already
            // loaded in-memory commit history. Do not reload the repository.
            if let Some(prev) = self.imp().search_debounce.borrow_mut().take() {
                prev.store(true, Ordering::Relaxed);
            }

            self.refresh_timeline_after_filter_change();
            return;
        }

        {
            let imp = self.imp();
            *imp.timeline_level.borrow_mut() = TimelineLevel::Commits;
            imp.selected_year.set(0);
            imp.selected_month.set(0);
            imp.timeline_stack.set_visible_child_name("commits");
            imp.timeline_back_button.set_visible(true);
            imp.timeline_header_title
                .set_title(&gettext("Search Results"));
            imp.timeline_header_title.set_subtitle("");
        }

        // Cancel any in-flight debounce.
        if let Some(prev) = self.imp().search_debounce.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }

        let token = Arc::new(AtomicBool::new(false));
        *self.imp().search_debounce.borrow_mut() = Some(token.clone());

        let win = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
            if token.load(Ordering::Relaxed) {
                return;
            }
            win.run_search(query);
        });
    }

    pub(super) fn update_search_filter_indicators(&self) {
        let imp = self.imp();
        let has_query = !imp.commit_search_entry.text().trim().is_empty();
        let has_filter = imp.filter_state.borrow().is_active();

        if has_query {
            imp.toolbar.search_button().add_css_class("accent");
        } else {
            imp.toolbar.search_button().remove_css_class("accent");
        }

        if has_filter {
            imp.filter_button.add_css_class("accent");
        } else {
            imp.filter_button.remove_css_class("accent");
        }
    }

    pub fn run_search(&self, query: String) {
        // Cancel any previous search before starting a new worker.
        if let Some(prev) = self.imp().search_cancel.borrow_mut().take() {
            prev.store(true, Ordering::Relaxed);
        }

        let cancel = Arc::new(AtomicBool::new(false));
        *self.imp().search_cancel.borrow_mut() = Some(cancel.clone());

        let filter = self.imp().filter_state.borrow().clone();
        let repo_path = self.imp().repo_path.borrow().clone();
        let branch_hashes = filter
            .branch
            .as_ref()
            .and_then(|branch| self.imp().branch_commit_index.borrow().get(branch).cloned());
        let q = query.to_lowercase();
        let file_filter_active = filter.files.is_active();
        let file_filter_active_for_ui = file_filter_active;
        let (include_folders, wanted_extensions) = filter.files.textual_match_terms();
        let file_filter_scope = if file_filter_active {
            let imp = self.imp();
            let level = *imp.timeline_level.borrow();
            let selected_year = imp.selected_year.get();
            let selected_month = imp.selected_month.get();

            match level {
                TimelineLevel::Commits if selected_year > 0 && selected_month > 0 => {
                    Some((selected_year, selected_month))
                }
                TimelineLevel::Months | TimelineLevel::Commits if selected_year > 0 => {
                    Some((selected_year, 0))
                }
                _ => None,
            }
        } else {
            None
        };
        let candidates: Vec<(usize, git2::Oid)> = {
            let commits = self.imp().all_commits.borrow();
            commits
                .iter()
                .enumerate()
                .filter(|(_, commit)| {
                    if let Some((scope_year, scope_month)) = file_filter_scope {
                        match year_month_from_timestamp(commit.timestamp) {
                            Some((year, month))
                                if year == scope_year
                                    && (scope_month == 0 || month == scope_month) => {}
                            _ => return false,
                        }
                    }

                    commit_matches_lightweight_filters(commit, &filter, branch_hashes.as_ref(), &q)
                })
                .map(|(index, commit)| (index, commit.oid()))
                .collect()
        };

        let file_index_snapshot = if file_filter_active {
            self.imp()
                .file_index_store
                .borrow()
                .as_ref()
                .map(FileIndexStore::snapshot)
                .unwrap_or_default()
        } else {
            HashMap::new()
        };

        let progress_dialog = if file_filter_active {
            let dialog = OperationProgressDialog::new();
            dialog.setup(
                &gettext("Indexing Changed Files"),
                &gettext("Reading changed-file metadata…"),
                cancel.clone(),
            );
            AdwDialogExt::present(&dialog, Some(self.upcast_ref::<gtk::Widget>()));
            Some(dialog)
        } else {
            None
        };

        let total_commits = candidates.len().max(1);
        let (tx, rx) = std::sync::mpsc::sync_channel::<SearchProgressMessage>(32);
        let worker_cancel = cancel.clone();
        let completed_filter = filter.clone();
        let completed_query = q.clone();
        let completed_scope = file_filter_scope;

        std::thread::spawn(move || {
            let mut result_indices = Vec::new();
            let mut newly_indexed_files: Vec<(git2::Oid, CommitFileIndex)> = Vec::new();
            let mut file_match_counts: HashMap<git2::Oid, usize> = HashMap::new();
            let mut cache_hits = 0usize;
            let mut diff_errors = 0usize;
            let mut indexed = 0usize;

            // For file-type filters, open the repository only once in this worker.
            // Changed files are cached by commit hash for the lifetime of the app.
            let repo_for_file_filter = if filter.files.is_active() {
                match repo_path.as_ref() {
                    Some(path) => match git2::Repository::open(path) {
                        Ok(repo) => Some(repo),
                        Err(e) => {
                            let _ = tx.send(SearchProgressMessage::Error(format!(
                                "{}: {e}",
                                gettext("Cannot open repository")
                            )));
                            return;
                        }
                    },
                    None => {
                        let _ = tx.send(SearchProgressMessage::Error(gettext(
                            "No repository loaded",
                        )));
                        return;
                    }
                }
            } else {
                None
            };

            for (candidate_index, oid) in candidates {
                if worker_cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(SearchProgressMessage::Cancelled);
                    return;
                }

                // ── File-type / extension filter ──────────────────────────
                if filter.files.is_active() {
                    let file_index = if let Some(cached_index) = file_index_snapshot.get(&oid) {
                        cache_hits += 1;
                        Arc::clone(cached_index)
                    } else if let Some(ref repo) = repo_for_file_filter {
                        match build_commit_file_index(repo, oid) {
                            Ok(index) => {
                                newly_indexed_files.push((oid, index.clone()));
                                Arc::new(index)
                            }
                            Err(_) => {
                                diff_errors += 1;
                                Arc::new(CommitFileIndex::default())
                            }
                        }
                    } else {
                        Arc::new(CommitFileIndex::default())
                    };

                    indexed += 1;
                    if indexed == 1 || indexed % 25 == 0 || indexed == total_commits {
                        let _ = tx.send(SearchProgressMessage::Progress {
                            indexed,
                            total: total_commits,
                            cache_hits,
                            matches: result_indices.len(),
                        });
                    }

                    let file_match_count =
                        file_index.match_count_textual(include_folders, &wanted_extensions);

                    if file_match_count == 0 {
                        continue;
                    }

                    file_match_counts.insert(oid, file_match_count);
                    result_indices.push(candidate_index);
                    continue;
                }

                result_indices.push(candidate_index);
            }

            let _ = tx.send(SearchProgressMessage::Done(SearchRunResult {
                result_indices,
                newly_indexed_files,
                file_match_counts,
                indexed,
                cache_hits,
                diff_errors,
            }));
        });

        let win = self.clone();
        glib::idle_add_local(move || {
            if cancel.load(Ordering::Relaxed) {
                if let Some(ref dialog) = progress_dialog {
                    dialog.finish_and_close();
                }
                return glib::ControlFlow::Break;
            }

            loop {
                match rx.try_recv() {
                    Ok(SearchProgressMessage::Progress {
                        indexed,
                        total,
                        cache_hits,
                        matches,
                    }) => {
                        if let Some(ref dialog) = progress_dialog {
                            let fraction = indexed as f64 / total.max(1) as f64;
                            dialog.set_progress(
                                fraction,
                                &format!(
                                    "{} {}/{} · {} {} · {} {}",
                                    gettext("Indexed"),
                                    indexed,
                                    total,
                                    cache_hits,
                                    gettext("cached"),
                                    matches,
                                    gettext("matches")
                                ),
                            );
                        }
                    }

                    Ok(SearchProgressMessage::Done(SearchRunResult {
                        result_indices,
                        newly_indexed_files,
                        file_match_counts,
                        indexed,
                        cache_hits,
                        diff_errors,
                    })) => {
                        if let Some(ref dialog) = progress_dialog {
                            dialog.set_progress(
                                1.0,
                                &format!(
                                    "{} {} · {} {}",
                                    gettext("Indexed"),
                                    indexed,
                                    cache_hits,
                                    gettext("cached")
                                ),
                            );
                            dialog.finish_and_close();
                        }

                        if !cancel.load(Ordering::Relaxed) {
                            if file_filter_active_for_ui {
                                *win.imp().file_filter_results_cache.borrow_mut() =
                                    Some(FileFilterResultsCache {
                                        filter: completed_filter.clone(),
                                        query: completed_query.clone(),
                                        scope: completed_scope,
                                        result_indices: result_indices.clone(),
                                        file_match_counts: file_match_counts.clone(),
                                    });
                            } else {
                                win.imp().file_filter_results_cache.borrow_mut().take();
                            }

                            if !newly_indexed_files.is_empty() {
                                if let Some(store) =
                                    win.imp().file_index_store.borrow_mut().as_mut()
                                {
                                    store.insert_many(newly_indexed_files);
                                    let store_to_save = store.clone();
                                    std::thread::spawn(move || {
                                        let _ = store_to_save.save();
                                    });
                                }
                            }

                            {
                                let commits = win.imp().all_commits.borrow();
                                let refs = result_indices
                                    .iter()
                                    .filter_map(|index| commits.get(*index))
                                    .collect::<Vec<_>>();
                                commit_controller::populate_commit_list_refs(
                                    &win.imp().commit_list,
                                    &refs,
                                );
                            }

                            if file_filter_active_for_ui {
                                win.update_file_filter_timeline(
                                    &result_indices,
                                    &file_match_counts,
                                );
                            }

                            if diff_errors > 0 {
                                win.show_toast(&format!(
                                    "{} {} commit(s)",
                                    gettext("Could not read changed files for"),
                                    diff_errors
                                ));
                            }
                        }

                        return glib::ControlFlow::Break;
                    }

                    Ok(SearchProgressMessage::Cancelled) => {
                        if let Some(ref dialog) = progress_dialog {
                            dialog.finish_and_close();
                        }
                        return glib::ControlFlow::Break;
                    }

                    Ok(SearchProgressMessage::Error(message)) => {
                        if let Some(ref dialog) = progress_dialog {
                            dialog.finish_and_close();
                        }
                        win.show_error(&message);
                        return glib::ControlFlow::Break;
                    }

                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        return glib::ControlFlow::Continue;
                    }

                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if let Some(ref dialog) = progress_dialog {
                            dialog.finish_and_close();
                        }
                        return glib::ControlFlow::Break;
                    }
                }
            }
        });
    }
}
