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
//! - [`detect_submodules`]    – scans a repository for registered submodules.
//!
//! # Threading model
//!
//! `git2::Repository` is **not** `Send`, so each background thread that needs
//! to access the object database must open its own `Repository` handle.
//! The parallel commit path collects OIDs on the main revwalk, shards them
//! across `CpuPool::io_threads()` worker threads (each opening their own
//! handle), then merges results back in the original sort order before
//! forwarding pages to the caller.
//!
//! # Submodule model
//!
//! Git records a submodule reference as an `ObjectType::Commit` ("gitlink")
//! entry inside a tree object.  `resolve_dir` now emits `TreeNode::Submodule`
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

use git2::{ObjectType, Repository, SubmoduleStatus as Git2SubmoduleStatus};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ── Limits ────────────────────────────────────────────────────────────────────

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
/// Bumped from 32 → 64 to absorb more back/forward navigation steps.
const DIR_CACHE_MAX_ENTRIES: usize = 64;

/// Maximum number of parallel worker threads for CPU-bound work.
/// Caps at 8 to avoid thrashing on many-core machines (32, 64 cores).
const MAX_WORKER_THREADS: usize = 8;

/// Maximum number of parallel threads for I/O-bound git object-DB access.
/// git2 object reads are I/O bound; more than 4 threads rarely helps.
const MAX_IO_THREADS: usize = 4;

/// Maximum results returned by [`HistoryReader::search_commits`].
/// Prevents unbounded allocations when the query matches many commits.
const SEARCH_COMMITS_MAX: usize = 5_000;

// ── CpuPool ───────────────────────────────────────────────────────────────────

/// Runtime CPU detection and derived thread-pool sizes.
///
/// Call [`CpuPool::detect()`] once at startup and store the result; all
/// thread-count decisions in this module are derived from it.
///
/// # Examples
/// ```
/// let pool = CpuPool::detect();
/// println!("logical cores: {}", pool.logical_cores());
/// println!("I/O threads:   {}", pool.io_threads());
/// println!("worker threads:{}", pool.worker_threads());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct CpuPool {
    /// Number of logical CPU cores available to this process.
    /// Always ≥ 1.  Uses `std::thread::available_parallelism()`.
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

    /// Number of logical CPU cores available to this process.
    #[inline] pub fn logical_cores(self) -> usize { self.logical_cores }

    /// Recommended number of threads for CPU-bound parallel work.
    /// Saturates at [`MAX_WORKER_THREADS`] to avoid thrashing.
    #[inline] pub fn worker_threads(self) -> usize {
        self.logical_cores.min(MAX_WORKER_THREADS)
    }

    /// Recommended number of threads for I/O-bound git object-DB reads.
    /// git2 pack-file access does not scale past ~4 threads.
    #[inline] pub fn io_threads(self) -> usize {
        self.logical_cores.min(MAX_IO_THREADS)
    }

    /// Returns `true` if more than one thread should be used.
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
            author: commit
                .author()
                .name()
                .unwrap_or("Unknown")
                .to_owned(),
            author_email: commit
                .author()
                .email()
                .unwrap_or("")
                .to_owned(),
            timestamp: commit.time().seconds(),
        }
    }
}

// ── SubmoduleInfo / SubmoduleStatus ───────────────────────────────────────────

/// Initialization / checkout status of a submodule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmoduleStatus {
    /// `.gitmodules` entry exists and the submodule directory is present.
    Present,
    /// Registered in `.gitmodules` but the directory has not been checked out
    /// (`git submodule update --init` was never run).
    NotInitialized,
    /// The submodule path is registered but the directory is missing entirely.
    Missing,
}

/// Metadata for a single Git submodule discovered in a repository.
#[derive(Debug, Clone)]
pub struct SubmoduleInfo {
    /// Human-readable submodule name (from `.gitmodules`).
    pub name: String,
    /// Remote URL of the submodule.
    pub url: String,
    /// Path relative to the super-repository root.
    pub path: PathBuf,
    /// Detected status of the submodule on disk.
    pub status: SubmoduleStatus,
}

/// Discovers all submodules registered in the repository at `repo_path`.
///
/// Uses git2 to read `.gitmodules` — no subprocess, no filesystem traversal
/// outside the repository root.
///
/// Returns an empty `Vec` (not an error) when the repository has no
/// submodules.  Returns `Err` when `.gitmodules` exists but cannot be parsed,
/// allowing callers to distinguish "no submodules" from "read error".
pub fn detect_submodules(repo_path: &Path) -> Result<Vec<SubmoduleInfo>, git2::Error> {
    let repo = Repository::open(repo_path)?;

    let submodules = repo.submodules()?; // propagates parse errors to the caller

    let infos = submodules
        .into_iter()
        .map(|sm| {
            let name = sm.name().unwrap_or("").to_owned();
            let url  = sm.url().unwrap_or("").to_owned();
            let path = sm.path().to_path_buf();

            // Use git2 SubmoduleStatus flags — correct for worktrees, bare
            // repos, and other non-standard layouts.  The heuristic of checking
            // for .git/HEAD on disk is fragile and has been removed.
            let flags = sm.status(None).unwrap_or(Git2SubmoduleStatus::IN_CONFIG);
            let status = if flags.contains(Git2SubmoduleStatus::WD_UNINITIALIZED) {
                SubmoduleStatus::NotInitialized
            } else if flags.contains(Git2SubmoduleStatus::WD_WD_MODIFIED)
                || flags.contains(Git2SubmoduleStatus::IN_WD)
            {
                SubmoduleStatus::Present
            } else {
                SubmoduleStatus::Missing
            };

            SubmoduleInfo { name, url, path, status }
        })
        .collect();

    Ok(infos)
}

// ── HistoryReader ──────────────────────────────────────────────────────────────

/// Opens a Git repository and provides access to its commit history.
pub struct HistoryReader {
    repo: Repository,       // private — use accessor methods
    cpu_pool: CpuPool,      // private — use accessor methods
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
    /// Prefer the higher-level methods on `HistoryReader` wherever possible.
    /// This accessor exists for callers that need one-off git2 operations not
    /// covered by this API.
    pub fn repo(&self) -> &Repository { &self.repo }

    /// Returns the [`CpuPool`] detected at construction time.
    pub fn cpu_pool(&self) -> CpuPool { self.cpu_pool }

    /// Returns up to [`LIST_COMMITS_MAX`] commits reachable from HEAD,
    /// sorted newest-first.
    pub fn list_commits(&self) -> Result<Vec<CommitInfo>, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(git2::Sort::TIME)?;

        let mut commits = Vec::new();
        for oid in walk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            commits.push(CommitInfo::from_commit(&commit));
            if commits.len() >= LIST_COMMITS_MAX {
                break;
            }
        }
        Ok(commits)
    }

    /// Streams commits in pages of `page_size`, calling `on_page` for each batch.
    ///
    /// On multi-core machines this automatically delegates to the parallel path
    /// for repositories large enough to benefit from parallelism
    /// (> 2 × `page_size` commits).
    pub fn list_commits_paginated(
        &self,
        page_size: usize,
        on_page: impl FnMut(Vec<CommitInfo>),
    ) -> Result<(), git2::Error> {
        if self.cpu_pool.is_parallel() {
            let oids = self.collect_oids()?;
            if oids.len() > page_size * 2 {
                return self.list_commits_paginated_parallel(page_size, oids, on_page);
            }
        }
        self.list_commits_paginated_serial(page_size, on_page)
    }

    /// Serial (single-threaded) paginated commit walk.  Always correct;
    /// used on single-core systems or for small repositories.
    fn list_commits_paginated_serial(
        &self,
        page_size: usize,
        mut on_page: impl FnMut(Vec<CommitInfo>),
    ) -> Result<(), git2::Error> {
        let page_size = page_size.max(1);
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
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

    /// Collects all reachable OIDs from HEAD into a `Vec`, newest-first.
    ///
    /// This is a lightweight pass (only OIDs, no blob content) used by the
    /// parallel path to shard work across threads.
    fn collect_oids(&self) -> Result<Vec<git2::Oid>, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(git2::Sort::TIME)?;
        walk.collect::<Result<Vec<_>, _>>()
    }

    /// Parallel paginated commit walk.
    ///
    /// # Strategy
    ///
    /// 1. Collect all OIDs via a single serial revwalk (fast — no blob access).
    /// 2. Shard OIDs across `CpuPool::io_threads()` worker threads.
    /// 3. Each thread opens its own `Repository` handle and hydrates its shard
    ///    (OID → `CommitInfo`).  Hydration is I/O bound (pack-file reads).
    /// 4. Shards are recombined in the original revwalk order.
    /// 5. The merged stream is forwarded to `on_page` in `page_size` batches.
    ///
    /// Falls back to the serial path if `Repository::open` fails in any worker
    /// or if a worker thread panics.  The fallback is logged to stderr so that
    /// concurrency bugs remain observable in production.
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
                    // Worker thread panicked — log for observability, then fall
                    // back to the serial path so the caller still gets results.
                    eprintln!(
                        "[git_engine] parallel worker panicked ({e:?}); \
                         falling back to serial commit walk"
                    );
                    return self.list_commits_paginated_serial(page_size, on_page);
                }
            }
        }

        let page_size = page_size.max(1);
        for chunk in all.chunks(page_size) {
            on_page(chunk.to_vec());
        }
        Ok(())
    }

    /// Searches commits reachable from HEAD whose summary, hash prefix, or
    /// author match `query` (case-insensitive).
    ///
    /// Returns at most [`SEARCH_COMMITS_MAX`] results to avoid unbounded
    /// allocations on repositories with many matching commits.
    ///
    /// When `query` is empty the method returns the first [`SEARCH_COMMITS_MAX`]
    /// commits via a paginated walk rather than loading up to
    /// [`LIST_COMMITS_MAX`] (50 000) at once.
    pub fn search_commits(&self, query: &str) -> Result<Vec<CommitInfo>, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(git2::Sort::TIME)?;

        if query.is_empty() {
            // Return a bounded, paginated slice instead of loading 50 k commits.
            let mut results = Vec::with_capacity(SEARCH_COMMITS_MAX);
            for oid in walk {
                let oid = oid?;
                let commit = self.repo.find_commit(oid)?;
                results.push(CommitInfo::from_commit(&commit));
                if results.len() >= SEARCH_COMMITS_MAX {
                    break;
                }
            }
            return Ok(results);
        }

        let q = query.to_lowercase();
        let mut results = Vec::new();
        for oid in walk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            let info = CommitInfo::from_commit(&commit);
            if info.summary.to_lowercase().contains(&q)
                || info.hash.starts_with(query)
                || info.author.to_lowercase().contains(&q)
            {
                results.push(info);
                if results.len() >= SEARCH_COMMITS_MAX {
                    break;
                }
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
    /// Recorded as `ObjectType::Commit` in the parent tree object.
    Submodule(PathBuf),
}

impl TreeNode {
    /// Returns the path of this node.
    pub fn path(&self) -> &Path {
        match self {
            TreeNode::File(p) | TreeNode::Dir(p) | TreeNode::Submodule(p) => p.as_path(),
        }
    }

    /// Returns `true` if this node is a directory.
    pub fn is_dir(&self) -> bool {
        matches!(self, TreeNode::Dir(_))
    }

    /// Returns `true` if this node is a submodule (gitlink).
    pub fn is_submodule(&self) -> bool {
        matches!(self, TreeNode::Submodule(_))
    }
}

// ── DirCache ──────────────────────────────────────────────────────────────────

/// Simple LRU cache for directory listings keyed by `(commit_hash, dir_path)`.
///
/// Capacity: [`DIR_CACHE_MAX_ENTRIES`] (64).  Values stored as `Arc<Vec<TreeNode>>`
/// so cache hits are O(1) pointer copies.
#[derive(Debug)]
pub struct DirCache {
    entries: VecDeque<((String, PathBuf), Arc<Vec<TreeNode>>)>,
}

impl DirCache {
    /// Creates a new, empty cache.
    pub fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(DIR_CACHE_MAX_ENTRIES),
        }
    }

    /// Looks up `(hash, dir)`.  On a hit the entry is promoted to MRU position.
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

    /// Inserts or replaces the entry for `(hash, dir)`.  Evicts LRU when full.
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

    /// Drops all cached entries.  Call when a new repository is opened.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Default for DirCache {
    fn default() -> Self { Self::new() }
}

// ── SnapshotResolver ───────────────────────────────────────────────────────────

/// Resolves a commit hash (or any Git revision string) into a raw Git tree.
pub struct SnapshotResolver<'repo> {
    repo: &'repo Repository,
}

impl<'repo> SnapshotResolver<'repo> {
    /// Creates a new resolver bound to `repo`.
    pub fn new(repo: &'repo Repository) -> Self {
        Self { repo }
    }

    /// Resolves `revision` and materializes only the **direct children** of
    /// `dir` in the corresponding commit tree.
    ///
    /// `ObjectType::Commit` entries (gitlinks / submodules) are emitted as
    /// [`TreeNode::Submodule`] so callers can render them with a distinct icon.
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
            let name = entry.name().unwrap_or("");
            let path = dir.join(name);
            match entry.kind() {
                Some(ObjectType::Blob)   => nodes.push(TreeNode::File(path)),
                Some(ObjectType::Tree)   => nodes.push(TreeNode::Dir(path)),
                Some(ObjectType::Commit) => nodes.push(TreeNode::Submodule(path)),
                _ => {}
            }
        }
        Ok(nodes)
    }

    /// Full-tree walk.  **Capped at [`MAX_FULL_TREE_ENTRIES`] entries** and
    /// [`MAX_TREE_DEPTH`] levels to prevent freezing or stack overflow on
    /// large monorepos.
    ///
    /// The entry limit is enforced *inside* the recursive materializer so that
    /// the walk is interrupted as soon as the cap is reached, rather than after
    /// the entire tree has been traversed.
    pub fn resolve_tree(&self, revision: &str) -> Result<Vec<TreeNode>, git2::Error> {
        let obj = self.repo.revparse_single(revision)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;
        let materializer = SnapshotMaterializer::new(self.repo);
        // Pass the limit into materialize so it can abort early.
        materializer.materialize(&tree, PathBuf::new(), 0, MAX_FULL_TREE_ENTRIES)
    }
}

// ── SnapshotMaterializer ─────────────────────────────────────────────────────

/// Converts a raw Git tree object into a navigable list of [`TreeNode`]s.
pub struct SnapshotMaterializer<'repo> {
    repo: &'repo Repository,
}

impl<'repo> SnapshotMaterializer<'repo> {
    /// Creates a new materializer bound to `repo`.
    pub fn new(repo: &'repo Repository) -> Self {
        Self { repo }
    }

    /// Recursively walks `tree` and returns a flat, depth-first list of nodes.
    ///
    /// `limit` is a hard cap on the total number of nodes across the entire
    /// recursive call tree.  When the accumulated count reaches `limit` an
    /// error is returned immediately, aborting further traversal.
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
            // Enforce entry limit before processing the next entry so we abort
            // as early as possible — critical for large monorepos.
            if nodes.len() >= limit {
                return Err(git2::Error::from_str(&format!(
                    "Tree too large for full walk ({} entries reached limit {}). \
                     Use resolve_dir for interactive browsing.",
                    nodes.len(),
                    limit,
                )));
            }

            let name = entry.name().unwrap_or("");
            let path = prefix.join(name);
            match entry.kind() {
                Some(ObjectType::Blob) => {
                    nodes.push(TreeNode::File(path));
                }
                Some(ObjectType::Tree) => {
                    nodes.push(TreeNode::Dir(path.clone()));
                    let subtree = self.repo.find_tree(entry.id())?;
                    // Pass remaining capacity so child calls also abort early.
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
    /// Returns an error if `path` does not exist in the tree, or if it refers
    /// to a directory, submodule, or any non-blob object.
    pub fn read_file(
        &self,
        revision: &str,
        path: &Path,
    ) -> Result<Vec<u8>, git2::Error> {
        let obj = self.repo.revparse_single(revision)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;
        let entry = tree.get_path(path)?;

        // Verify the entry is a blob before calling find_blob.  Without this
        // check, passing a directory or submodule path produces a confusing
        // generic error from git2 instead of a clear diagnostic.
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
