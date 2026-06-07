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

use gettextrs::gettext;
use gtk::glib;
use gtk::prelude::*;
use crate::git_engine::CommitInfo;

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

/// Populates `list_box` with rows for each commit in `commits`.
///
/// All existing children are removed before inserting new rows.
///
/// PERF: Rows are inserted in idle callbacks batched at `BATCH_SIZE`
/// entries per frame so the GTK main loop stays responsive even when
/// rebuilding the sidebar with 10k+ filtered results.
pub fn populate_commit_list(list_box: &gtk::ListBox, commits: &[CommitInfo]) {
    // Remove all existing rows first.
    while let Some(child) = list_box.first_child() {
        child.unparent();
    }

    if commits.is_empty() {
        return;
    }

    const BATCH_SIZE: usize = 500;

    // Fast path: small lists fit in a single synchronous pass.
    if commits.len() <= BATCH_SIZE {
        for commit in commits {
            list_box.append(&build_commit_row(commit));
        }
        return;
    }

    // Large lists: schedule batches over multiple idle iterations.
    let owned: Vec<CommitInfo> = commits.to_vec();
    let list_weak = list_box.downgrade();
    let remaining = std::rc::Rc::new(std::cell::RefCell::new(owned));

    schedule_batch(list_weak, remaining);
}

/// Schedules one idle callback that inserts up to `BATCH_SIZE` rows,
/// then re-schedules itself if more rows remain.
fn schedule_batch(
    list_weak: glib::object::WeakRef<gtk::ListBox>,
    remaining: std::rc::Rc<std::cell::RefCell<Vec<CommitInfo>>>,
) {
    const BATCH_SIZE: usize = 500;

    glib::idle_add_local_once(move || {
        let Some(list_box) = list_weak.upgrade() else { return };
        let mut rem = remaining.borrow_mut();
        let end = BATCH_SIZE.min(rem.len());
        for commit in rem.drain(..end) {
            list_box.append(&build_commit_row(&commit));
        }
        let still_pending = !rem.is_empty();
        drop(rem);
        if still_pending {
            schedule_batch(list_weak, remaining.clone());
        }
    });
}

/// Appends a batch of new commits to `list_box` WITHOUT clearing existing rows.
pub fn append_commit_batch(list_box: &gtk::ListBox, commits: &[CommitInfo]) {
    for commit in commits {
        list_box.append(&build_commit_row(commit));
    }
}

/// Filters `commits` by `query` (case-insensitive match on summary, hash
/// prefix, or author).
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
