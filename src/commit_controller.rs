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
//! sidebar list and for filtering them. Extracted from `window.rs` to
//! keep the main window module focused on layout and wiring.

use gettextrs::gettext;
use gtk::prelude::*;
use crate::git_engine::CommitInfo;

/// Builds a [`gtk::ListBoxRow`] that represents a single commit entry.
///
/// The row contains:
/// - A summary label (first line of the commit message).
/// - A meta label with the abbreviated hash and author name.
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
pub fn populate_commit_list(list_box: &gtk::ListBox, commits: &[CommitInfo]) {
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }
    for commit in commits {
        list_box.append(&build_commit_row(commit));
    }
}

/// Filters `commits` by `query` (case-insensitive match on summary, hash
/// prefix, or author).
///
/// Returns the full slice unchanged when `query` is empty.
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
        // translators: {n} is the number of items
        format!("{n} {}", gettext("items"))
    }
}
