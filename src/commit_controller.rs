/* commit_controller.rs
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

//! Commit list controller.
//!
//! Responsible for building GTK widgets that represent commits in the
//! sidebar list and for filtering them.  Extracted from `window.rs` to
//! keep the main window module focused on layout and wiring.
//!
//! # Performance model
//!
//! `gtk::ListBox` keeps **every row as a live widget** in memory, so the
//! number of rows has a direct cost in RAM and layout time.  Two mitigations
//! are in place until a `gtk::ListView` (model/factory) migration lands:
//!
//! * [`MAX_RENDERED_ROWS`]: cap on rows rendered by [`populate_commit_list`].
//!   Rows beyond the cap are not rendered; a hint tells the user to search.
//! * [`HARD_APPEND_CAP`]: safety limit for live-append during background
//!   loading, preventing unbounded growth on kernel-sized repositories.
//!
//! Both caps affect only the **visible widget list** — `all_commits` in
//! `window.rs` always holds the full dataset and search operates on it.
//!
//! Row insertion is always **batched across idle frames** ([`POPULATE_BATCH`] /
//! [`APPEND_BATCH`] rows per frame) so the GTK main loop never stalls.

use gettextrs::gettext;
use gtk::glib;
use gtk::prelude::*;
use crate::git_engine::CommitInfo;

// ── Tuning constants ─────────────────────────────────────────────────────────

/// Rows inserted per idle frame during a **full list rebuild**.
///
/// 150 rows × ~45 µs/row (widget construction + layout) ≈ 6.75 ms —
/// just under the 6.9 ms budget for a 144 Hz display.
/// Increase to 200 on 60 Hz displays if you want faster initial fill;
/// decrease to 80 if construction cost is higher (complex themes).
const POPULATE_BATCH: usize = 150;

/// Rows inserted per idle frame during **live background appending**.
///
/// Smaller than `POPULATE_BATCH` because the list is already visible
/// while streaming, so any per-frame spike is immediately noticeable.
const APPEND_BATCH: usize = 100;

/// Maximum number of widget rows [`populate_commit_list`] will render.
///
/// `gtk::ListBox` allocates a live widget for every child regardless of
/// scroll position.  Beyond ~5 000 rows the memory overhead and layout
/// cost become significant.  When the list is truncated a hint row is
/// appended instructing the user to search.
///
/// The full `all_commits` dataset in `window.rs` is **never** truncated;
/// only the sidebar widget count is capped.
const MAX_RENDERED_ROWS: usize = 5_000;

/// Hard cap on the total number of widget rows during live appending.
///
/// Prevents unbounded widget growth on repositories with tens of thousands
/// of commits (e.g. the Linux kernel).  Appending stops silently once the
/// `gtk::ListBox` child count reaches this value; search still works on the
/// complete in-memory dataset.
const HARD_APPEND_CAP: usize = 15_000;

// ── Row builder ─────────────────────────────────────────────────────────────

/// Builds a [`gtk::ListBoxRow`] that represents a single commit entry.
pub fn build_commit_row(commit: &CommitInfo) -> gtk::ListBoxRow {
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

/// Builds a special "truncated" hint row shown when the list exceeds
/// [`MAX_RENDERED_ROWS`].  The row is not selectable.
fn build_truncation_hint_row(total: usize, rendered: usize) -> gtk::ListBoxRow {
    let hidden = total.saturating_sub(rendered);
    let label = gtk::Label::builder()
        .label(&format!(
            "⚠️ {} {} — {}",
            hidden,
            gettext("commits not shown"),
            gettext("use the search box to filter"),
        ))
        .xalign(0.5)
        .wrap(true)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(12)
        .margin_end(12)
        .build();
    label.add_css_class("caption");
    label.add_css_class("dim-label");

    gtk::ListBoxRow::builder()
        .selectable(false)
        .activatable(false)
        .child(&label)
        .build()
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Populates `list_box` with rows for each commit in `commits`.
///
/// All existing children are removed before inserting new rows.
///
/// # Performance
///
/// * Rows are inserted in idle callbacks batched at [`POPULATE_BATCH`]
///   entries per frame, keeping the GTK main loop responsive.
/// * At most [`MAX_RENDERED_ROWS`] widget rows are created.  If `commits`
///   is larger a truncation hint row is appended so the user knows to
///   search.  The search path in `window.rs` always operates on the full
///   `all_commits` in-memory dataset — not on the widget list.
pub fn populate_commit_list(list_box: &gtk::ListBox, commits: &[CommitInfo]) {
    // Remove all existing rows first.
    while let Some(child) = list_box.first_child() {
        child.unparent();
    }

    if commits.is_empty() {
        return;
    }

    let total = commits.len();
    let render_count = total.min(MAX_RENDERED_ROWS);
    let truncated = total > MAX_RENDERED_ROWS;

    // Fast path: tiny list, single synchronous pass.
    if render_count <= POPULATE_BATCH {
        for commit in &commits[..render_count] {
            list_box.append(&build_commit_row(commit));
        }
        if truncated {
            list_box.append(&build_truncation_hint_row(total, render_count));
        }
        return;
    }

    // Slow path: schedule batches over multiple idle frames.
    let owned: Vec<CommitInfo> = commits[..render_count].to_vec();
    let list_weak = list_box.downgrade();
    let remaining = std::rc::Rc::new(std::cell::RefCell::new(owned));

    schedule_batch_populate(list_weak, remaining, total, truncated);
}

/// Appends a batch of new commits to `list_box` WITHOUT clearing existing rows.
///
/// Used by the background pagination loop in `window.rs` to stream commits
/// into the sidebar as they arrive from the background thread.
///
/// # Performance
///
/// * Insertion is spread over idle frames at [`APPEND_BATCH`] rows/frame
///   so the list is always responsive during live loading.
/// * Appending stops once the widget count reaches [`HARD_APPEND_CAP`]
///   to prevent unbounded memory growth.  The full commit dataset in
///   `window.rs` is not affected — search remains complete.
pub fn append_commit_batch(list_box: &gtk::ListBox, commits: &[CommitInfo]) {
    if commits.is_empty() {
        return;
    }

    // Count current children to honour HARD_APPEND_CAP.
    // gtk::ListBox does not expose a child count directly; we track it by
    // iterating once.  This is O(n) but called at most once per 500-commit
    // page, so the amortised cost is negligible.
    let current_count = {
        let mut n = 0usize;
        let mut child = list_box.first_child();
        while child.is_some() {
            n += 1;
            child = child.and_then(|w| w.next_sibling());
        }
        n
    };

    if current_count >= HARD_APPEND_CAP {
        // Widget cap reached — stop appending rows silently.
        // The caller (window.rs) still stores commits in `all_commits`.
        return;
    }

    // How many more rows may we append?
    let headroom = HARD_APPEND_CAP.saturating_sub(current_count);
    let to_append: Vec<CommitInfo> = commits.iter().take(headroom).cloned().collect();

    if to_append.is_empty() {
        return;
    }

    // Fast path: fits in one batch.
    if to_append.len() <= APPEND_BATCH {
        for commit in &to_append {
            list_box.append(&build_commit_row(commit));
        }
        return;
    }

    // Slow path: stream over idle frames.
    let list_weak = list_box.downgrade();
    let remaining = std::rc::Rc::new(std::cell::RefCell::new(to_append));
    schedule_batch_append(list_weak, remaining);
}

// ── Internal idle-batch helpers ───────────────────────────────────────────────

/// Drives the idle-batch loop for [`populate_commit_list`].
/// Appends [`POPULATE_BATCH`] rows per frame; when exhausted, optionally
/// appends the truncation hint.
fn schedule_batch_populate(
    list_weak: glib::object::WeakRef<gtk::ListBox>,
    remaining: std::rc::Rc<std::cell::RefCell<Vec<CommitInfo>>>,
    total: usize,
    truncated: bool,
) {
    glib::idle_add_local_once(move || {
        let Some(list_box) = list_weak.upgrade() else { return };
        let mut rem = remaining.borrow_mut();
        let end = POPULATE_BATCH.min(rem.len());
        for commit in rem.drain(..end) {
            list_box.append(&build_commit_row(&commit));
        }
        let still_pending = !rem.is_empty();
        drop(rem);

        if still_pending {
            schedule_batch_populate(list_weak, remaining.clone(), total, truncated);
        } else if truncated {
            // All rendered rows are done; now append the hint.
            list_box.append(&build_truncation_hint_row(total, MAX_RENDERED_ROWS));
        }
    });
}

/// Drives the idle-batch loop for [`append_commit_batch`].
/// Appends [`APPEND_BATCH`] rows per frame.
fn schedule_batch_append(
    list_weak: glib::object::WeakRef<gtk::ListBox>,
    remaining: std::rc::Rc<std::cell::RefCell<Vec<CommitInfo>>>,
) {
    glib::idle_add_local_once(move || {
        let Some(list_box) = list_weak.upgrade() else { return };
        let mut rem = remaining.borrow_mut();
        let end = APPEND_BATCH.min(rem.len());
        for commit in rem.drain(..end) {
            list_box.append(&build_commit_row(&commit));
        }
        let still_pending = !rem.is_empty();
        drop(rem);
        if still_pending {
            schedule_batch_append(list_weak, remaining.clone());
        }
    });
}

// ── Search / filter helpers ───────────────────────────────────────────────────

/// Filters `commits` by `query` (case-insensitive match on summary, hash
/// prefix, or author).
///
/// Kept as public API for future use (export, clipboard copy, etc.).
/// The interactive search path in `window.rs` runs this logic off-thread
/// with cancellation support.
#[allow(dead_code)]
pub fn filter_commits<'a>(commits: &'a [CommitInfo], query: &str) -> Vec<&'a CommitInfo> {
    if query.is_empty() {
        return commits.iter().collect();
    }
    let q = query.to_lowercase();
    commits
        .iter()
        .filter(|c| {
            c.summary.to_lowercase().contains(&q)
                || c.hash.starts_with(query)
                || c.author.to_lowercase().contains(&q)
        })
        .collect()
}

/// Returns the human-readable item-count subtitle shown in the header.
pub fn item_count_subtitle(n: usize) -> String {
    if n == 1 {
        gettext("1 item")
    } else {
        format!("{n} {}", gettext("items"))
    }
}
