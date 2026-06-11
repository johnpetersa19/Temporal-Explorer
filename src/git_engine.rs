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

use git2::{ObjectType, Repository, SubmoduleIgnore};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ── Limits ──────────────────────────────────────────────────────────────────

/// Hard cap for [`HistoryReader::list_commits`] (non-paginated).
/// Above this, callers should use [`HistoryReader::list_commits_paginated`].
pub const LIST_COMMITS_MAX: usize = 50_000;

/// Hard cap for a full recursive tree walk in [`SnapshotResolver::resolve_tree`].
/// The limit is enforced *inside* [`SnapshotMaterializer::materialize`] so that
/// the walk is aborted early — before all memory and I/O are consumed.
const MAX_FULL_TREE_ENTRIES: usize = 8_000;

/// Maximum recursion depth for [`SnapshotMaterializer::materialize`].
/// Prevents stack overflow on pathologically deep trees.
const MAX_TREE_DEPTH: usize = 64;

/// Number of cached directory listings in [`DirCache`].
const DIR_CACHE_MAX_ENTRIES: usize = 64;

/// Maximum number of parallel worker threads for CPU-bound work.
const MAX_WORKER_THREADS: usize = 8;

/// Maximum number of parallel threads for I/O-bound git object-DB access.
const MAX_IO_THREADS: usize = 4;

/// Maximum results returned by [`HistoryReader::search_commits`].
const SEARCH_COMMITS_MAX: usize = 5_000;

/// Minimum number of commits required before the parallel path is preferred
/// over the serial path.  Decoupled from `page_size` so that the parallelism
/// threshold does not vary with the caller's pagination preference.
const MIN_COMMITS_FOR_PARALLEL: usize = 2_000;

// ── Internal helpers ───────────────────────────────────────────────────────────

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
/// `Err` with error class `Reference` (code -3, "reference 'refs/heads/...'
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

// ── CpuPool ───────────────────────────────────────────────────────────────────

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

    #[inline] pub fn logical_cores(self) -> usize { self.logical_cores }

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

// ── CommitInfo ────────────────────────────────────────────────────────────────

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
}

impl CommitInfo {
    fn from_commit(commit: &git2::Commit<'_>) -> Self {
        Self {
            hash: commit.id().to_string(),
            summary: commit.summary().unwrap_or("").to_owned(),
            author: commit.author().name().unwrap_or("Unknown").to_owned(),
            author_email: commit.author().email().unwrap_or("").to_owned(),
            timestamp: commit.time().seconds(),
        }
    }
}

// ── SubmoduleInfo / SubmoduleStatus ───────────────────────────────────────────────

/// Initialization / checkout status of a submodule.
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
pub fn detect_submodules_at(repo_path: &Path) -> Result<Vec<SubmoduleInfo>, git2::Error> {
    let repo = Repository::open(repo_path)?;
    detect_submodules(&repo)
}

// ── HistoryReader ──────────────────────────────────────────────────────────────────

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
    pub fn repo(&self) -> &Repository { &self.repo }

    /// Returns the [`CpuPool`] detected at construction time.
    pub fn cpu_pool(&self) -> CpuPool { self.cpu_pool }

    /// Returns up to [`LIST_COMMITS_MAX`] commits reachable from HEAD,
    /// sorted newest-first.
    pub fn list_commits(&self) -> Result<Vec<CommitInfo>, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        push_head_safe(&mut walk, &self.repo)?;
        walk.set_sorting(git2::Sort::TIME)?;

        let mut commits = Vec::new();
        for oid in walk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            commits.push(CommitInfo::from_commit(&commit));
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
            page.push(CommitInfo::from_commit(&commit));
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
                            results.push(CommitInfo::from_commit(&commit));
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

    /// Searches commits by hash prefix, summary, or author (all case-insensitive).
    ///
    /// Returns at most [`SEARCH_COMMITS_MAX`] results.
    pub fn search_commits(&self, query: &str) -> Result<Vec<CommitInfo>, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        push_head_safe(&mut walk, &self.repo)?;
        walk.set_sorting(git2::Sort::TIME)?;

        if query.is_empty() {
            let mut results = Vec::with_capacity(SEARCH_COMMITS_MAX);
            for oid in walk {
                let oid = oid?;
                let commit = self.repo.find_commit(oid)?;
                results.push(CommitInfo::from_commit(&commit));
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
            let info = CommitInfo::from_commit(&commit);

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
}

// ── TreeNode ───────────────────────────────────────────────────────────────────

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

// ── DirCache ───────────────────────────────────────────────────────────────────

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
        let entry = self.entries.remove(pos).unwrap();
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

// ── SnapshotResolver ─────────────────────────────────────────────────────────────────

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
    pub fn resolve_tree(&self, revision: &str) -> Result<Vec<TreeNode>, git2::Error> {
        let obj = self.repo.revparse_single(revision)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;
        let materializer = SnapshotMaterializer::new(self.repo);
        materializer.materialize(&tree, PathBuf::new(), 0, MAX_FULL_TREE_ENTRIES)
    }
}

// ── SnapshotMaterializer ───────────────────────────────────────────────────────────────

/// Converts a raw Git tree object into a navigable list of [`TreeNode`]s.
pub struct SnapshotMaterializer<'repo> {
    repo: &'repo Repository,
}

impl<'repo> SnapshotMaterializer<'repo> {
    pub fn new(repo: &'repo Repository) -> Self { Self { repo } }

    /// Recursively walks `tree` and returns a flat, depth-first list of nodes.
    ///
    /// Aborts early when `limit` is reached or `MAX_TREE_DEPTH` is exceeded.
    /// Entries with missing or empty names are silently skipped.
    pub fn materialize(
        &self,
        tree: &git2::Tree<'_>,
        prefix: PathBuf,
        depth: usize,
        limit: usize,
    ) -> Result<Vec<TreeNode>, git2::Error> {
        if depth > MAX_TREE_DEPTH {
            return Err(git2::Error::from_str(&format!(
                "Tree depth limit ({MAX_TREE_DEPTH}) exceeded at '{}'. \
                 Truncating subtree.",
                prefix.display()
            )));
        }

        let mut nodes = Vec::new();
        for entry in tree.iter() {
            if nodes.len() >= limit {
                return Err(git2::Error::from_str(&format!(
                    "Tree too large for full walk ({} entries reached limit {}). \
                     Use resolve_dir for interactive browsing.",
                    nodes.len(), limit,
                )));
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
                    let subtree = self.repo.find_tree(entry.id())?;
                    let remaining = limit.saturating_sub(nodes.len());
                    let mut children = self.materialize(&subtree, path, depth + 1, remaining)?;
                    nodes.append(&mut children);
                }
                Some(ObjectType::Commit) => {
                    nodes.push(TreeNode::Submodule(path));
                }
                _ => {}
            }
        }
        Ok(nodes)
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
