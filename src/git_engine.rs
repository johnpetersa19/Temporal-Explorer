/* git_engine.rs
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

//! Git engine: reads history, resolves snapshots and materializes file trees.
//!
//! # Module layout
//!
//! - [`CpuPool`]              – detects available CPU cores and derives thread counts.
//! - [`HistoryReader`]        – opens a repository and iterates commits.
//! - [`CommitInfo`]           – lightweight commit data for the UI list.
//! - [`SnapshotResolver`]     – resolves a commit hash into a full tree.
//! - [`SnapshotMaterializer`] – converts the resolved tree into [`TreeNode`]s.
//! - [`TreeNode`]             – a single file, directory, or submodule in a snapshot.
//! - [`DirCache`]             – LRU cache of (hash, dir) → Arc<Vec<TreeNode>>.
//! - [`SubmoduleInfo`]        – metadata for a discovered git submodule.
//! - [`detect_submodules`]    – inspects submodules from an open repository handle.
//! - [`detect_submodules_at`] – convenience wrapper that opens the repo by path.
//!
//! # Threading model
//!
//! `git2::Repository` is **not** `Send`, so each background thread that needs
//! to access the object database must open its own `Repository` handle.
//! The parallel commit path probes how many OIDs exist (up to
//! `MIN_COMMITS_FOR_PARALLEL + 1`) to decide whether to use threads, then
//! shards them across `CpuPool::io_threads()` worker threads (each opening
//! their own handle), and merges results back in the original sort order
//! before forwarding pages to the caller.  `on_page` is only ever called
//! after all workers have joined successfully — eliminating the duplicate-page
//! bug that would occur if a panic triggered a serial fallback mid-stream.
//!
//! # Submodule model
//!
//! Git records a submodule reference as an `ObjectType::Commit` ("gitlink")
//! entry inside a tree object.  `resolve_dir` emits `TreeNode::Submodule`
//! for those entries so callers can render them with a distinct icon.
//! `detect_submodules` reads `.gitmodules` via git2 (no subprocess) to
//! populate name, URL, and path metadata for each registered submodule.
//!
//! # Security model
//!
//! **Path traversal is structurally impossible in this module.**
//!
//! Every path lookup goes through [`git2::Tree::get_path`], which resolves
//! components inside the Git *object database* — an in-memory tree of OIDs —
//! not through the real filesystem.  Because the object database has no concept
//! of `..` escaping a root, a caller-supplied path such as `"../../etc/passwd"`
//! will simply fail with `git2::Error` ("the path '../../etc/passwd' does not
//! exist in the given tree") instead of opening any file outside the
//! repository.
//!
//! # Accessing `HistoryReader` fields
//!
//! `repo` and `cpu_pool` are private.  Use the public accessor methods
//! `reader.repo()` and `reader.cpu_pool()` — never access the fields directly.

use gettextrs::gettext;
use git2::{ObjectType, Repository, SubmoduleIgnore};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ── Limits ───────────────────────────────────────────────────────────────────

/// Hard cap for [`HistoryReader::list_commits`] (non-paginated).
/// Above this, callers should use [`HistoryReader::list_commits_paginated`].
///
/// Exposed as `pub` so external callers can size pre-allocated buffers
/// without duplicating the magic number.  The compiler warns about
/// "never used" because no call site in this crate references it directly
/// at runtime — the actual enforcement is the `if commits.len() >= LIST_COMMITS_MAX`
/// branch inside `list_commits`.  The constant is kept public as part of
/// the documented API contract.
#[allow(dead_code)]
pub const LIST_COMMITS_MAX: usize = 50_000;

/// Hard cap for a full recursive tree walk in [`SnapshotResolver::resolve_tree`].
/// The limit is enforced *inside* [`SnapshotMaterializer::materialize`] so that
/// the walk is aborted early — before all memory and I/O are consumed.
///
/// Not referenced from outside this module; `#[allow(dead_code)]` suppresses
/// the warning without removing the documented safety bound.
#[allow(dead_code)]
const MAX_FULL_TREE_ENTRIES: usize = 8_000;

/// Maximum recursion depth for [`SnapshotMaterializer::materialize`].
/// Prevents stack overflow on pathologically deep trees.
///
/// Not referenced from outside this module; `#[allow(dead_code)]` suppresses
/// the warning without removing the documented safety bound.
#[allow(dead_code)]
const MAX_TREE_DEPTH: usize = 64;

/// Number of cached directory listings in [`DirCache`].
const DIR_CACHE_MAX_ENTRIES: usize = 64;

/// Maximum number of parallel worker threads for CPU-bound work.
///
/// Referenced only through [`CpuPool::worker_threads`], which is itself
/// currently unused at the call-site level.  Both are preserved as part of
/// the public threading API — see the note on `CpuPool`.
#[allow(dead_code)]
const MAX_WORKER_THREADS: usize = 8;

/// Maximum number of parallel threads for I/O-bound git object-DB access.
const MAX_IO_THREADS: usize = 4;

/// Maximum results returned by [`HistoryReader::search_commits`].
const SEARCH_COMMITS_MAX: usize = 5_000;

/// Minimum number of commits required before the parallel path is preferred
/// over the serial path.  Decoupled from `page_size` so that the parallelism
/// threshold does not vary with the caller's pagination preference.
const MIN_COMMITS_FOR_PARALLEL: usize = 2_000;

// ── Internal helpers ───────────────────────────────────────────────────────────────────

/// Converts a git2 tree entry kind + path into the corresponding [`TreeNode`].
///
/// Returns `None` for entry kinds that are not rendered (e.g. unknown / tag).
/// Centralises the `ObjectType → TreeNode` mapping so that adding a new
/// variant (e.g. symlinks via `ObjectType::Tag` in some git implementations)
/// only requires a change in one place.
#[inline]
fn entry_to_tree_node(kind: Option<ObjectType>, path: PathBuf) -> Option<TreeNode> {
    match kind {
        Some(ObjectType::Blob)   => Some(TreeNode::File(path)),
        Some(ObjectType::Tree)   => Some(TreeNode::Dir(path)),
        Some(ObjectType::Commit) => Some(TreeNode::Submodule(path)),
        _                        => None,
    }
}

/// Pushes HEAD onto a revwalk, gracefully handling empty repositories.
///
/// In a freshly-initialised repository with no commits, `push_head()` returns
/// `Err` with error class `Reference` (code -3, "reference 'refs/heads/...'\
/// not found").  Propagating that error leaves the UI in a broken state
/// (the title bar shows "Loading…" forever) even though the repository itself
/// is perfectly valid — it simply has no history yet.
///
/// This helper absorbs that specific error and returns `Ok(())` instead, so
/// the caller's revwalk iterator yields zero OIDs and the UI renders an
/// empty-repository state cleanly.
#[inline]
fn push_head_safe(walk: &mut git2::Revwalk<'_>, repo: &Repository) -> Result<(), git2::Error> {
    match walk.push_head() {
        Ok(()) => Ok(()),
        Err(e) if e.class() == git2::ErrorClass::Reference => {
            // Empty repository: HEAD exists but points to an unborn branch.
            // Verify by checking whether any commit-like object is reachable.
            if repo.is_empty().unwrap_or(true) {
                Ok(()) // Treat as zero commits — not an error.
            } else {
                Err(e) // Reference error in a non-empty repo is a real problem.
            }
        }
        Err(e) => Err(e),
    }
}

// ── CpuPool ──────────────────────────────────────────────────────────────────────

/// Runtime CPU detection and derived thread-pool sizes.
#[derive(Debug, Clone, Copy)]
pub struct CpuPool {
    logical_cores: usize,
}

impl CpuPool {
    /// Detects available parallelism.  Never panics; falls back to `1`.
    pub fn detect() -> Self {
        let logical_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        Self { logical_cores }
    }

    /// Returns the number of logical CPU cores detected at construction time.
    ///
    /// Currently unused inside this crate — `io_threads` is the active
    /// accessor.  Kept as part of the public `CpuPool` API so external
    /// consumers can inspect core count without re-detecting it.
    #[allow(dead_code)]
    #[inline] pub fn logical_cores(self) -> usize { self.logical_cores }

    /// Returns the recommended number of CPU-bound worker threads
    /// (capped at [`MAX_WORKER_THREADS`]).
    ///
    /// Currently unused inside this crate — the active parallel path calls
    /// `io_threads` instead.  Kept as part of the public threading API for
    /// future CPU-bound work (e.g. diff generation, blame).
    #[allow(dead_code)]
    #[inline] pub fn worker_threads(self) -> usize {
        self.logical_cores.min(MAX_WORKER_THREADS)
    }

    #[inline] pub fn io_threads(self) -> usize {
        self.logical_cores.min(MAX_IO_THREADS)
    }

    #[inline] pub fn is_parallel(self) -> bool { self.logical_cores > 1 }
}

impl Default for CpuPool {
    fn default() -> Self { Self::detect() }
}

// ── CommitInfo ───────────────────────────────────────────────────────────────────────────

/// Lightweight representation of a single commit, used to populate the UI list.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    /// Full 40-character SHA-1 hash.
    pub hash: String,
    /// First line of the commit message (summary).
    pub summary: String,
    /// Author name.
    pub author: String,
    /// Author e-mail address.  Useful for deduplicating authors with different
    /// display names and for generating Gravatar avatar URLs.
    pub author_email: String,
    /// Unix timestamp (seconds since epoch).
    pub timestamp: i64,
    /// Changed files in this commit.
    pub changed_files: Vec<String>,
}

impl CommitInfo {
    fn from_commit(commit: &git2::Commit<'_>, repo: &git2::Repository) -> Self {
        let mut changed_files = Vec::new();
        if let Ok(tree) = commit.tree() {
            if commit.parent_count() > 0 {
                // For both regular commits (1 parent) and merge commits (N > 1
                // parents), diff only against the **first parent**.
                //
                // Rationale: diffing against every parent of a merge commit
                // causes files that exist only in parent[i] to appear as
                // "modified" in the diff parent[j] → merge_tree for all j ≠ i,
                // producing false positives in the extension filter.  The
                // sort/dedup below can remove exact duplicates but cannot
                // eliminate the semantically incorrect entries.
                //
                // Using only parent[0] matches the behaviour of `git show` and
                // `git log -p`, which is what users expect when inspecting a
                // merge commit's changed files.
                if let Ok(parent) = commit.parent(0) {
                    if let Ok(parent_tree) = parent.tree() {
                        if let Ok(diff) = repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), None) {
                            let _ = diff.foreach(
                                &mut |delta, _| {
                                    if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                                        changed_files.push(path.to_owned());
                                    }
                                    true
                                },
                                None,
                                None,
                                None,
                            );
                        }
                    }
                }
            } else {
                if let Ok(diff) = repo.diff_tree_to_tree(None, Some(&tree), None) {
                    let _ = diff.foreach(
                        &mut |delta, _| {
                            if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                                changed_files.push(path.to_owned());
                            }
                            true
                        },
                        None,
                        None,
                        None,
                    );
                }
            }
        }
        changed_files.sort();
        changed_files.dedup();

        Self {
            hash: commit.id().to_string(),
            summary: commit.summary().unwrap_or("").to_owned(),
            author: commit.author().name().unwrap_or(&gettext("Unknown")).to_owned(),
            author_email: commit.author().email().unwrap_or("").to_owned(),
            timestamp: commit.time().seconds(),
            changed_files,
        }
    }
}

// ── SubmoduleInfo / SubmoduleStatus ──────────────────────────────────────────────────────

/// Initialization / checkout status of a submodule.
///
/// Currently unused by the UI layer — submodule support is planned for a
/// future release.  `#[allow(dead_code)]` suppresses the compiler warning
/// while keeping the type available for that work.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmoduleStatus {
    /// `.gitmodules` entry exists and the submodule directory is present.
    Present,
    /// Registered in `.gitmodules` but not yet checked out.
    NotInitialized,
    /// Registered but the directory is missing entirely.
    Missing,
}

/// Metadata for a single Git submodule discovered in a repository.
///
/// Currently unused by the UI layer — submodule support is planned for a
/// future release.  `#[allow(dead_code)]` suppresses the compiler warning
/// while keeping the struct available for that work.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct SubmoduleInfo {
    pub name: String,
    pub url: String,
    pub path: PathBuf,
    pub status: SubmoduleStatus,
}

/// Discovers all submodules registered in `repo`.
///
/// Accepts an already-open `&Repository` to avoid redundant `Repository::open`
/// calls when the caller already holds a handle (e.g. inside `HistoryReader`).
///
/// Submodule status is queried via [`Repository::submodule_status`] — the
/// correct stable API in git2.  `git2::Submodule::status()` is **not** part
/// of the public API and must not be called.
///
/// Returns `Err` when `.gitmodules` cannot be parsed, allowing callers to
/// distinguish "no submodules" from "read error".
///
/// # Note — currently unused by the UI
///
/// The submodule panel is planned for a future release.  Until it lands,
/// `#[allow(dead_code)]` suppresses the compiler warning without removing
/// this function from the public API.
#[allow(dead_code)]
pub fn detect_submodules(repo: &Repository) -> Result<Vec<SubmoduleInfo>, git2::Error> {
    let submodules = repo.submodules()?;

    let mut infos = Vec::with_capacity(submodules.len());
    for sm in &submodules {
        let name = sm.name().unwrap_or("").to_owned();
        let url  = sm.url().unwrap_or("").to_owned();
        let path = sm.path().to_path_buf();

        // `Repository::submodule_status` is the stable public API for
        // querying submodule flags.  `SubmoduleIgnore::None` means "report
        // all changes including untracked files".
        let flags = repo
            .submodule_status(&name, SubmoduleIgnore::None)
            .unwrap_or(git2::SubmoduleStatus::IN_CONFIG);

        let status = if flags.contains(git2::SubmoduleStatus::WD_UNINITIALIZED) {
            SubmoduleStatus::NotInitialized
        } else if flags.contains(git2::SubmoduleStatus::IN_WD) {
            SubmoduleStatus::Present
        } else {
            SubmoduleStatus::Missing
        };

        infos.push(SubmoduleInfo { name, url, path, status });
    }

    Ok(infos)
}

/// Convenience wrapper: opens the repository at `repo_path` and delegates to
/// [`detect_submodules`].  Use this only when no `Repository` handle is
/// available; otherwise prefer the `&Repository` overload to avoid
/// a redundant `open()` call.
///
/// # Note — currently unused by the UI
///
/// See the note on [`detect_submodules`].  Both functions will become active
/// once the submodule panel is implemented.
#[allow(dead_code)]
pub fn detect_submodules_at(repo_path: &Path) -> Result<Vec<SubmoduleInfo>, git2::Error> {
    let repo = Repository::open(repo_path)?;
    detect_submodules(&repo)
}

// ── HistoryReader ──────────────────────────────────────────────────────────────────────────────

/// Opens a Git repository and provides access to its commit history.
///
/// Fields are private.  Use [`HistoryReader::repo`] and
/// [`HistoryReader::cpu_pool`] to access them — never use field syntax.
pub struct HistoryReader {
    repo: Repository,
    cpu_pool: CpuPool,
}

impl HistoryReader {
    /// Opens the repository at `path` (bare or with a working tree).
    pub fn open(path: &Path) -> Result<Self, git2::Error> {
        Ok(Self {
            repo: Repository::open(path)?,
            cpu_pool: CpuPool::detect(),
        })
    }

    /// Returns a reference to the underlying [`Repository`].
    ///
    /// Use `reader.repo()` — **not** `reader.repo` (private field).
    ///
    /// # Note — currently unused inside this crate
    ///
    /// The accessor is part of the public API documented in the module-level
    /// doc-comment.  External crates (e.g. an integration test harness or a
    /// future plugin crate) may call it without the compiler knowing.
    #[allow(dead_code)]
    pub fn repo(&self) -> &Repository { &self.repo }

    /// Returns the [`CpuPool`] detected at construction time.
    ///
    /// # Note — currently unused inside this crate
    ///
    /// See the note on [`HistoryReader::repo`].
    #[allow(dead_code)]
    pub fn cpu_pool(&self) -> CpuPool { self.cpu_pool }

    /// Returns up to [`LIST_COMMITS_MAX`] commits reachable from HEAD,
    /// sorted newest-first.
    ///
    /// # Note — currently unused inside this crate
    ///
    /// The active code path is [`list_commits_paginated`] (streaming).
    /// This non-paginated variant is preserved for consumers that need
    /// all commits in one allocation (e.g. snapshot diffing, export tools).
    #[allow(dead_code)]
    pub fn list_commits(&self) -> Result<Vec<CommitInfo>, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        push_head_safe(&mut walk, &self.repo)?;
        walk.set_sorting(git2::Sort::TIME)?;

        let mut commits = Vec::new();
        for oid in walk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            commits.push(CommitInfo::from_commit(&commit, &self.repo));
            if commits.len() >= LIST_COMMITS_MAX { break; }
        }
        Ok(commits)
    }

    /// Streams commits in pages of `page_size`, calling `on_page` for each batch.
    ///
    /// Uses [`MIN_COMMITS_FOR_PARALLEL`] (not `page_size`) as the threshold for
    /// switching to the parallel path, so the decision is independent of the
    /// caller's pagination preference.
    pub fn list_commits_paginated(
        &self,
        page_size: usize,
        on_page: impl FnMut(Vec<CommitInfo>),
    ) -> Result<(), git2::Error> {
        if self.cpu_pool.is_parallel() {
            let count = self.probe_oid_count(MIN_COMMITS_FOR_PARALLEL + 1)?;
            if count > MIN_COMMITS_FOR_PARALLEL {
                let oids = self.collect_all_oids()?;
                return self.list_commits_paginated_parallel(page_size, oids, on_page);
            }
        }
        self.list_commits_paginated_serial(page_size, on_page)
    }

    /// Serial (single-threaded) paginated commit walk.
    fn list_commits_paginated_serial(
        &self,
        page_size: usize,
        mut on_page: impl FnMut(Vec<CommitInfo>),
    ) -> Result<(), git2::Error> {
        let page_size = page_size.max(1);
        let mut walk = self.repo.revwalk()?;
        push_head_safe(&mut walk, &self.repo)?;
        walk.set_sorting(git2::Sort::TIME)?;

        let mut page: Vec<CommitInfo> = Vec::with_capacity(page_size);
        for oid in walk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            page.push(CommitInfo::from_commit(&commit, &self.repo));
            if page.len() >= page_size {
                on_page(std::mem::replace(&mut page, Vec::with_capacity(page_size)));
            }
        }
        if !page.is_empty() { on_page(page); }
        Ok(())
    }

    /// Probes up to `limit + 1` OIDs from HEAD — O(limit), not O(N).
    fn probe_oid_count(&self, limit: usize) -> Result<usize, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        push_head_safe(&mut walk, &self.repo)?;
        walk.set_sorting(git2::Sort::TIME)?;
        let mut count = 0usize;
        for oid in walk {
            oid?;
            count += 1;
            if count > limit { break; }
        }
        Ok(count)
    }

    /// Collects all reachable OIDs from HEAD, newest-first.
    fn collect_all_oids(&self) -> Result<Vec<git2::Oid>, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        push_head_safe(&mut walk, &self.repo)?;
        walk.set_sorting(git2::Sort::TIME)?;
        walk.collect::<Result<Vec<_>, _>>()
    }

    /// Parallel paginated commit walk.
    ///
    /// `on_page` is called **only after all worker threads have joined**
    /// successfully, preventing duplicate-page delivery on panic fallback.
    ///
    /// After merging all shards, commits are re-sorted by timestamp (newest-first)
    /// to restore chronological order that may have been disrupted by shard
    /// interleaving. This guarantees the same ordering as the serial path and
    /// ensures the timeline sidebar sees all years correctly.
    fn list_commits_paginated_parallel(
        &self,
        page_size: usize,
        oids: Vec<git2::Oid>,
        mut on_page: impl FnMut(Vec<CommitInfo>),
    ) -> Result<(), git2::Error> {
        let n_threads = self.cpu_pool.io_threads().max(2);
        let repo_path = self.repo.path().to_path_buf();

        let chunk_size = (oids.len() + n_threads - 1) / n_threads;
        let shards: Vec<Vec<git2::Oid>> = oids
            .chunks(chunk_size)
            .map(|c| c.to_vec())
            .collect();

        let handles: Vec<std::thread::JoinHandle<Vec<CommitInfo>>> = shards
            .into_iter()
            .map(|shard| {
                let path = repo_path.clone();
                std::thread::spawn(move || {
                    let repo = match Repository::open(&path) {
                        Ok(r) => r,
                        Err(_) => return Vec::new(),
                    };
                    let mut results = Vec::with_capacity(shard.len());
                    for oid in shard {
                        if let Ok(commit) = repo.find_commit(oid) {
                            results.push(CommitInfo::from_commit(&commit, &repo));
                        }
                    }
                    results
                })
            })
            .collect();

        let mut all: Vec<CommitInfo> = Vec::with_capacity(oids.len());
        for handle in handles {
            match handle.join() {
                Ok(shard_commits) => all.extend(shard_commits),
                Err(e) => {
                    eprintln!(
                        "[git_engine] parallel worker panicked ({e:?}); \
                         falling back to serial commit walk"
                    );
                    return self.list_commits_paginated_serial(page_size, on_page);
                }
            }
        }

        // Re-sort by timestamp (newest-first) after merging shards.
        // Shards are distributed by OID index, not by time, so the merged
        // result can be out of chronological order. This sort restores the
        // same ordering produced by the serial path and ensures the timeline
        // sidebar displays all years correctly.
        all.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

        let page_size = page_size.max(1);
        for chunk in all.chunks(page_size) {
            on_page(chunk.to_vec());
        }
        Ok(())
    }

    /// Backend commit search via `revwalk` — searches by hash prefix, summary,
    /// or author (all case-insensitive).  Returns at most [`SEARCH_COMMITS_MAX`]
    /// results.
    ///
    /// # Note — currently unused by the UI
    ///
    /// The UI performs search inline inside `window.rs::on_search_changed`,
    /// filtering the already-loaded `all_commits` `Vec` in a `thread::spawn`
    /// without opening a new revwalk.  This function implements the same logic
    /// at the git-engine level and is preserved for a future refactor that
    /// moves search responsibility from `window.rs` into the engine (e.g. to
    /// support incremental streaming search over very large histories that do
    /// not fit in memory).  Until that refactor lands, the compiler warning is
    /// suppressed with `#[allow(dead_code)]` to avoid misleading noise in CI.
    #[allow(dead_code)]
    pub fn search_commits(&self, query: &str) -> Result<Vec<CommitInfo>, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        push_head_safe(&mut walk, &self.repo)?;
        walk.set_sorting(git2::Sort::TIME)?;

        if query.is_empty() {
            let mut results = Vec::with_capacity(SEARCH_COMMITS_MAX);
            for oid in walk {
                let oid = oid?;
                let commit = self.repo.find_commit(oid)?;
                results.push(CommitInfo::from_commit(&commit, &self.repo));
                if results.len() >= SEARCH_COMMITS_MAX { break; }
            }
            return Ok(results);
        }

        let q = query.to_lowercase();
        let mut results = Vec::new();

        for oid in walk {
            let oid = oid?;
            let oid_str = oid.to_string();
            let hash_match = oid_str.starts_with(&q);

            let commit = self.repo.find_commit(oid)?;
            let info = CommitInfo::from_commit(&commit, &self.repo);

            if hash_match
                || info.summary.to_lowercase().contains(&q)
                || info.author.to_lowercase().contains(&q)
            {
                results.push(info);
                if results.len() >= SEARCH_COMMITS_MAX { break; }
            }
        }
        Ok(results)
    }

    /// Extracts the underlying `git2::Repository` out of this reader.
    ///
    /// Used in `window.rs` to avoid opening the same repository path twice:
    /// the window opens it once for validation via `HistoryReader::open`, then
    /// calls this method to retrieve the inner handle for tree/snapshot browsing.
    ///
    /// # Note — currently unused inside this crate
    ///
    /// The active code path in `window.rs::load_repository` opens the repo
    /// independently via `git2::Repository::open`.  This accessor is preserved
    /// for a future refactor that routes all repository handles through
    /// `HistoryReader` to avoid redundant opens.
    #[allow(dead_code)]
    pub fn into_git2(self) -> Repository { self.repo }
}

// ── TreeNode ───────────────────────────────────────────────────────────────────────────

/// A single node in a materialized snapshot tree.
#[derive(Debug, Clone)]
pub enum TreeNode {
    /// A regular file at the given repository-relative path.
    File(PathBuf),
    /// A directory at the given repository-relative path.
    Dir(PathBuf),
    /// A Git submodule (gitlink) at the given repository-relative path.
    Submodule(PathBuf),
}

impl TreeNode {
    pub fn path(&self) -> &Path {
        match self {
            TreeNode::File(p) | TreeNode::Dir(p) | TreeNode::Submodule(p) => p.as_path(),
        }
    }

    pub fn is_dir(&self) -> bool { matches!(self, TreeNode::Dir(_)) }
    pub fn is_submodule(&self) -> bool { matches!(self, TreeNode::Submodule(_)) }
}

// ── DirCache ───────────────────────────────────────────────────────────────────────────

/// Simple LRU cache for directory listings keyed by `(commit_hash, dir_path)`.
#[derive(Debug)]
pub struct DirCache {
    entries: VecDeque<((String, PathBuf), Arc<Vec<TreeNode>>)>,
}

impl DirCache {
    pub fn new() -> Self {
        Self { entries: VecDeque::with_capacity(DIR_CACHE_MAX_ENTRIES) }
    }

    pub fn get(&mut self, hash: &str, dir: &Path) -> Option<Arc<Vec<TreeNode>>> {
        let pos = self
            .entries
            .iter()
            .position(|((h, d), _)| h == hash && d == dir)?;
        let entry = self.entries.remove(pos)?;
        let arc = Arc::clone(&entry.1);
        self.entries.push_front(entry);
        Some(arc)
    }

    pub fn insert(&mut self, hash: String, dir: PathBuf, nodes: Vec<TreeNode>) {
        if let Some(pos) = self
            .entries
            .iter()
            .position(|((h, d), _)| *h == hash && *d == dir)
        {
            self.entries.remove(pos);
        }
        if self.entries.len() >= DIR_CACHE_MAX_ENTRIES {
            self.entries.pop_back();
        }
        self.entries.push_front(((hash, dir), Arc::new(nodes)));
    }

    pub fn clear(&mut self) { self.entries.clear(); }
}

impl Default for DirCache {
    fn default() -> Self { Self::new() }
}

// ── SnapshotResolver ──────────────────────────────────────────────────────────────────────────────

/// Resolves a commit hash (or any Git revision string) into a raw Git tree.
pub struct SnapshotResolver<'repo> {
    repo: &'repo Repository,
}

impl<'repo> SnapshotResolver<'repo> {
    pub fn new(repo: &'repo Repository) -> Self { Self { repo } }

    /// Resolves `revision` and materializes only the **direct children** of
    /// `dir` in the corresponding commit tree.
    ///
    /// Entries with missing or empty names are silently skipped.
    pub fn resolve_dir(
        &self,
        revision: &str,
        dir: &Path,
    ) -> Result<Vec<TreeNode>, git2::Error> {
        let obj = self.repo.revparse_single(revision)?;
        let commit = obj.peel_to_commit()?;
        let root_tree = commit.tree()?;

        let subtree = if dir.as_os_str().is_empty() {
            root_tree
        } else {
            let entry = root_tree.get_path(dir)?;
            self.repo.find_tree(entry.id())?
        };

        let mut nodes = Vec::new();
        for entry in subtree.iter() {
            let name = match entry.name() {
                Some(n) if !n.is_empty() => n,
                _ => continue,
            };
            let path = dir.join(name);
            if let Some(node) = entry_to_tree_node(entry.kind(), path) {
                nodes.push(node);
            }
        }
        Ok(nodes)
    }

    /// Full-tree walk, capped at [`MAX_FULL_TREE_ENTRIES`] entries and
    /// [`MAX_TREE_DEPTH`] levels.
    ///
    /// Hitting either cap is **not** treated as an error: the function returns
    /// `Ok` with the nodes collected so far and logs the truncation reason to
    /// stderr.  Only genuine git2 I/O failures propagate as `Err`.
    ///
    /// # Note — currently unused by the UI
    ///
    /// The active code path for file browsing calls [`resolve_dir`] (lazy,
    /// one directory at a time).  This full-tree variant is preserved for
    /// future non-interactive consumers such as export, indexing, or static
    /// analysis tools that need the complete snapshot in one call.
    #[allow(dead_code)]
    pub fn resolve_tree(&self, revision: &str) -> Result<Vec<TreeNode>, git2::Error> {
        let obj = self.repo.revparse_single(revision)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;
        let materializer = SnapshotMaterializer::new(self.repo);
        // materialize_inner uses MaterializeOutcome to distinguish truncation
        // (Ok variant) from real git2 errors.  resolve_tree maps both
        // Truncated and Complete to Ok, never surfacing the limit as Err.
        match materializer.materialize_inner(&tree, PathBuf::new(), 0, MAX_FULL_TREE_ENTRIES)? {
            MaterializeOutcome::Complete(nodes) => Ok(nodes),
            MaterializeOutcome::Truncated(nodes, reason) => {
                eprintln!("[git_engine] resolve_tree: tree truncated ({reason})");
                Ok(nodes)
            }
        }
    }
}

// ── MaterializeOutcome ──────────────────────────────────────────────────────────────────────────

/// Internal result of [`SnapshotMaterializer::materialize_inner`].
///
/// Separates the "limit reached" signal from true git2 I/O errors so that
/// [`SnapshotResolver::resolve_tree`] can return `Ok` on truncation instead
/// of propagating a fake `Err` to the UI.
///
/// This enum is private to the module and must not be exposed in the public API.
///
/// # Note — currently unused at the call-site level
///
/// `materialize_inner` is only called from `resolve_tree`, which is itself
/// guarded by `#[allow(dead_code)]`.  This attribute silences the cascade
/// warning without removing the type.
#[allow(dead_code)]
enum MaterializeOutcome {
    /// Walk completed without hitting any limit.
    Complete(Vec<TreeNode>),
    /// Walk was cut short at an entry or depth limit.  The `String` contains a
    /// human-readable reason logged to stderr by the caller.
    Truncated(Vec<TreeNode>, String),
}

// ── SnapshotMaterializer ───────────────────────────────────────────────────────────────────────────────

/// Converts a raw Git tree object into a navigable list of [`TreeNode`]s.
pub struct SnapshotMaterializer<'repo> {
    repo: &'repo Repository,
}

impl<'repo> SnapshotMaterializer<'repo> {
    pub fn new(repo: &'repo Repository) -> Self { Self { repo } }

    /// Public entry point kept for API compatibility.
    ///
    /// Delegates to [`materialize_inner`] and maps [`MaterializeOutcome`] back
    /// to the original `Result<Vec<TreeNode>, git2::Error>` signature:
    /// - `Complete`  → `Ok(nodes)`
    /// - `Truncated` → `Ok(nodes)` (truncation is not an error)
    /// - git2 I/O failures → `Err(e)` (propagated unchanged)
    ///
    /// # Note — currently unused by the UI
    ///
    /// See [`SnapshotResolver::resolve_tree`] for context.  This method
    /// exists as a stable public entry point for callers that already hold
    /// a `git2::Tree` reference and do not need to go through `resolve_tree`.
    #[allow(dead_code)]
    pub fn materialize(
        &self,
        tree: &git2::Tree<'_>,
        prefix: PathBuf,
        depth: usize,
        limit: usize,
    ) -> Result<Vec<TreeNode>, git2::Error> {
        match self.materialize_inner(tree, prefix, depth, limit)? {
            MaterializeOutcome::Complete(nodes) => Ok(nodes),
            MaterializeOutcome::Truncated(nodes, _reason) => Ok(nodes),
        }
    }

    /// Core recursive walk.  Returns a [`MaterializeOutcome`] so that callers
    /// can distinguish a clean finish from a limit-induced truncation without
    /// abusing `Err` as a control-flow signal.
    ///
    /// Only genuine git2 I/O errors (e.g. `find_tree` failures) are returned
    /// as `Err`.  Reaching the entry cap or the depth cap is encoded as
    /// `Ok(Truncated(...))`
    ///
    /// # Note — currently unused by the UI
    ///
    /// Called only from `materialize` and `resolve_tree`, both of which are
    /// guarded by `#[allow(dead_code)]`.  This attribute silences the cascade
    /// warning without removing the implementation.
    #[allow(dead_code)]
    fn materialize_inner(
        &self,
        tree: &git2::Tree<'_>,
        prefix: PathBuf,
        depth: usize,
        limit: usize,
    ) -> Result<MaterializeOutcome, git2::Error> {
        if depth > MAX_TREE_DEPTH {
            return Ok(MaterializeOutcome::Truncated(
                Vec::new(),
                format!(
                    "depth limit ({MAX_TREE_DEPTH}) exceeded at '{}'",
                    prefix.display()
                ),
            ));
        }

        let mut nodes = Vec::new();
        for entry in tree.iter() {
            if nodes.len() >= limit {
                return Ok(MaterializeOutcome::Truncated(
                    nodes,
                    format!(
                        "{} entries reached limit {limit} — use resolve_dir for interactive browsing",
                        limit,
                    ),
                ));
            }

            let name = match entry.name() {
                Some(n) if !n.is_empty() => n,
                _ => continue,
            };

            let path = prefix.join(name);
            match entry.kind() {
                Some(ObjectType::Blob) => {
                    nodes.push(TreeNode::File(path));
                }
                Some(ObjectType::Tree) => {
                    nodes.push(TreeNode::Dir(path.clone()));
                    let subtree = self.repo.find_tree(entry.id())?; // real I/O error — propagate
                    let remaining = limit.saturating_sub(nodes.len());
                    match self.materialize_inner(&subtree, path, depth + 1, remaining)? {
                        MaterializeOutcome::Complete(mut children) => {
                            nodes.append(&mut children);
                        }
                        MaterializeOutcome::Truncated(mut children, reason) => {
                            nodes.append(&mut children);
                            // Propagate the truncation signal upward so the
                            // top-level caller (resolve_tree) can log it once.
                            return Ok(MaterializeOutcome::Truncated(nodes, reason));
                        }
                    }
                }
                Some(ObjectType::Commit) => {
                    nodes.push(TreeNode::Submodule(path));
                }
                _ => {}
            }
        }
        Ok(MaterializeOutcome::Complete(nodes))
    }

    /// Reads the raw byte content of a file at `path` in the given `revision`.
    ///
    /// Returns an error if `path` refers to a directory, submodule, or any
    /// non-blob object.
    pub fn read_file(
        &self,
        revision: &str,
        path: &Path,
    ) -> Result<Vec<u8>, git2::Error> {
        let obj = self.repo.revparse_single(revision)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;
        let entry = tree.get_path(path)?;

        if entry.kind() != Some(ObjectType::Blob) {
            return Err(git2::Error::from_str(&format!(
                "path '{}' is not a file (kind: {:?})",
                path.display(),
                entry.kind(),
            )));
        }

        let blob = self.repo.find_blob(entry.id())?;
        Ok(blob.content().to_vec())
    }
}
