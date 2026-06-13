/* search_filter_popover.rs
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

//! `SearchFilterPopover` — advanced search/filter panel.
//!
//! Inspired by `nautilus-search-popover` (Nautilus 47+), adapted for
//! Git-commit exploration:
//!
//! * **Date** — quick chips (Today / Yesterday / Past Week / Month / Year)
//!              plus a "…" button that opens `DateRangeDialog`.
//! * **Author** — free-text entry *plus* auto-generated chips from
//!                the unique authors in the current commit list.
//! * **Branch** — chips populated from `git branch -a` on repo open.
//! * **Changed files** — chip toggles for Rust / TOML / Blueprint / other.
//!
//! The popover emits a `filters-changed` signal carrying a `FilterState`
//! struct.  `window.rs` listens to it and re-runs `run_search()` with the
//! active filter applied on top of the text query.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gettextrs::gettext;
use std::cell::RefCell;

use crate::git_engine::CommitInfo;

// ── FilterDateRange ────────────────────────────────────────────────────────────

/// A half-open date range `[from, to)` expressed as Unix timestamps.
/// Both ends are optional; `None` means "unbounded".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterDateRange {
    pub from: Option<i64>,
    pub to:   Option<i64>,
}

impl FilterDateRange {
    /// Returns `true` when at least one bound is set.
    pub fn is_active(&self) -> bool {
        self.from.is_some() || self.to.is_some()
    }

    /// Returns `true` when `ts` falls inside the range.
    pub fn contains(&self, ts: i64) -> bool {
        let after  = self.from.map_or(true, |f| ts >= f);
        let before = self.to.map_or(true,   |t| ts <  t);
        after && before
    }

    // ── Preset constructors ────────────────────────────────────────────────

    pub fn today() -> Self {
        let now = glib::DateTime::now_local().unwrap();
        let start = glib::DateTime::new(
            &glib::TimeZone::local(),
            now.year(), now.month(), now.day_of_month(),
            0, 0, 0.0,
        ).unwrap();
        let end = start.add_days(1).unwrap();
        Self { from: Some(start.to_unix()), to: Some(end.to_unix()) }
    }

    pub fn yesterday() -> Self {
        let now = glib::DateTime::now_local().unwrap();
        let start = glib::DateTime::new(
            &glib::TimeZone::local(),
            now.year(), now.month(), now.day_of_month(),
            0, 0, 0.0,
        ).unwrap().add_days(-1).unwrap();
        let end = start.add_days(1).unwrap();
        Self { from: Some(start.to_unix()), to: Some(end.to_unix()) }
    }

    pub fn last_n_days(n: i32) -> Self {
        let now = glib::DateTime::now_local().unwrap();
        let start = now.add_days(-n).unwrap();
        Self { from: Some(start.to_unix()), to: None }
    }
}

// ── FileTypeFilter ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileTypeFilter {
    pub rust:      bool,
    pub toml:      bool,
    pub blueprint: bool,
    pub other_ext: Option<String>,
}

impl FileTypeFilter {
    pub fn is_active(&self) -> bool {
        self.rust || self.toml || self.blueprint || self.other_ext.is_some()
    }
}

// ── FilterState ────────────────────────────────────────────────────────────────

/// All active filter constraints from the popover.
/// `window.rs` applies this on top of the free-text query in `run_search()`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterState {
    pub date:   FilterDateRange,
    pub author: Option<String>,
    pub branch: Option<String>,
    pub files:  FileTypeFilter,
}

impl FilterState {
    /// `true` when at least one filter is active.
    pub fn is_active(&self) -> bool {
        self.date.is_active()
            || self.author.is_some()
            || self.branch.is_some()
            || self.files.is_active()