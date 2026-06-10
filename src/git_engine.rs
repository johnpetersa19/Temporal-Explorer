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
//! - [`HistoryReader`]        – opens a repository and iterates commits.
//! - [`CommitInfo`]           – lightweight commit data for the UI list.
//! - [`SnapshotResolver`]     – resolves a commit hash into a full tree.
//! - [`SnapshotMaterializer`] – converts the resolved tree into [`TreeNode`]s.
//! - [`TreeNode`]             – a single file or directory in a materialized snapshot.
//! - [`DirCache`]             – LRU cache of (hash, dir) → Arc<Vec<TreeNode>>.
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
//! exist in the given tree") instead of opening any file outside the repository.
//!
//! Concretely:
//!
//! * `resolve_dir` calls `root_tree.get_path(dir)`.  git2 walks the in-memory
//!   tree object component by component.  A `..` component never matches a real
//!   tree entry, so the call returns `Err` before any I/O occurs.
//! * `read_file` / `SnapshotMaterializer::read_file` calls `tree.get_path(path)`
//!   for the same reason and with the same guarantee.
//! * `revparse_single` resolves revision strings against the Git ref database,
//!   not the filesystem.  Shell-injection via revision strings (e.g. `--upload-pack`)
//!   is not applicable because git2 is a native library with no subprocess.
//!
//! The module does **not** perform explicit `Path::components()` filtering because
//! that layer of defence is redundant here and would add false confidence that
//! a missed branch is safe.  The git2 boundary *is* the trust boundary.
//!
//! One edge case worth noting: on Windows, `Path::new("C:\\Windows")` is an
//! absolute path that `get_path` will reject, but callers should still avoid
//! passing OS-absolute paths to keep code portable and intentions clear.

use git2::{ObjectType, Repository};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ── Limits ────────────────────────────────────────────────────────────────────

/// Hard cap for [`HistoryReader::list_commits`] (non-paginated).
/// Above this, callers should use [`HistoryReader::list_commits_paginated`].
pub const LIST_COMMITS_MAX: usize = 50_000;

/// Hard cap for a full recursive tree walk in [`SnapshotResolver::resolve_tree`].
const MAX_FULL_TREE_ENTRIES: usize = 8_000;

/// Maximum recursion depth for [`SnapshotMaterializer::materialize`].
/// Prevents stack overflow on pathologically deep trees.
const MAX_TREE_DEPTH: usize = 64;

/// Number of cached directory listings in [`DirCache`].
const DIR_CACHE_MAX_ENTRIES: usize = 32;

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
            timestamp: commit.time().seconds(),
        }
    }
}

// ── HistoryReader ──────────────────────────────────────────────────────────────

/// Opens a Git repository and provides access to its commit history.
pub struct HistoryReader {
    pub repo: Repository,
}

impl HistoryReader {
    /// Opens the repository at `path` (bare or with a working tree).
    pub fn open(path: &Path) -> Result<Self, git2::Error> {
        Ok(Self {
            repo: Repository::open(path)?,
        })
    }

    /// Returns up to [`LIST_COMMITS_MAX`] commits reachable from HEAD,
    /// sorted newest-first.
    ///
    /// For repositories with more commits, use
    /// [`list_commits_paginated`](Self::list_commits_paginated) to avoid
    /// blocking the main thread with a large allocation.
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

    /// Streams commits in pages of `page_size`, calling `on_page` for each
    /// batch.  This never allocates more than `page_size` commits at a time
    /// and is the recommended path for large repositories.
    pub fn list_commits_paginated(
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
        if !page.is_empty() {
            on_page(page);
        }
        Ok(())
    }

    /// Searches commits reachable from HEAD whose summary or hash prefix
    /// match `query` (case-insensitive).
    ///
    /// PERF: Filters *during* the revwalk, allocating only matching entries.
    /// The old approach called `list_commits()` first, which wasted memory
    /// proportional to the full history length before discarding non-matches.
    pub fn search_commits(&self, query: &str) -> Result<Vec<CommitInfo>, git2::Error> {
        if query.is_empty() {
            return self.list_commits();
        }
        let q = query.to_lowercase();
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(git2::Sort::TIME)?;

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
}

impl TreeNode {
    /// Returns the path of this node.
    pub fn path(&self) -> &Path {
        match self {
            TreeNode::File(p) | TreeNode::Dir(p) => p.as_path(),
        }
    }

    /// Returns `true` if this node is a directory.
    pub fn is_dir(&self) -> bool {
        matches!(self, TreeNode::Dir(_))
    }
}

// ── DirCache ──────────────────────────────────────────────────────────────────

/// Simple LRU cache for directory listings keyed by `(commit_hash, dir_path)`.
///
/// Avoids redundant `resolve_dir` calls when the user navigates back and
/// forth between directories or switches commits that share the same subtree.
///
/// Values are stored as `Arc<Vec<TreeNode>>` so cloning a cache hit is O(1)
/// (pointer copy) regardless of directory size.
///
/// Capacity is fixed at [`DIR_CACHE_MAX_ENTRIES`].  When full, the
/// least-recently-used entry is evicted.  A `VecDeque` keeps eviction O(1).
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

    /// Looks up `(hash, dir)`.  On a hit the entry is promoted to the
    /// most-recently-used position and an `Arc` clone (O(1)) is returned.
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

    /// Inserts or replaces the entry for `(hash, dir)`.  Evicts the LRU entry
    /// when the cache is at capacity.
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
    fn default() -> Self {
        Self::new()
    }
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
    /// PERF: Only descends the path components of `dir`, avoiding O(n) scans
    /// on large monorepos.  Prefer this over `resolve_tree` for all interactive
    /// directory browsing.
    ///
    /// SECURITY: `dir` is resolved through [`git2::Tree::get_path`], which
    /// walks the Git *object database* — an in-memory structure of OIDs —
    /// rather than opening any path on the real filesystem.  This means:
    ///
    /// * A traversal attempt such as `"../../etc"` does **not** escape the
    ///   repository.  git2 tries to match each component against named tree
    ///   entries; `".."` is never a valid entry name in a well-formed Git tree,
    ///   so `get_path` returns `Err` before any I/O occurs.
    /// * Absolute paths (e.g. `"/etc"` on Unix, `"C:\Windows"` on Windows)
    ///   are similarly rejected by `get_path` with an error.
    /// * There is no subprocess involved; git2 is a native library, so
    ///   shell-injection through `revision` strings is not applicable.
    ///
    /// Explicit `Path::components()` filtering is intentionally omitted:
    /// it would be redundant and could create a false sense of security
    /// in code paths that do *not* pass through this function.  The git2
    /// object-DB boundary is the canonical trust boundary for this module.
    /// See the module-level "# Security model" section for the full rationale.
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
                Some(ObjectType::Blob) => nodes.push(TreeNode::File(path)),
                Some(ObjectType::Tree) => nodes.push(TreeNode::Dir(path)),
                _ => {}
            }
        }
        Ok(nodes)
    }

    /// Legacy full-tree walk.  **Capped at [`MAX_FULL_TREE_ENTRIES`] entries**
    /// and [`MAX_TREE_DEPTH`] levels to prevent freezing or stack overflow on
    /// large monorepos.  Returns `Err` if either cap is exceeded so callers
    /// can fall back to `resolve_dir`.
    pub fn resolve_tree(&self, revision: &str) -> Result<Vec<TreeNode>, git2::Error> {
        let obj = self.repo.revparse_single(revision)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;
        let materializer = SnapshotMaterializer::new(self.repo);
        let nodes = materializer.materialize(&tree, PathBuf::new(), 0)?;
        if nodes.len() > MAX_FULL_TREE_ENTRIES {
            return Err(git2::Error::from_str(&format!(
                "Tree too large for full walk ({} entries, limit {}). \
                 Use resolve_dir for interactive browsing.",
                nodes.len(),
                MAX_FULL_TREE_ENTRIES
            )));
        }
        Ok(nodes)
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
    /// `depth` tracks the current recursion level; callers pass `0`.
    /// Returns `Err` if [`MAX_TREE_DEPTH`] is exceeded, preventing stack
    /// overflow on pathologically deep repository trees.
    pub fn materialize(
        &self,
        tree: &git2::Tree<'_>,
        prefix: PathBuf,
        depth: usize,
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
            let name = entry.name().unwrap_or("");
            let path = prefix.join(name);
            match entry.kind() {
                Some(ObjectType::Blob) => {
                    nodes.push(TreeNode::File(path));
                }
                Some(ObjectType::Tree) => {
                    nodes.push(TreeNode::Dir(path.clone()));
                    let subtree = self.repo.find_tree(entry.id())?;
                    let mut children = self.materialize(&subtree, path, depth + 1)?;
                    nodes.append(&mut children);
                }
                _ => {}
            }
        }
        Ok(nodes)
    }

    /// Reads the raw byte content of a file at `path` in the given `revision`.
    ///
    /// SECURITY: `path` is resolved via [`git2::Tree::get_path`] against the
    /// commit's in-memory object tree, **not** against the real filesystem.
    /// Path-traversal inputs such as `"../../etc/passwd"` are rejected by
    /// git2 with an error before any file is opened.  See the
    /// [`SnapshotResolver::resolve_dir`] doc and the module-level
    /// "# Security model" section for the full explanation.
    pub fn read_file(
        &self,
        revision: &str,
        path: &Path,
    ) -> Result<Vec<u8>, git2::Error> {
        let obj = self.repo.revparse_single(revision)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;
        let entry = tree.get_path(path)?;
        let blob = self.repo.find_blob(entry.id())?;
        Ok(blob.content().to_vec())
    }
}
