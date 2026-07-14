/* tests/git_engine_tests.rs
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

//! Unit tests for [`crate::git_engine`].
//!
//! Every test builds a fully self-contained, in-memory Git repository using
//! [`git2`] alone — no GTK, no filesystem fixtures on disk (temp dirs are
//! created by [`tempfile`] when a physical path is required by git2).
//!
//! Run with:
//! ```text
//! cargo test
//! ```
//!
//! The test module is gated behind `#[cfg(test)]` so it compiles only when
//! running tests and does not affect the release binary.

#[cfg(test)]
mod git_engine_tests {
    use git2::{Repository, Signature, Time};
    use std::path::{Path, PathBuf};

    use crate::git_engine::{
        CommitFileIndex, CommitInfo, DirCache, HistoryReader, SnapshotMaterializer,
        SnapshotResolver, TreeNode, FILE_CATEGORY_DOCUMENTS, FILE_CATEGORY_FOLDERS,
        FILE_CATEGORY_IMAGES, FILE_CATEGORY_TEXT,
    };

    // ── Fixture helpers ───────────────────────────────────────────────────────
    //
    // These helpers build minimal but realistic in-memory git2 repos.
    // They are free functions (not methods) so any test can call them
    // without boilerplate.

    /// Creates a git signature with deterministic values.
    fn sig(name: &str, email: &str, ts: i64) -> Signature<'static> {
        Signature::new(name, email, &Time::new(ts, 0)).unwrap()
    }

    /// Initialises a bare, in-memory-style repo in a fresh temp directory.
    /// Returns `(tempdir, repo)` — keep `tempdir` alive for the test duration.
    fn init_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo = Repository::init(dir.path()).expect("git init");
        // Suppress the initial-branch warning by setting the config.
        repo.config()
            .unwrap()
            .set_str("init.defaultBranch", "main")
            .ok();
        (dir, repo)
    }

    /// Writes `files` (path → content) into the repository index and commits
    /// them.  Returns the new commit OID.
    ///
    /// If `parent_oid` is `None` this is treated as the root (initial) commit.
    fn commit_files(
        repo: &Repository,
        files: &[(&str, &[u8])],
        message: &str,
        author_name: &str,
        ts: i64,
        parent_oid: Option<git2::Oid>,
    ) -> git2::Oid {
        let mut index = repo.index().unwrap();

        for (rel_path, content) in files {
            let blob_oid = repo.blob(content).unwrap();
            let entry = git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode: 0o100644,
                uid: 0,
                gid: 0,
                file_size: content.len() as u32,
                id: blob_oid,
                flags: 0,
                flags_extended: 0,
                path: rel_path.as_bytes().to_vec(),
            };
            index.add(&entry).unwrap();
        }

        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        let author = sig(author_name, &format!("{author_name}@test.local"), ts);
        let committer = sig("CI", "ci@test.local", ts);

        let parents: Vec<git2::Commit<'_>> = parent_oid
            .map(|oid| vec![repo.find_commit(oid).unwrap()])
            .unwrap_or_default();
        let parent_refs: Vec<&git2::Commit<'_>> = parents.iter().collect();

        repo.commit(
            Some("HEAD"),
            &author,
            &committer,
            message,
            &tree,
            &parent_refs,
        )
        .unwrap()
    }

    // ── TreeNode helpers ──────────────────────────────────────────────────────

    #[test]
    fn tree_node_file_path_and_is_dir() {
        let node = TreeNode::File(PathBuf::from("src/main.rs"));
        assert_eq!(node.path(), Path::new("src/main.rs"));
        assert!(!node.is_dir());
    }

    #[test]
    fn tree_node_dir_path_and_is_dir() {
        let node = TreeNode::Dir(PathBuf::from("src"));
        assert_eq!(node.path(), Path::new("src"));
        assert!(node.is_dir());
    }

    // ── DirCache ──────────────────────────────────────────────────────────────

    #[test]
    fn dir_cache_miss_returns_none() {
        let mut cache = DirCache::new();
        assert!(cache.get("abc123", Path::new("")).is_none());
    }

    #[test]
    fn dir_cache_insert_then_hit() {
        let mut cache = DirCache::new();
        let nodes = vec![TreeNode::File(PathBuf::from("README.md"))];
        cache.insert("abc".into(), PathBuf::new(), nodes);
        let result = cache.get("abc", Path::new(""));
        assert!(result.is_some());
        let arc = result.unwrap();
        assert_eq!(arc.len(), 1);
    }

    #[test]
    fn dir_cache_hit_promotes_to_front() {
        let mut cache = DirCache::new();
        // Insert two entries; then access the first one so it moves to front.
        cache.insert(
            "h1".into(),
            PathBuf::new(),
            vec![TreeNode::Dir(PathBuf::from("a"))],
        );
        cache.insert(
            "h2".into(),
            PathBuf::new(),
            vec![TreeNode::Dir(PathBuf::from("b"))],
        );
        // Access "h1" — it should become MRU.
        cache.get("h1", Path::new(""));
        // Both entries should still be retrievable.
        assert!(cache.get("h1", Path::new("")).is_some());
        assert!(cache.get("h2", Path::new("")).is_some());
    }

    #[test]
    fn dir_cache_insert_replaces_existing_key() {
        let mut cache = DirCache::new();
        cache.insert(
            "h1".into(),
            PathBuf::new(),
            vec![TreeNode::File(PathBuf::from("old.txt"))],
        );
        cache.insert(
            "h1".into(),
            PathBuf::new(),
            vec![
                TreeNode::File(PathBuf::from("new1.txt")),
                TreeNode::File(PathBuf::from("new2.txt")),
            ],
        );
        let arc = cache.get("h1", Path::new("")).unwrap();
        // Should see the new value, not the old one.
        assert_eq!(arc.len(), 2);
    }

    #[test]
    fn dir_cache_evicts_lru_when_full() {
        // DIR_CACHE_MAX_ENTRIES is 64; fill it to capacity and then add one more.
        let mut cache = DirCache::new();
        for i in 0..64usize {
            cache.insert(format!("h{i}"), PathBuf::new(), vec![]);
        }
        // "h0" was inserted first — it is the LRU entry.
        // Adding a 33rd entry should evict "h0".
        cache.insert("h_new".into(), PathBuf::new(), vec![]);
        assert!(
            cache.get("h0", Path::new("")).is_none(),
            "LRU entry h0 should have been evicted"
        );
        assert!(
            cache.get("h_new", Path::new("")).is_some(),
            "newest entry should still be present"
        );
    }

    #[test]
    fn dir_cache_clear_empties_all_entries() {
        let mut cache = DirCache::new();
        cache.insert("h1".into(), PathBuf::new(), vec![]);
        cache.insert("h2".into(), PathBuf::new(), vec![]);
        cache.clear();
        assert!(cache.get("h1", Path::new("")).is_none());
        assert!(cache.get("h2", Path::new("")).is_none());
    }

    // ── HistoryReader — single commit ─────────────────────────────────────────

    #[test]
    fn history_reader_list_commits_single() {
        let (_dir, repo) = init_repo();
        commit_files(
            &repo,
            &[("README.md", b"hello")],
            "Initial commit",
            "Alice",
            1_700_000_000,
            None,
        );

        let reader = HistoryReader::open(_dir.path()).unwrap();
        let commits = reader.list_commits().unwrap();
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].summary, "Initial commit");
        assert_eq!(commits[0].author, "Alice");
        assert_eq!(commits[0].timestamp, 1_700_000_000);
        // Hash must be a valid 40-char hex string.
        assert_eq!(commits[0].hash.len(), 40);
    }

    #[test]
    fn history_reader_list_commits_ordered_newest_first() {
        let (_dir, repo) = init_repo();
        let c1 = commit_files(&repo, &[("a.txt", b"a")], "First", "Alice", 1_000, None);
        let c2 = commit_files(&repo, &[("b.txt", b"b")], "Second", "Bob", 2_000, Some(c1));
        commit_files(&repo, &[("c.txt", b"c")], "Third", "Carol", 3_000, Some(c2));

        let reader = HistoryReader::open(_dir.path()).unwrap();
        let commits = reader.list_commits().unwrap();
        assert_eq!(commits.len(), 3);
        // Newest first (timestamp descending).
        assert_eq!(commits[0].summary, "Third");
        assert_eq!(commits[1].summary, "Second");
        assert_eq!(commits[2].summary, "First");
    }

    // ── HistoryReader — paginated ─────────────────────────────────────────────

    #[test]
    fn list_commits_paginated_all_pages_collected() {
        let (_dir, repo) = init_repo();
        // Build 7 commits.
        let mut prev = None;
        for i in 0..7usize {
            prev = Some(commit_files(
                &repo,
                &[(&format!("f{i}.txt"), b"x")],
                &format!("commit {i}"),
                "Dev",
                i as i64 * 100,
                prev,
            ));
        }

        let reader = HistoryReader::open(_dir.path()).unwrap();
        let mut all: Vec<CommitInfo> = Vec::new();
        let mut page_count = 0usize;
        reader
            .list_commits_paginated(3, |page| {
                page_count += 1;
                all.extend(page);
            })
            .unwrap();

        // 7 commits with page_size=3 → 3 pages (3+3+1).
        assert_eq!(all.len(), 7, "should collect all 7 commits");
        assert_eq!(page_count, 3, "should deliver exactly 3 pages");
        // Still ordered newest-first.
        assert_eq!(all[0].summary, "commit 6");
        assert_eq!(all[6].summary, "commit 0");
    }

    #[test]
    fn list_commits_paginated_exact_multiple() {
        let (_dir, repo) = init_repo();
        let mut prev = None;
        for i in 0..6usize {
            prev = Some(commit_files(
                &repo,
                &[(&format!("f{i}.txt"), b"x")],
                &format!("msg {i}"),
                "Dev",
                i as i64,
                prev,
            ));
        }
        let reader = HistoryReader::open(_dir.path()).unwrap();
        let mut pages = 0usize;
        let mut total = 0usize;
        reader
            .list_commits_paginated(2, |page| {
                pages += 1;
                total += page.len();
            })
            .unwrap();
        assert_eq!(total, 6);
        assert_eq!(pages, 3); // 6 / 2 = 3 full pages, no remainder.
    }

    // ── HistoryReader — search ────────────────────────────────────────────────

    #[test]
    fn search_commits_by_summary_case_insensitive() {
        let (_dir, repo) = init_repo();
        let c1 = commit_files(&repo, &[("a.txt", b"a")], "Fix login bug", "Alice", 1, None);
        commit_files(
            &repo,
            &[("b.txt", b"b")],
            "Add dark mode",
            "Bob",
            2,
            Some(c1),
        );

        let reader = HistoryReader::open(_dir.path()).unwrap();
        let results = reader.search_commits("LOGIN").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].summary, "Fix login bug");
    }

    #[test]
    fn search_commits_by_author() {
        let (_dir, repo) = init_repo();
        let c1 = commit_files(&repo, &[("a.txt", b"x")], "Alpha", "Alice", 1, None);
        commit_files(&repo, &[("b.txt", b"y")], "Beta", "Bob", 2, Some(c1));

        let reader = HistoryReader::open(_dir.path()).unwrap();
        let results = reader.search_commits("bob").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].author, "Bob");
    }

    #[test]
    fn search_commits_by_hash_prefix() {
        let (_dir, repo) = init_repo();
        commit_files(&repo, &[("x.txt", b"x")], "Only commit", "Dev", 1, None);

        let reader = HistoryReader::open(_dir.path()).unwrap();
        let all = reader.list_commits().unwrap();
        let prefix = &all[0].hash[..8];

        let results = reader.search_commits(prefix).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].hash, all[0].hash);
    }

    #[test]
    fn search_commits_empty_query_returns_all() {
        let (_dir, repo) = init_repo();
        let c1 = commit_files(&repo, &[("a.txt", b"a")], "One", "Dev", 1, None);
        commit_files(&repo, &[("b.txt", b"b")], "Two", "Dev", 2, Some(c1));

        let reader = HistoryReader::open(_dir.path()).unwrap();
        let results = reader.search_commits("").unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn search_commits_no_match_returns_empty() {
        let (_dir, repo) = init_repo();
        commit_files(&repo, &[("a.txt", b"a")], "Initial", "Dev", 1, None);

        let reader = HistoryReader::open(_dir.path()).unwrap();
        let results = reader.search_commits("zzz_no_match_zzz").unwrap();
        assert!(results.is_empty());
    }

    // ── SnapshotResolver::resolve_dir ─────────────────────────────────────────

    #[test]
    fn resolve_dir_root_lists_files_and_dirs() {
        let (_dir, repo) = init_repo();
        commit_files(
            &repo,
            &[
                ("README.md", b"readme"),
                ("src/main.rs", b"fn main() {}"),
                ("src/lib.rs", b"// lib"),
            ],
            "Init",
            "Dev",
            1,
            None,
        );

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let hash = head.id().to_string();
        let resolver = SnapshotResolver::new(&repo);

        let nodes = resolver.resolve_dir(&hash, Path::new("")).unwrap();
        // Root should contain: README.md (file) + src (dir)
        assert_eq!(nodes.len(), 2, "root should have 2 entries");

        let dirs: Vec<_> = nodes.iter().filter(|n| n.is_dir()).collect();
        let files: Vec<_> = nodes.iter().filter(|n| !n.is_dir()).collect();
        assert_eq!(dirs.len(), 1);
        assert_eq!(files.len(), 1);
        assert_eq!(dirs[0].path(), Path::new("src"));
        assert_eq!(files[0].path(), Path::new("README.md"));
    }

    #[test]
    fn resolve_dir_subdirectory_lists_children() {
        let (_dir, repo) = init_repo();
        commit_files(
            &repo,
            &[("src/main.rs", b"fn main() {}"), ("src/lib.rs", b"// lib")],
            "Init",
            "Dev",
            1,
            None,
        );

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let hash = head.id().to_string();
        let resolver = SnapshotResolver::new(&repo);

        let nodes = resolver.resolve_dir(&hash, Path::new("src")).unwrap();
        assert_eq!(nodes.len(), 2);

        let mut names: Vec<&str> = nodes
            .iter()
            .map(|n| n.path().file_name().unwrap().to_str().unwrap())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["lib.rs", "main.rs"]);
    }

    #[test]
    fn resolve_dir_invalid_revision_returns_error() {
        let (_dir, repo) = init_repo();
        commit_files(&repo, &[("a.txt", b"a")], "Init", "Dev", 1, None);

        let resolver = SnapshotResolver::new(&repo);
        let result = resolver.resolve_dir("deadbeefdeadbeef", Path::new(""));
        assert!(result.is_err(), "invalid revision should return Err");
    }

    #[test]
    fn resolve_dir_nonexistent_path_returns_error() {
        let (_dir, repo) = init_repo();
        commit_files(&repo, &[("a.txt", b"a")], "Init", "Dev", 1, None);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let hash = head.id().to_string();
        let resolver = SnapshotResolver::new(&repo);
        let result = resolver.resolve_dir(&hash, Path::new("no_such_dir"));
        assert!(result.is_err(), "missing path should return Err");
    }

    // ── SnapshotMaterializer::materialize ─────────────────────────────────────

    #[test]
    fn materializer_flat_tree() {
        let (_dir, repo) = init_repo();
        commit_files(
            &repo,
            &[("a.txt", b"aaa"), ("b.txt", b"bbb")],
            "Flat",
            "Dev",
            1,
            None,
        );

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let tree = head.tree().unwrap();
        let materializer = SnapshotMaterializer::new(&repo);
        let nodes = materializer
            .materialize(&tree, PathBuf::new(), 0, usize::MAX)
            .unwrap();

        assert_eq!(nodes.len(), 2);
        assert!(nodes.iter().all(|n| !n.is_dir()));
    }

    #[test]
    fn materializer_nested_tree_includes_dir_and_children() {
        let (_dir, repo) = init_repo();
        commit_files(
            &repo,
            &[
                ("README.md", b"r"),
                ("src/main.rs", b"m"),
                ("src/utils/helper.rs", b"h"),
            ],
            "Nested",
            "Dev",
            1,
            None,
        );

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let tree = head.tree().unwrap();
        let materializer = SnapshotMaterializer::new(&repo);
        let nodes = materializer
            .materialize(&tree, PathBuf::new(), 0, usize::MAX)
            .unwrap();

        // Expected: README.md, src (dir), src/main.rs, src/utils (dir), src/utils/helper.rs
        assert_eq!(nodes.len(), 5);

        let dirs: Vec<_> = nodes.iter().filter(|n| n.is_dir()).collect();
        let files: Vec<_> = nodes.iter().filter(|n| !n.is_dir()).collect();
        assert_eq!(dirs.len(), 2);
        assert_eq!(files.len(), 3);
    }

    // ── SnapshotMaterializer::read_file ───────────────────────────────────────

    #[test]
    fn read_file_returns_correct_content() {
        let (_dir, repo) = init_repo();
        let expected = b"Hello, world!";
        commit_files(
            &repo,
            &[("greet.txt", expected)],
            "Add greeting",
            "Dev",
            1,
            None,
        );

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let hash = head.id().to_string();
        let materializer = SnapshotMaterializer::new(&repo);
        let content = materializer
            .read_file(&hash, Path::new("greet.txt"))
            .unwrap();
        assert_eq!(content, expected);
    }

    #[test]
    fn read_file_missing_path_returns_error() {
        let (_dir, repo) = init_repo();
        commit_files(&repo, &[("a.txt", b"a")], "Init", "Dev", 1, None);

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let hash = head.id().to_string();
        let materializer = SnapshotMaterializer::new(&repo);
        let result = materializer.read_file(&hash, Path::new("missing.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn read_file_binary_content_roundtrip() {
        let (_dir, repo) = init_repo();
        // A minimal 4-byte "binary" payload.
        let binary: &[u8] = &[0x00, 0xFF, 0x1B, 0x42];
        commit_files(
            &repo,
            &[("data.bin", binary)],
            "Binary file",
            "Dev",
            1,
            None,
        );

        let head = repo.head().unwrap().peel_to_commit().unwrap();
        let hash = head.id().to_string();
        let materializer = SnapshotMaterializer::new(&repo);
        let content = materializer
            .read_file(&hash, Path::new("data.bin"))
            .unwrap();
        assert_eq!(content, binary);
    }

    // ── CommitInfo field correctness ──────────────────────────────────────────

    #[test]
    fn commit_info_fields_match_git_commit() {
        let (_dir, repo) = init_repo();
        let oid = commit_files(
            &repo,
            &[("x.txt", b"x")],
            "Detailed commit message",
            "Test Author",
            9_999_999,
            None,
        );

        let reader = HistoryReader::open(_dir.path()).unwrap();
        let commits = reader.list_commits().unwrap();
        assert_eq!(commits.len(), 1);

        let c = &commits[0];
        assert_eq!(c.hash, oid.to_string());
        assert_eq!(c.summary, "Detailed commit message");
        assert_eq!(c.author, "Test Author");
        assert_eq!(c.timestamp, 9_999_999);
    }

    #[test]
    fn commit_info_hash_is_40_hex_chars() {
        let (_dir, repo) = init_repo();
        commit_files(&repo, &[("f.txt", b"f")], "Msg", "Dev", 1, None);

        let reader = HistoryReader::open(_dir.path()).unwrap();
        let commits = reader.list_commits().unwrap();
        let hash = &commits[0].hash;
        assert_eq!(hash.len(), 40);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn commit_info_matches_text_query_without_allocating() {
        let info = CommitInfo::for_test(
            "abcd1234".to_string(),
            "Fix search path".to_string(),
            "Jane Doe".to_string(),
            "Jane@Example.com".to_string(),
            42,
        );

        assert!(info.matches_text_query("fix"));
        assert!(info.matches_text_query("jane@example.com"));
        assert!(info.author_matches_query("jane"));
        assert!(!info.matches_text_query("missing"));
    }

    #[test]
    fn commit_file_index_compacts_and_matches_categories() {
        let index = CommitFileIndex::from_paths(&[
            "docs/readme.md".to_string(),
            "src/lib.rs".to_string(),
            "assets/logo.png".to_string(),
            "notes.txt".to_string(),
        ]);

        assert_eq!(index.match_count(FILE_CATEGORY_TEXT, None), 3);
        assert_eq!(index.match_count(FILE_CATEGORY_IMAGES, None), 1);
        assert_eq!(index.match_count(FILE_CATEGORY_DOCUMENTS, None), 0);
        assert_eq!(index.match_count(FILE_CATEGORY_FOLDERS, None), 3);
        assert_eq!(index.match_count(0, Some("md")), 1);
    }
}
