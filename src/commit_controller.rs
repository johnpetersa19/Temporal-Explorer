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
//! * `HARD_APPEND_CAP`: safety limit for live-append during background
//!   loading, preventing unbounded growth on kernel-sized repositories.
//!
//! Both caps affect only the **visible widget list** — `all_commits` in
//! `window.rs` always holds the full dataset and search operates on it.
//!
//! Row insertion is always **batched across idle frames** ([`POPULATE_BATCH`] /
//! `APPEND_BATCH` rows per frame) so the GTK main loop never stalls.
//!
//! # Stale-batch cancellation
//!
//! Each call to [`populate_commit_list`] increments an **`AtomicU64` generation
//! counter** stored as `GObject` data on the `ListBox` itself. Every idle
//! frame checks that its captured generation still matches the counter before
//! touching any widget. If the counter has advanced (because a new populate
//! call cleared and rebuilt the list), the frame becomes a no-op and exits.
//! This prevents GTK use-after-free crashes (`gtk_widget_insert_after`
//! assertion failures) that occur when stale batches try to append to a
//! `ListBox` whose children were already `unparent()`-ed by a later call.

use gettextrs::gettext;
use gtk::glib;
use gtk::prelude::*;
use crate::git_engine::CommitInfo;
use crate::timeline_filter;
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

// ── Tuning constants ────────────────────────────────────────────────────────

/// Rows inserted per idle frame during a **full list rebuild**.
const POPULATE_BATCH: usize = 150;

/// Rows inserted per idle frame during **live background appending**.
/// Reserved for the future `gtk::ListView` migration.
#[allow(dead_code)]
const APPEND_BATCH: usize = 100;

/// Maximum number of widget rows [`populate_commit_list`] will render.
const MAX_RENDERED_ROWS: usize = 5_000;

/// Hard cap on the total number of widget rows during live appending.
/// Reserved for the future `gtk::ListView` migration.
#[allow(dead_code)]
const HARD_APPEND_CAP: usize = 15_000;

// ── Generation counter helpers ────────────────────────────────────────────
//
// We store an Arc<AtomicU64> in a thread-local keyed by the ListBox pointer
// address. This avoids GObject qdata FFI entirely.

use std::collections::HashMap;
use std::cell::RefCell;

thread_local! {
    /// Maps ListBox raw pointer address → generation counter.
    static GENERATIONS: RefCell<HashMap<usize, Arc<AtomicU64>>> = RefCell::new(HashMap::new());
}

/// Returns the generation counter for `list_box`, creating it if absent.
fn get_or_create_generation(list_box: &gtk::ListBox) -> Arc<AtomicU64> {
    let key = list_box.as_ptr() as usize;
    GENERATIONS.with(|map| {
        map.borrow_mut()
            .entry(key)
            .or_insert_with(|| Arc::new(AtomicU64::new(0)))
            .clone()
    })
}

/// Increments the generation for `list_box` and returns the new value.
/// Call this at the start of every populate to invalidate pending batches.
fn next_generation(list_box: &gtk::ListBox) -> u64 {
    let counter = get_or_create_generation(list_box);
    counter.fetch_add(1, Ordering::Relaxed) + 1
}

// ── Row builders ──────────────────────────────────────────────────────

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

/// Builds a row representing a **year** in the timeline drill-down.
pub fn build_year_row(year: i32, count: usize) -> gtk::ListBoxRow {
    let hbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(16)
        .margin_end(12)
        .build();

    let label = gtk::Label::builder()
        .label(&year.to_string())
        .xalign(0.0)
        .hexpand(true)
        .build();
    label.add_css_class("heading");

    let badge = gtk::Label::builder()
        .label(&count.to_string())
        .xalign(1.0)
        .build();
    badge.add_css_class("caption");
    badge.add_css_class("dim-label");

    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    chevron.set_pixel_size(12);
    chevron.set_margin_start(4);
    chevron.add_css_class("dim-label");

    hbox.append(&label);
    hbox.append(&badge);
    hbox.append(&chevron);

    gtk::ListBoxRow::builder()
        .name(&year.to_string())
        .child(&hbox)
        .build()
}

/// Builds a row representing a **month** inside a selected year.
pub fn build_month_row(month: u32, count: usize) -> gtk::ListBoxRow {
    let hbox = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .margin_top(8)
        .margin_bottom(8)
        .margin_start(16)
        .margin_end(12)
        .build();

    let label = gtk::Label::builder()
        .label(timeline_filter::month_name(month))
        .xalign(0.0)
        .hexpand(true)
        .build();

    let badge = gtk::Label::builder()
        .label(&count.to_string())
        .xalign(1.0)
        .build();
    badge.add_css_class("caption");
    badge.add_css_class("dim-label");

    let chevron = gtk::Image::from_icon_name("go-next-symbolic");
    chevron.set_pixel_size(12);
    chevron.set_margin_start(4);
    chevron.add_css_class("dim-label");

    hbox.append(&label);
    hbox.append(&badge);
    hbox.append(&chevron);

    gtk::ListBoxRow::builder()
        .name(&month.to_string())
        .child(&hbox)
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

// ── Public API ───────────────────────────────────────────────────────────────

/// Populates `list_box` with rows for each commit in `commits`.
///
/// Increments the generation counter on `list_box` so that any idle frames
/// still running from a previous call will detect the stale generation and
/// abort, preventing use-after-free crashes in GTK widget internals.
pub fn populate_commit_list(list_box: &gtk::ListBox, commits: &[CommitInfo]) {
    // Advance generation — invalidates all pending idle batches for this list.
    let gen = next_generation(list_box);
    let gen_counter = get_or_create_generation(list_box);

    while let Some(child) = list_box.first_child() {
        child.unparent();
    }

    if commits.is_empty() { return; }

    let total = commits.len();
    let render_count = total.min(MAX_RENDERED_ROWS);
    let truncated = total > MAX_RENDERED_ROWS;

    if render_count <= POPULATE_BATCH {
        for commit in &commits[..render_count] {
            list_box.append(&build_commit_row(commit));
        }
        if truncated {
            list_box.append(&build_truncation_hint_row(total, render_count));
        }
        return;
    }

    let owned: Vec<CommitInfo> = commits[..render_count].to_vec();
    let list_weak = list_box.downgrade();
    let remaining = std::rc::Rc::new(std::cell::RefCell::new(owned));
    schedule_batch_populate(list_weak, remaining, total, truncated, gen, gen_counter);
}

/// Populates `list_box` with year rows derived from `commits`.
pub fn populate_year_list(list_box: &gtk::ListBox, commits: &[CommitInfo]) {
    while let Some(child) = list_box.first_child() {
        child.unparent();
    }
    for (year, count) in timeline_filter::years_in_range(commits) {
        list_box.append(&build_year_row(year, count));
    }
}

/// Populates `list_box` with month rows for `year` derived from `commits`.
pub fn populate_month_list(list_box: &gtk::ListBox, commits: &[CommitInfo], year: i32) {
    while let Some(child) = list_box.first_child() {
        child.unparent();
    }
    for (month, count) in timeline_filter::months_for_year(commits, year) {
        list_box.append(&build_month_row(month, count));
    }
}

/// Appends a batch of new commits to `list_box` WITHOUT clearing existing rows.
///
/// Reserved for the future `gtk::ListView` migration.
#[allow(dead_code)]
pub fn append_commit_batch(list_box: &gtk::ListBox, commits: &[CommitInfo]) {
    if commits.is_empty() { return; }

    let current_count = {
        let mut n = 0usize;
        let mut child = list_box.first_child();
        while child.is_some() {
            n += 1;
            child = child.and_then(|w| w.next_sibling());
        }
        n
    };

    if current_count >= HARD_APPEND_CAP { return; }

    let headroom = HARD_APPEND_CAP.saturating_sub(current_count);
    let to_append: Vec<CommitInfo> = commits.iter().take(headroom).cloned().collect();

    if to_append.is_empty() { return; }

    if to_append.len() <= APPEND_BATCH {
        for commit in &to_append {
            list_box.append(&build_commit_row(commit));
        }
        return;
    }

    let list_weak = list_box.downgrade();
    let remaining = std::rc::Rc::new(std::cell::RefCell::new(to_append));
    schedule_batch_append(list_weak, remaining);
}

// ── Internal idle-batch helpers ──────────────────────────────────────────────

fn schedule_batch_populate(
    list_weak: glib::object::WeakRef<gtk::ListBox>,
    remaining: std::rc::Rc<std::cell::RefCell<Vec<CommitInfo>>>,
    total: usize,
    truncated: bool,
    gen: u64,
    gen_counter: Arc<AtomicU64>,
) {
    glib::idle_add_local_once(move || {
        // Abort if a newer populate call has superseded this batch.
        if gen_counter.load(Ordering::Relaxed) != gen {
            return;
        }

        let Some(list_box) = list_weak.upgrade() else { return };

        let mut rem = remaining.borrow_mut();
        let end = POPULATE_BATCH.min(rem.len());
        for commit in rem.drain(..end) {
            list_box.append(&build_commit_row(&commit));
        }
        let still_pending = !rem.is_empty();
        drop(rem);

        if still_pending {
            schedule_batch_populate(list_weak, remaining.clone(), total, truncated, gen, gen_counter);
        } else if truncated {
            list_box.append(&build_truncation_hint_row(total, MAX_RENDERED_ROWS));
        }
    });
}

/// Reserved for the future `gtk::ListView` migration.
#[allow(dead_code)]
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

// ── Search / filter helpers ───────────────────────────────────────────────

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
