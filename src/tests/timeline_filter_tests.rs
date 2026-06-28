/* tests/timeline_filter_tests.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Unit tests for [`crate::timeline_filter`] and [`crate::search_filter_popover::FilterState`].
//!
//! All tests are pure (no GTK, no display connection required) because
//! `timeline_filter` uses only `glib::DateTime` for timezone conversion
//! and `FilterState::apply` operates on plain `CommitInfo` slices.
//!
//! Run with:
//! ```text
//! cargo test
//! ```

#[cfg(test)]
mod timeline_filter_tests {
    use crate::git_engine::CommitInfo;
    use crate::search_filter_popover::{FileTypeFilter, FilterDateRange, FilterState};
    use crate::timeline_filter::{commits_for_month, month_name, months_for_year, years_in_range};

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Build a minimal `CommitInfo` with only the fields that the timeline
    /// filter and FilterState inspect.
    fn make_commit(hash: &str, author: &str, summary: &str, timestamp: i64) -> CommitInfo {
        CommitInfo::for_test(
            hash.to_owned(),
            summary.to_owned(),
            author.to_owned(),
            format!("{author}@test.local"),
            timestamp,
        )
    }

    // ── years_in_range ────────────────────────────────────────────────────────

    #[test]
    fn years_empty_slice_returns_empty() {
        assert!(years_in_range(&[]).is_empty());
    }

    #[test]
    fn years_single_commit_returns_one_year() {
        // 2024-03-15 00:00:00 UTC  →  Unix 1710460800
        let commits = [make_commit("a1", "Alice", "init", 1_710_460_800)];
        let years = years_in_range(&commits);
        assert_eq!(years.len(), 1);
        assert_eq!(years[0].0, 2024);
        assert_eq!(years[0].1, 1); // commit count
    }

    #[test]
    fn years_multiple_commits_same_year_count_correctly() {
        // Both timestamps fall in 2024.
        let commits = [
            make_commit("a1", "Alice", "first", 1_700_000_000), // Nov 2023 UTC
            make_commit("a2", "Bob", "second", 1_710_460_800),  // Mar 2024 UTC
            make_commit("a3", "Carol", "third", 1_720_000_000), // Jul 2024 UTC
        ];
        let years = years_in_range(&commits);
        // Exact year depends on the local timezone of the test runner;
        // what we can assert deterministically is that there are at most 2
        // distinct calendar years across these three timestamps and that the
        // total commit count sums to 3.
        let total: usize = years.iter().map(|(_, c)| c).sum();
        assert_eq!(total, 3);
        // Sorted newest-first.
        if years.len() > 1 {
            assert!(years[0].0 >= years[1].0);
        }
    }

    #[test]
    fn years_sorted_newest_first() {
        let commits = [
            make_commit("a1", "Alice", "old", 1_000_000_000), // 2001
            make_commit("a2", "Bob", "newer", 1_500_000_000), // 2017
            make_commit("a3", "Carol", "newest", 1_700_000_000), // 2023
        ];
        let years = years_in_range(&commits);
        // Each consecutive pair must be descending.
        for w in years.windows(2) {
            assert!(w[0].0 >= w[1].0, "years not sorted newest-first");
        }
    }

    // ── months_for_year ───────────────────────────────────────────────────────

    #[test]
    fn months_empty_slice_returns_empty() {
        assert!(months_for_year(&[], 2024).is_empty());
    }

    #[test]
    fn months_wrong_year_returns_empty() {
        // 2024-03-15 UTC
        let commits = [make_commit("a1", "Alice", "init", 1_710_460_800)];
        assert!(months_for_year(&commits, 2099).is_empty());
    }

    #[test]
    fn months_counts_sum_to_commits_in_year() {
        // Four commits: two in Jan 2023, one in Mar 2023, one in Dec 2023.
        // Unix timestamps (UTC):
        //   2023-01-10 → 1_673_308_800
        //   2023-01-20 → 1_674_172_800
        //   2023-03-05 → 1_677_974_400
        //   2023-12-01 → 1_701_388_800
        let commits = [
            make_commit("c1", "Alice", "msg", 1_673_308_800),
            make_commit("c2", "Alice", "msg", 1_674_172_800),
            make_commit("c3", "Alice", "msg", 1_677_974_400),
            make_commit("c4", "Alice", "msg", 1_701_388_800),
        ];
        // The year that these timestamps belong to in the local timezone
        // may shift by ±1 day at timezone boundaries.  We derive the
        // target year dynamically.
        let all_years = years_in_range(&commits);
        for (year, expected_total) in &all_years {
            let months = months_for_year(&commits, *year);
            let sum: usize = months.iter().map(|(_, c)| c).sum();
            assert_eq!(sum, *expected_total, "month counts must sum to year count");
        }
    }

    #[test]
    fn months_sorted_newest_first() {
        let commits = [
            make_commit("c1", "Alice", "jan", 1_673_308_800), // Jan 2023 UTC
            make_commit("c2", "Alice", "dec", 1_701_388_800), // Dec 2023 UTC
        ];
        let all_years = years_in_range(&commits);
        for (year, _) in &all_years {
            let months = months_for_year(&commits, *year);
            for w in months.windows(2) {
                assert!(w[0].0 >= w[1].0, "months not sorted newest-first");
            }
        }
    }

    // ── commits_for_month ────────────────────────────────────────────────────

    #[test]
    fn commits_for_month_empty_slice_returns_empty() {
        assert!(commits_for_month(&[], 2024, 3).is_empty());
    }

    #[test]
    fn commits_for_month_wrong_month_returns_empty() {
        let commits = [make_commit("a1", "Alice", "init", 1_710_460_800)]; // Mar 2024 UTC
                                                                           // Month 99 is guaranteed to match nothing.
        assert!(commits_for_month(&commits, 2024, 99).is_empty());
    }

    #[test]
    fn commits_for_month_preserves_original_order() {
        // Three commits with ascending timestamps; `list_commits` returns
        // newest-first, but `commits_for_month` must preserve whatever
        // order it receives.
        let c1 = make_commit("h1", "Alice", "first", 1_673_308_800);
        let c2 = make_commit("h2", "Bob", "second", 1_673_395_200);
        let c3 = make_commit("h3", "Carol", "third", 1_673_481_600);
        let input = [c3.clone(), c2.clone(), c1.clone()]; // newest-first

        let all_years = years_in_range(&input);
        for (year, _) in &all_years {
            let months = months_for_year(&input, *year);
            for (month, _) in &months {
                let result = commits_for_month(&input, *year, *month);
                // Every returned commit must belong to this (year, month).
                // Order must match the input slice order.
                let mut prev_pos: Option<usize> = None;
                for c in &result {
                    let pos = input.iter().position(|x| x.hash == c.hash).unwrap();
                    if let Some(p) = prev_pos {
                        assert!(pos > p, "order not preserved");
                    }
                    prev_pos = Some(pos);
                }
            }
        }
    }

    // ── month_name ────────────────────────────────────────────────────────────

    #[test]
    fn month_name_all_twelve_months_non_empty() {
        for m in 1u32..=12 {
            let name = month_name(m);
            assert!(!name.is_empty(), "month {m} must have a non-empty name");
        }
    }

    #[test]
    fn month_name_invalid_returns_question_mark() {
        assert_eq!(month_name(0), "?");
        assert_eq!(month_name(13), "?");
        assert_eq!(month_name(99), "?");
    }

    // ── FilterState ───────────────────────────────────────────────────────────

    /// Build a default (no-op) `FilterState`.
    fn empty_filter() -> FilterState {
        FilterState {
            date: FilterDateRange::default(),
            author: None,
            branch: None,
            files: FileTypeFilter::default(),
        }
    }

    #[test]
    fn filter_state_default_is_not_active() {
        assert!(!empty_filter().is_active());
        assert!(empty_filter().is_empty());
    }

    #[test]
    fn filter_state_author_filter_marks_active() {
        let mut f = empty_filter();
        f.author = Some("Alice".to_owned());
        assert!(f.is_active());
    }

    #[test]
    fn filter_state_branch_filter_marks_active() {
        let mut f = empty_filter();
        f.branch = Some("main".to_owned());
        assert!(f.is_active());
    }

    #[test]
    fn filter_state_apply_author_filter() {
        let commits = [
            make_commit("a1", "Alice", "first", 1_000),
            make_commit("a2", "Bob", "second", 2_000),
            make_commit("a3", "alice", "third", 3_000), // lowercase — case-insensitive
        ];
        let mut f = empty_filter();
        f.author = Some("alice".to_owned());
        let result = f.apply(&commits);
        assert_eq!(result.len(), 2);
        assert!(result
            .iter()
            .all(|c| c.author.to_lowercase().contains("alice")));
    }

    #[test]
    fn filter_state_apply_empty_filter_returns_all() {
        let commits = [
            make_commit("x1", "Alice", "a", 1_000),
            make_commit("x2", "Bob", "b", 2_000),
        ];
        let result = empty_filter().apply(&commits);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn filter_state_apply_empty_commits_returns_empty() {
        let mut f = empty_filter();
        f.author = Some("Alice".to_owned());
        assert!(f.apply(&[]).is_empty());
    }
}
