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
//! * [`years_in_range`]    — sorted list of distinct years (newest first).
//! * [`months_for_year`]   — list of (month_number, commit_count) pairs for
//!                           a specific year, newest first.
//! * [`commits_for_month`] — borrowed slice of commits matching a
//!                           (year, month) pair.  Returns `Vec<&CommitInfo>`
//!                           so callers avoid cloning the full `CommitInfo`
//!                           (including its `changed_files: Vec<String>`).
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
///
/// `#[inline]` is important here: this is called inside a tight loop over
/// every commit in [`years_in_range`] and [`months_for_year`], and the
/// compiler can then fold the `DateTime` construction into the loop body.
#[inline]
fn ts_to_ym(ts: i64) -> Option<(i32, u32)> {
    let dt = glib::DateTime::from_unix_local(ts).ok()?;
    Some((dt.year(), dt.month() as u32))
}

/// Returns the distinct years present in `commits`, sorted newest-first.
///
/// Each entry is `(year, commit_count)`.
///
/// Uses a plain `Vec` accumulator instead of a `BTreeMap` — the ordered-map
/// property was never used (the result was re-sorted anyway), so a Vec +
/// sort is cheaper: no heap tree allocation, no per-insert rebalancing.
pub fn years_in_range(commits: &[CommitInfo]) -> Vec<(i32, usize)> {
    // Collect (year, 1) pairs, then fold duplicates.
    let mut v: Vec<(i32, usize)> = Vec::new();
    for c in commits {
        if let Some((y, _)) = ts_to_ym(c.timestamp) {
            // Linear scan is fine: the number of distinct years is tiny
            // (typical repo: < 20 years), so this is faster than a HashMap.
            if let Some(entry) = v.iter_mut().find(|(yr, _)| *yr == y) {
                entry.1 += 1;
            } else {
                v.push((y, 1));
            }
        }
    }
    v.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    v
}

/// Returns the distinct months inside `year` for the given commits.
///
/// Each entry is `(month_1_12, commit_count)`, sorted newest-first
/// (month 12 → month 1).
///
/// Uses a fixed `[usize; 13]` stack array instead of a `BTreeMap`:
/// months are always in `1..=12`, so index arithmetic replaces heap
/// allocation entirely.  Index 0 is unused (months are 1-based).
pub fn months_for_year(commits: &[CommitInfo], year: i32) -> Vec<(u32, usize)> {
    let mut counts = [0usize; 13]; // index 0 unused; 1..=12 are the months
    for c in commits {
        if let Some((y, m)) = ts_to_ym(c.timestamp) {
            if y == year {
                counts[m as usize] += 1;
            }
        }
    }
    // Emit only non-zero months, newest (12) first.
    let mut v: Vec<(u32, usize)> = (1u32..=12)
        .filter(|&m| counts[m as usize] > 0)
        .map(|m| (m, counts[m as usize]))
        .collect();
    v.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    v
}

/// Returns **references** to the commits that fall inside `(year, month)`,
/// in their original order (newest-first, as delivered by the git walk).
///
/// Returning `Vec<&CommitInfo>` instead of `Vec<CommitInfo>` avoids cloning
/// `CommitInfo` — in particular its `changed_files: Vec<String>` field —
/// on every sidebar month selection.  All call sites only need to read the
/// commits, not own them.
pub fn commits_for_month<'a>(
    commits: &'a [CommitInfo],
    year: i32,
    month: u32,
) -> Vec<&'a CommitInfo> {
    commits
        .iter()
        .filter(|c| matches!(ts_to_ym(c.timestamp), Some((y, m)) if y == year && m == month))
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
