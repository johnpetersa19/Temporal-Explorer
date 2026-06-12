/* timeline_filter.rs
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

//! Pure grouping logic for the temporal navigation sidebar.
//!
//! Converts a flat `&[CommitInfo]` into:
//!
//! * [`years_in_range`]   — sorted list of distinct years (newest first).
//! * [`months_for_year`]  — list of (month_number, commit_count) pairs for
//!                          a specific year, newest first.
//! * [`commits_for_month`]— slice of commits matching a (year, month) pair.
//!
//! All functions are pure (no GTK side-effects) so they can be tested
//! without a display connection.

use gtk::glib;
use gettextrs::gettext;
use crate::git_engine::CommitInfo;

/// Decode a Unix timestamp into (year, month_1_12) in local time.
///
/// Uses `glib::DateTime` so the conversion respects the system timezone
/// exactly as the rest of the UI does.
fn ts_to_ym(ts: i64) -> Option<(i32, u32)> {
    let dt = glib::DateTime::from_unix_local(ts).ok()?;
    Some((dt.year(), dt.month() as u32))
}

/// Returns the distinct years present in `commits`, sorted newest-first.
///
/// Each entry is `(year, commit_count)`.
pub fn years_in_range(commits: &[CommitInfo]) -> Vec<(i32, usize)> {
    let mut map: std::collections::BTreeMap<i32, usize> = std::collections::BTreeMap::new();
    for c in commits {
        if let Some((y, _)) = ts_to_ym(c.timestamp) {
            *map.entry(y).or_insert(0) += 1;
        }
    }
    let mut v: Vec<(i32, usize)> = map.into_iter().collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v
}

/// Returns the distinct months inside `year` for the given commits.
///
/// Each entry is `(month_1_12, commit_count)`, sorted newest-first
/// (month 12 → month 1).
pub fn months_for_year(commits: &[CommitInfo], year: i32) -> Vec<(u32, usize)> {
    let mut map: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for c in commits {
        if let Some((y, m)) = ts_to_ym(c.timestamp) {
            if y == year {
                *map.entry(m).or_insert(0) += 1;
            }
        }
    }
    let mut v: Vec<(u32, usize)> = map.into_iter().collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v
}

/// Returns the commits that fall inside `(year, month)`, in their original
/// order (which is newest-first, as delivered by the git walk).
pub fn commits_for_month(commits: &[CommitInfo], year: i32, month: u32) -> Vec<CommitInfo> {
    commits
        .iter()
        .filter(|c| matches!(ts_to_ym(c.timestamp), Some((y, m)) if y == year && m == month))
        .cloned()
        .collect()
}

/// Human-readable month name — translated via gettext.
///
/// Returns an owned `String` so the translated value lives long enough
/// to be used in GTK label setters.
pub fn month_name(month: u32) -> String {
    match month {
        1  => gettext("January"),
        2  => gettext("February"),
        3  => gettext("March"),
        4  => gettext("April"),
        5  => gettext("May"),
        6  => gettext("June"),
        7  => gettext("July"),
        8  => gettext("August"),
        9  => gettext("September"),
        10 => gettext("October"),
        11 => gettext("November"),
        12 => gettext("December"),
        _  => String::from("?"),
    }
}
