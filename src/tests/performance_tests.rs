#[cfg(test)]
mod performance_tests {
    use crate::git_engine::HistoryReader;
    use std::path::PathBuf;
    use std::time::Instant;

    /// Manual, reproducible engine benchmark.
    ///
    /// Run against a large repository with:
    /// `TEMPORAL_BENCH_REPO=/path/to/repo cargo test --release engine_baseline -- --ignored --nocapture`
    #[test]
    #[ignore = "manual performance benchmark"]
    fn engine_baseline() {
        let path = std::env::var_os("TEMPORAL_BENCH_REPO")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let reader = HistoryReader::open(&path).expect("open benchmark repository");

        let started = Instant::now();
        let mut first_page = None;
        let mut commits = Vec::new();
        reader
            .list_commits_paginated(500, |page| {
                first_page.get_or_insert_with(|| started.elapsed());
                commits.extend(page);
            })
            .expect("walk commit history");
        let history_elapsed = started.elapsed();

        let diff_started = Instant::now();
        let repo = git2::Repository::open(&path).expect("reopen benchmark repository");
        for commit in &mut commits {
            commit
                .load_changed_files_result(&repo)
                .expect("load changed files");
        }
        let diff_elapsed = diff_started.elapsed();

        eprintln!(
            "ENGINE_BENCH commits={} first_page_ms={} history_ms={} changed_files_ms={}",
            commits.len(),
            first_page.unwrap_or_default().as_millis(),
            history_elapsed.as_millis(),
            diff_elapsed.as_millis(),
        );
    }
}
