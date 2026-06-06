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
//! - [`HistoryReader`]       – opens a repository and iterates commits.
//! - [`CommitInfo`]          – lightweight commit data for the UI list.
//! - [`SnapshotResolver`]    – resolves a commit hash into a full tree.
//! - [`SnapshotMaterializer`]– converts the resolved tree into [`TreeNode`]s.
//! - [`TreeNode`]            – a single file or directory in a materialized snapshot.

use git2::{ObjectType, Repository};
use std::path::{Path, PathBuf};

// ── CommitInfo ───────────────────────────────────────────────────────────────

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

// ── HistoryReader ────────────────────────────────────────────────────────────

/// Opens a Git repository and provides access to its commit history.
///
/// # Example
/// ```no_run
/// let reader = HistoryReader::open(std::path::Path::new("/path/to/repo")).unwrap();
/// let commits = reader.list_commits().unwrap();
/// ```
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

    /// Returns all commits reachable from HEAD, sorted newest-first.
    pub fn list_commits(&self) -> Result<Vec<CommitInfo>, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        walk.push_head()?;
        walk.set_sorting(git2::Sort::TIME)?;

        let mut commits = Vec::new();
        for oid in walk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            commits.push(CommitInfo::from_commit(&commit));
        }
        Ok(commits)
    }

    /// Filters commits whose summary contains `query` (case-insensitive).
    pub fn search_commits(&self, query: &str) -> Result<Vec<CommitInfo>, git2::Error> {
        let q = query.to_lowercase();
        Ok(self
            .list_commits()?
            .into_iter()
            .filter(|c| c.summary.to_lowercase().contains(&q) || c.hash.starts_with(&q))
            .collect())
    }
}

// ── TreeNode ─────────────────────────────────────────────────────────────────

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

// ── SnapshotResolver ─────────────────────────────────────────────────────────

/// Resolves a commit hash (or any Git revision string) into a raw Git tree.
///
/// This is a thin wrapper that validates the revision and hands the tree
/// to [`SnapshotMaterializer`] for further processing.
pub struct SnapshotResolver<'repo> {
    repo: &'repo Repository,
}

impl<'repo> SnapshotResolver<'repo> {
    /// Creates a new resolver bound to `repo`.
    pub fn new(repo: &'repo Repository) -> Self {
        Self { repo }
    }

    /// Resolves `revision` (hash, branch, tag, `HEAD~3`, …) and materializes
    /// the corresponding file tree.
    ///
    /// Returns a flat, depth-first ordered list of [`TreeNode`]s.
    pub fn resolve_tree(&self, revision: &str) -> Result<Vec<TreeNode>, git2::Error> {
        let obj = self.repo.revparse_single(revision)?;
        let commit = obj.peel_to_commit()?;
        let tree = commit.tree()?;

        let materializer = SnapshotMaterializer::new(self.repo);
        materializer.materialize(&tree, PathBuf::new())
    }
}

// ── SnapshotMaterializer ─────────────────────────────────────────────────────

/// Converts a raw Git tree object into a navigable list of [`TreeNode`]s.
///
/// This is the core of the Temporal Explorer engine.  It walks the tree
/// recursively, turning every blob into a [`TreeNode::File`] and every
/// sub-tree into a [`TreeNode::Dir`], preserving the original directory
/// structure of the chosen revision.
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
    /// `prefix` is the accumulated path from the repository root to `tree`.
    pub fn materialize(
        &self,
        tree: &git2::Tree<'_>,
        prefix: PathBuf,
    ) -> Result<Vec<TreeNode>, git2::Error> {
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
                    let mut children = self.materialize(&subtree, path)?;
                    nodes.append(&mut children);
                }
                // Submodules (Commit) and other exotic objects are skipped.
                _ => {}
            }
        }

        Ok(nodes)
    }

    /// Reads the raw byte content of a file at `path` in the given `revision`.
    ///
    /// Useful for a future file-preview panel.
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
