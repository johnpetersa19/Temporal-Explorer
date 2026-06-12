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

//! Git back-end: repository opening, history walking, tree resolving,
//! and file materialisation.
//!
//! All types in this module are pure Rust with no GTK dependency so
//! they can be unit-tested without a display connection.
//!
//! # Module layout
//!
//! | Type | Role |
//! |------|------|
//! | [`CommitInfo`] | Plain-old data transferred to the UI |
//! | [`HistoryReader`] | Walks the default-branch commit list |
//! | [`SnapshotResolver`] | Resolves a commit hash → directory tree |
//! | [`SnapshotMaterializer`] | Reads raw file bytes from a commit |
//! | [`DirCache`] | LRU cache for resolved directory listings |

use gettextrs::gettext;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

// ── Public data types ────────────────────────────────────────────────────────

/// Lightweight commit summary passed to the UI.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub hash:      String,
    pub summary:   String,
    pub author:    String,
    pub timestamp: i64,
}

/// A node inside a resolved directory tree.
#[derive(Debug, Clone)]
pub enum TreeNode {
    Dir(PathBuf),
    File(PathBuf),
    Submodule(PathBuf),
}

impl TreeNode {
    pub fn path(&self) -> &Path {
        match self { Self::Dir(p) | Self::File(p) | Self::Submodule(p) => p }
    }
    pub fn is_dir(&self) -> bool { matches!(self, Self::Dir(_)) }
    pub fn is_submodule(&self) -> bool { matches!(self, Self::Submodule(_)) }
}

// ── LRU directory cache ──────────────────────────────────────────────────────

/// Maximum number of directory listings kept in memory.
const DIR_CACHE_CAP: usize = 64;

/// Simple LRU cache for `(hash, dir_path) → Vec<TreeNode>`.
///
/// Avoids re-running `git ls-tree` when the user navigates back to a
/// directory they already visited in the same session.
///
/// The eviction policy is "remove the oldest inserted entry" — a true
/// LRU would require a linked-hash-map but insertion-order is good
/// enough for a navigation cache where sequential access is the norm.
#[derive(Debug, Default)]
pub struct DirCache {
    map:         HashMap<(String, PathBuf), Vec<TreeNode>>,
    insert_order: Vec<(String, PathBuf)>,
}

impl DirCache {
    pub fn get(&self, hash: &str, dir: &Path) -> Option<&Vec<TreeNode>> {
        self.map.get(&(hash.to_owned(), dir.to_owned()))
    }

    pub fn insert(&mut self, hash: &str, dir: PathBuf, nodes: Vec<TreeNode>) {
        let key = (hash.to_owned(), dir);
        if self.map.contains_key(&key) {
            self.map.insert(key, nodes);
            return;
        }
        if self.map.len() >= DIR_CACHE_CAP {
            if let Some(oldest) = self.insert_order.first().cloned() {
                self.map.remove(&oldest);
                self.insert_order.remove(0);
            }
        }
        self.insert_order.push(key.clone());
        self.map.insert(key, nodes);
    }
}

// ── HistoryReader ────────────────────────────────────────────────────────────

/// Opens the repository and walks commit history on the default branch.
pub struct HistoryReader {
    repo: git2::Repository,
}

impl HistoryReader {
    /// Opens the Git repository at `path`.
    pub fn open(path: &Path) -> Result<Self, git2::Error> {
        Ok(Self { repo: git2::Repository::open(path)? })
    }

    /// Returns all commits reachable from HEAD, newest first.
    ///
    /// Each commit is converted into a [`CommitInfo`] before being pushed
    /// into the result list — the `git2::Commit` lifetime is tied to the
    /// `git2::Repository` borrow and cannot be stored directly.
    pub fn list_commits(&self) -> Result<Vec<CommitInfo>, git2::Error> {
        let mut revwalk = self.repo.revwalk()?;
        revwalk.push_head()?;
        revwalk.set_sorting(git2::Sort::TIME)?;

        let mut result = Vec::new();
        for oid in revwalk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            result.push(CommitInfo {
                hash:      commit.id().to_string(),
                summary:   commit.summary().unwrap_or("").to_owned(),
                author:    commit.author().name().unwrap_or(&gettext("Unknown")).to_owned(),
                timestamp: commit.time().seconds(),
            });
        }
        Ok(result)
    }
}

// ── SnapshotResolver ─────────────────────────────────────────────────────────

/// Resolves a commit hash and a directory path into a list of [`TreeNode`]s.
pub struct SnapshotResolver<'a> {
    repo: &'a git2::Repository,
}

impl<'a> SnapshotResolver<'a> {
    pub fn new(repo: &'a git2::Repository) -> Self { Self { repo } }

    /// Returns the direct children of `dir` at commit `hash`.
    ///
    /// * Empty `dir` → root of the commit tree.
    /// * Submodules are detected via `gitlink` tree entries and returned as
    ///   [`TreeNode::Submodule`].
    /// * Entries that cannot be resolved to a tree or blob are silently
    ///   skipped so a single corrupted entry does not abort the listing.
    pub fn resolve_dir(
        &self,
        hash: &str,
        dir:  &Path,
    ) -> Result<Vec<TreeNode>, git2::Error> {
        let oid = git2::Oid::from_str(hash)?;
        let commit = self.repo.find_commit(oid)?;
        let tree   = commit.tree()?;

        let subtree: git2::Tree<'_> = if dir == Path::new("") {
            tree
        } else {
            let entry = tree.get_path(dir)?;
            self.repo.find_tree(entry.id())?
        };

        let mut dirs:  Vec<PathBuf> = Vec::new();
        let mut files: Vec<PathBuf> = Vec::new();
        let mut subs:  Vec<PathBuf> = Vec::new();

        // git2 tree entries are already sorted; we just partition them.
        // FileMode::Commit (0o160000) marks a submodule / gitlink entry.
        const GITLINK: i32 = 0o160000;

        let mut truncated = false;
        const ENTRY_LIMIT: usize = 50_000;
        let mut count = 0usize;

        subtree.walk(git2::TreeWalkMode::PreOrder, |root, entry| {
            if !root.is_empty() {
                return git2::TreeWalkResult::Skip;
            }
            if count >= ENTRY_LIMIT {
                truncated = true;
                return git2::TreeWalkResult::Abort;
            }
            count += 1;

            let name = match entry.name() {
                Some(n) => n,
                None    => return git2::TreeWalkResult::Ok,
            };
            let path = if dir == Path::new("") {
                PathBuf::from(name)
            } else {
                dir.join(name)
            };

            match entry.filemode() {
                m if m == GITLINK => subs.push(path),
                _ => match entry.kind() {
                    Some(git2::ObjectType::Tree) => dirs.push(path),
                    Some(git2::ObjectType::Blob) => files.push(path),
                    _ => {}
                },
            }
            git2::TreeWalkResult::Ok
        })?;

        if truncated {
            let reason = format!(">{ENTRY_LIMIT} entries");
            eprintln!("[git_engine] resolve_tree: tree truncated ({reason})");
        }

        dirs.sort_unstable();
        files.sort_unstable();
        subs.sort_unstable();

        let mut nodes: Vec<TreeNode> = Vec::with_capacity(dirs.len() + files.len() + subs.len());
        nodes.extend(subs.into_iter().map(TreeNode::Submodule));
        nodes.extend(dirs.into_iter().map(TreeNode::Dir));
        nodes.extend(files.into_iter().map(TreeNode::File));
        Ok(nodes)
    }
}

// ── SnapshotMaterializer ─────────────────────────────────────────────────────

/// Reads raw bytes of a file at a specific commit.
pub struct SnapshotMaterializer<'a> {
    repo: &'a git2::Repository,
}

impl<'a> SnapshotMaterializer<'a> {
    pub fn new(repo: &'a git2::Repository) -> Self { Self { repo } }

    /// Returns the raw bytes of `file_path` at commit `revision`.
    ///
    /// `revision` can be a full or abbreviated hash — it is resolved via
    /// [`git2::Repository::revparse_single`] before walking the tree.
    pub fn read_file(
        &self,
        revision:  &str,
        file_path: &Path,
    ) -> Result<Vec<u8>, git2::Error> {
        let obj    = self.repo.revparse_single(revision)?;
        let commit = obj.peel_to_commit()?;
        let tree   = commit.tree()?;
        let entry  = tree.get_path(file_path)?;
        let blob   = self.repo.find_blob(entry.id())?;
        Ok(blob.content().to_vec())
    }
}
