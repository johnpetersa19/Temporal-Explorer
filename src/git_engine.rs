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
//! The parallel commit path uses a single `collect_all_oids` pass to both
//! gather all OIDs and decide whether to parallelize, sharding them across
//! `CpuPool::io_threads()` workers (each opening their own handle) via a
//! shared `Arc<Vec<Oid>>` (zero copies of the OID list).  Results are merged
//! and re-sorted by timestamp before forwarding pages to the caller via
//! `into_iter().take()` (zero clones of `CommitInfo` on page delivery).
//! `on_page` is only ever called after all workers have joined successfully —
//! eliminating the duplicate-page bug that would occur if a panic triggered a
//! serial fallback mid-stream.
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
//!
//! # Performance — lazy `changed_files`
//!
//! `CommitInfo::changed_files` is **not** populated during the initial history
//! walk.  Computing a `diff_tree_to_tree` for every commit while streaming
//! tens of thousands of rows is O(N × diff_cost) and saturates the CPU.
//!
//! The field starts as an empty `Vec`.  Call [`CommitInfo::load_changed_files`]
//! with an open `Repository` handle when the diff is actually needed — for
//! example, when the user selects a commit in the sidebar or opens the
//! `FilterTypesDialog`.  The method is idempotent: it sets an internal
//! `files_loaded` flag on completion, so subsequent calls are no-ops even
//! for root commits that touched zero files (where `changed_files` stays
//! empty and the old `!is_empty()` check would have been incorrect).
//!
//! # Performance — raw OID in `CommitInfo`
//!
//! `CommitInfo` stores the raw [`git2::Oid`] (20 bytes, no heap allocation)
//! alongside the display `hash` string.  `load_changed_files` uses it directly
//! via [`CommitInfo::oid`], avoiding the `git2::Oid::from_str(&self.hash)`
//! parse that every earlier call site required.
//!
//! # Performance — single revwalk pass
//!
//! `list_commits_paginated` now collects all OIDs in a single revwalk pass and
//! checks the count in memory, replacing the old two-pass design
//! (`probe_oid_count` up to 2 001 steps + `collect_all_oids` for all N).
//! For a repository with 50 000 commits this saves one revwalk setup and
//! ~2 001 redundant OID reads on every load.
//!
//! # Performance — normalized text filters
//!
//! `CommitInfo` stores lowercase copies of the summary, author and e-mail so
//! repeated search predicates can run without allocating per row.

use gettextrs::gettext;
use git2::{DiffOptions, ObjectType, Repository, SubmoduleIgnore};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};

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

pub const FILE_CATEGORY_AUDIO: u16 = 1 << 0;
pub const FILE_CATEGORY_DOCUMENTS: u16 = 1 << 1;
pub const FILE_CATEGORY_FOLDERS: u16 = 1 << 2;
pub const FILE_CATEGORY_IMAGES: u16 = 1 << 3;
pub const FILE_CATEGORY_PDF: u16 = 1 << 4;
pub const FILE_CATEGORY_TEXT: u16 = 1 << 5;
pub const FILE_CATEGORY_VIDEOS: u16 = 1 << 6;

const FILE_INDEX_CACHE_VERSION: &str = "TEFI1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileIndexGroup {
    pub categories: u16,
    pub extension: String,
    pub count: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommitFileIndex {
    groups: Vec<FileIndexGroup>,
}

impl CommitFileIndex {
    pub fn from_paths(paths: &[String]) -> Self {
        let mut grouped: HashMap<(u16, String), u32> = HashMap::new();
        for path in paths {
            let path_obj = Path::new(path);
            let extension = path_obj
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let categories = categories_for_path(path_obj, &extension);
            *grouped.entry((categories, extension)).or_default() += 1;
        }
        let mut groups = grouped
            .into_iter()
            .map(|((categories, extension), count)| FileIndexGroup {
                categories,
                extension,
                count,
            })
            .collect::<Vec<_>>();
        groups.sort_unstable_by(|a, b| {
            a.extension
                .cmp(&b.extension)
                .then(a.categories.cmp(&b.categories))
        });
        Self { groups }
    }

    pub fn match_count(&self, categories: u16, extension: Option<&str>) -> usize {
        let wanted_extension =
            extension.map(|value| value.trim_start_matches('.').to_ascii_lowercase());
        self.groups
            .iter()
            .filter(|group| {
                group.categories & categories != 0
                    || wanted_extension
                        .as_ref()
                        .is_some_and(|wanted| group.extension == *wanted)
            })
            .map(|group| group.count as usize)
            .sum()
    }

    pub fn match_count_textual(
        &self,
        include_folders: bool,
        wanted_extensions: &[String],
    ) -> usize {
        self.groups
            .iter()
            .filter(|group| {
                (include_folders && group.categories & FILE_CATEGORY_FOLDERS != 0)
                    || wanted_extensions
                        .iter()
                        .any(|wanted| group.extension == *wanted)
            })
            .map(|group| group.count as usize)
            .sum()
    }

    fn encode(&self) -> String {
        self.groups
            .iter()
            .map(|group| {
                format!(
                    "{:x},{},{}",
                    group.categories,
                    hex_encode(group.extension.as_bytes()),
                    group.count
                )
            })
            .collect::<Vec<_>>()
            .join(";")
    }

    fn decode(encoded: &str) -> Option<Self> {
        if encoded.is_empty() {
            return Some(Self::default());
        }
        let mut groups = Vec::new();
        for raw_group in encoded.split(';') {
            let mut fields = raw_group.splitn(3, ',');
            groups.push(FileIndexGroup {
                categories: u16::from_str_radix(fields.next()?, 16).ok()?,
                extension: String::from_utf8(hex_decode(fields.next()?)?).ok()?,
                count: fields.next()?.parse().ok()?,
            });
        }
        Some(Self { groups })
    }
}

#[derive(Debug, Clone)]
pub struct FileIndexStore {
    cache_path: PathBuf,
    repository_id: String,
    entries: HashMap<git2::Oid, Arc<CommitFileIndex>>,
}

impl FileIndexStore {
    pub fn open(repo_path: &Path) -> Self {
        let repository_id = fs::canonicalize(repo_path)
            .unwrap_or_else(|_| repo_path.to_path_buf())
            .to_string_lossy()
            .into_owned();
        let cache_path = file_index_cache_path(&repository_id);
        let entries = load_file_index(&cache_path, &repository_id).unwrap_or_default();
        Self {
            cache_path,
            repository_id,
            entries,
        }
    }

    pub fn snapshot(&self) -> HashMap<git2::Oid, Arc<CommitFileIndex>> {
        self.entries.clone()
    }

    pub fn insert_many(&mut self, entries: Vec<(git2::Oid, CommitFileIndex)>) {
        for (oid, index) in entries {
            self.entries.insert(oid, Arc::new(index));
        }
    }

    pub fn retain_oids(&mut self, reachable: &HashMap<git2::Oid, usize>) {
        self.entries.retain(|oid, _| reachable.contains_key(oid));
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.cache_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temp_path = self.cache_path.with_extension("tmp");
        let mut file = fs::File::create(&temp_path)?;
        writeln!(
            file,
            "{FILE_INDEX_CACHE_VERSION}\t{}",
            hex_encode(self.repository_id.as_bytes())
        )?;
        let mut entries = self.entries.iter().collect::<Vec<_>>();
        entries.sort_unstable_by(|(left, _), (right, _)| left.as_bytes().cmp(right.as_bytes()));
        for (oid, index) in entries {
            writeln!(file, "{}\t{}", oid, index.encode())?;
        }
        file.flush()?;
        fs::rename(temp_path, &self.cache_path)
    }
}

fn categories_for_path(path: &Path, extension: &str) -> u16 {
    let mut categories = 0;
    if path
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
    {
        categories |= FILE_CATEGORY_FOLDERS;
    }
    if matches!(
        extension,
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "mid" | "midi"
    ) {
        categories |= FILE_CATEGORY_AUDIO;
    }
    if matches!(
        extension,
        "doc" | "docx" | "odt" | "ott" | "rtf" | "abw" | "pages"
    ) {
        categories |= FILE_CATEGORY_DOCUMENTS;
    }
    if matches!(
        extension,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" | "heic" | "avif"
    ) {
        categories |= FILE_CATEGORY_IMAGES;
    }
    if extension == "pdf" {
        categories |= FILE_CATEGORY_PDF;
    }
    if matches!(
        extension,
        "txt"
            | "md"
            | "markdown"
            | "rst"
            | "log"
            | "csv"
            | "json"
            | "jsonc"
            | "yaml"
            | "yml"
            | "toml"
            | "xml"
            | "html"
            | "css"
            | "scss"
            | "js"
            | "ts"
            | "jsx"
            | "tsx"
            | "rs"
            | "c"
            | "h"
            | "cpp"
            | "hpp"
            | "cc"
            | "py"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "go"
            | "java"
            | "kt"
            | "swift"
            | "php"
            | "rb"
            | "lua"
            | "blp"
            | "ui"
            | "desktop"
            | "service"
    ) {
        categories |= FILE_CATEGORY_TEXT;
    }
    if matches!(
        extension,
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "flv" | "wmv" | "mpeg" | "mpg"
    ) {
        categories |= FILE_CATEGORY_VIDEOS;
    }
    categories
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn hex_decode(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

fn file_index_cache_path(repository_id: &str) -> PathBuf {
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    repository_id.hash(&mut hasher);
    root.join("temporal-explorer")
        .join(format!("file-index-v1-{:016x}.cache", hasher.finish()))
}

fn load_file_index(
    path: &Path,
    repository_id: &str,
) -> std::io::Result<HashMap<git2::Oid, Arc<CommitFileIndex>>> {
    let file = fs::File::open(path)?;
    let mut lines = BufReader::new(file).lines();
    let Some(header) = lines.next().transpose()? else {
        return Ok(HashMap::new());
    };
    let expected = format!(
        "{FILE_INDEX_CACHE_VERSION}\t{}",
        hex_encode(repository_id.as_bytes())
    );
    if header != expected {
        return Ok(HashMap::new());
    }
    let mut entries = HashMap::new();
    for line in lines.map_while(Result::ok) {
        let Some((raw_oid, raw_index)) = line.split_once('\t') else {
            continue;
        };
        let (Ok(oid), Some(index)) = (
            git2::Oid::from_str(raw_oid),
            CommitFileIndex::decode(raw_index),
        ) else {
            continue;
        };
        entries.insert(oid, Arc::new(index));
    }
    Ok(entries)
}

pub fn build_commit_file_index(
    repo: &Repository,
    oid: git2::Oid,
) -> Result<CommitFileIndex, git2::Error> {
    let commit = repo.find_commit(oid)?;
    let tree = commit.tree()?;
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None
    };
    let diff = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None)?;
    let mut paths = Vec::with_capacity(8);
    diff.foreach(
        &mut |delta, _| {
            if let Some(path) = delta.new_file().path().and_then(|path| path.to_str()) {
                paths.push(path.to_owned());
            }
            true
        },
        None,
        None,
        None,
    )?;
    paths.sort_unstable();
    paths.dedup();
    Ok(CommitFileIndex::from_paths(&paths))
}

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
        Some(ObjectType::Blob) => Some(TreeNode::File(path)),
        Some(ObjectType::Tree) => Some(TreeNode::Dir(path)),
        Some(ObjectType::Commit) => Some(TreeNode::Submodule(path)),
        _ => None,
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
    #[inline]
    pub fn logical_cores(self) -> usize {
        self.logical_cores
    }

    /// Returns the recommended number of CPU-bound worker threads
    /// (capped at [`MAX_WORKER_THREADS`]).
    ///
    /// Currently unused inside this crate — the active parallel path calls
    /// `io_threads` instead.  Kept as part of the public threading API for
    /// future CPU-bound work (e.g. diff generation, blame).
    #[allow(dead_code)]
    #[inline]
    pub fn worker_threads(self) -> usize {
        self.logical_cores.min(MAX_WORKER_THREADS)
    }

    #[inline]
    pub fn io_threads(self) -> usize {
        self.logical_cores.min(MAX_IO_THREADS)
    }

    #[inline]
    pub fn is_parallel(self) -> bool {
        self.logical_cores > 1
    }
}

impl Default for CpuPool {
    fn default() -> Self {
        Self::detect()
    }
}

// ── CommitInfo ───────────────────────────────────────────────────────────────────────────

/// Lightweight representation of a single commit, used to populate the UI list.
///
/// `changed_files` starts **empty** and is only populated on demand by
/// [`CommitInfo::load_changed_files`].  This avoids running a full
/// `diff_tree_to_tree` for every commit during the initial history walk,
/// which was the root cause of the 80 % CPU spike and slow load times.
///
/// # Stored OID
///
/// The raw `git2::Oid` is stored alongside `hash` so that git2 operations in
/// [`load_changed_files`] can call `repo.find_commit(self.oid)` directly,
/// without re-parsing the hex `hash` string on every invocation.
/// `git2::Oid` is `[u8; 20]` — `Copy + Send + Sync`, no heap allocation.
/// Access it via [`CommitInfo::oid()`].
#[derive(Debug, Clone)]
pub struct CommitInfo {
    /// Raw OID — avoids re-parsing `hash` in [`load_changed_files`].
    /// Private to preserve struct-literal stability for existing consumers.
    oid: git2::Oid,
    /// Full 40-character SHA-1 hash.
    pub hash: String,
    /// First line of the commit message (summary).
    pub summary: String,
    summary_lower: String,
    /// Author name.
    pub author: String,
    author_lower: String,
    /// Author e-mail address.  Useful for deduplicating authors with different
    /// display names and for generating Gravatar avatar URLs.
    pub author_email: String,
    author_email_lower: String,
    /// Unix timestamp (seconds since epoch).
    pub timestamp: i64,
    /// Files changed by this commit (extensions used by `FilterTypesDialog`).
    ///
    /// Populated lazily on first call to [`load_changed_files`].
    /// Empty until that call is made.
    pub changed_files: Vec<String>,
    /// Whether [`load_changed_files`] has been called and completed.
    ///
    /// Using an explicit flag instead of `!changed_files.is_empty()` is
    /// necessary for root commits that modify zero files: their diff is empty
    /// by definition, so `is_empty()` would be `true` even after the diff has
    /// been computed — causing spurious re-diffs on every subsequent call.
    files_loaded: bool,
}

impl CommitInfo {
    #[cfg(test)]
    pub(crate) fn for_test(
        hash: String,
        summary: String,
        author: String,
        author_email: String,
        timestamp: i64,
    ) -> Self {
        let summary_lower = summary.to_lowercase();
        let author_lower = author.to_lowercase();
        let author_email_lower = author_email.to_lowercase();
        Self {
            oid: git2::Oid::zero(),
            hash,
            summary,
            summary_lower,
            author,
            author_lower,
            author_email,
            author_email_lower,
            timestamp,
            changed_files: Vec::new(),
            files_loaded: false,
        }
    }

    /// Returns the raw [`git2::Oid`] for this commit.
    ///
    /// Prefer this over `git2::Oid::from_str(&commit.hash)` when passing
    /// the OID to git2 functions — it avoids the hex-to-bytes parse and the
    /// potential (though unlikely) `Err` branch.
    #[inline]
    pub fn oid(&self) -> git2::Oid {
        self.oid
    }

    // ── Private fast constructor ────────────────────────────────────────────────
    //
    // Collects only the lightweight scalar fields. No diff is computed.
    // Used by every code path that walks the history (serial, parallel, search).
    #[inline]
    fn from_commit_fast(commit: &git2::Commit<'_>) -> Self {
        let summary = commit.summary().unwrap_or("").to_owned();
        let author = commit
            .author()
            .name()
            .unwrap_or(&gettext("Unknown"))
            .to_owned();
        let author_email = commit.author().email().unwrap_or("").to_owned();
        Self {
            oid: commit.id(),
            hash: commit.id().to_string(),
            summary_lower: summary.to_lowercase(),
            summary,
            author_lower: author.to_lowercase(),
            author,
            author_email_lower: author_email.to_lowercase(),
            author_email,
            timestamp: commit.time().seconds(),
            changed_files: Vec::new(),
            files_loaded: false,
        }
    }

    // ── Legacy constructor (kept for call-site compatibility) ───────────────────
    //
    // Accepts `repo` for signature compatibility with the old API but no longer
    // uses it — the diff is deferred to `load_changed_files`. All internal call
    // sites have been updated to `from_commit_fast`; this wrapper exists so that
    // any external consumers that already call `CommitInfo::from_commit` continue
    // to compile without changes.
    #[allow(dead_code)]
    pub(crate) fn from_commit(commit: &git2::Commit<'_>, _repo: &git2::Repository) -> Self {
        Self::from_commit_fast(commit)
    }

    /// Computes and caches the list of files changed by this commit.
    ///
    /// This method is **idempotent**: once called, it sets an internal
    /// `files_loaded` flag and returns immediately on all subsequent calls
    /// without opening any git objects — even for root commits whose diff
    /// is empty (where `changed_files` stays empty).
    ///
    /// Call this lazily — only when the diff is actually needed, for example:
    /// - when the user clicks a commit row to inspect it, or
    /// - when `FilterTypesDialog` needs to filter commits by file extension.
    ///
    /// For merge commits the diff is computed against the **first parent only**,
    /// matching the behaviour of `git show` / `git log -p` and avoiding the
    /// false-positive duplicates that arise from diffing all N parents.
    pub fn load_changed_files(&mut self, repo: &git2::Repository) {
        let _ = self.load_changed_files_result(repo);
    }

    /// Computes and caches the list of files changed by this commit, returning
    /// any git2 error to callers that need user-visible diagnostics.
    pub fn load_changed_files_result(
        &mut self,
        repo: &git2::Repository,
    ) -> Result<(), git2::Error> {
        // Idempotent — skip if the diff has already been computed.
        // Using a dedicated flag instead of `!changed_files.is_empty()` is
        // necessary for root commits that touch zero files.
        if self.files_loaded {
            return Ok(());
        }

        // Use the stored OID directly — no hex re-parse needed.
        let commit = repo.find_commit(self.oid)?;
        let tree = commit.tree()?;

        // Pre-size for a typical commit; avoids the first 3–4 reallocations.
        let mut files = Vec::with_capacity(8);

        if commit.parent_count() > 0 {
            // Regular commit or merge commit — diff against first parent only.
            // See module-level doc for the rationale.
            let parent = commit.parent(0)?;
            let parent_tree = parent.tree()?;
            let diff = repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), None)?;
            diff.foreach(
                &mut |delta, _| {
                    if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                        files.push(path.to_owned());
                    }
                    true
                },
                None,
                None,
                None,
            )?;
        } else {
            // Root commit — diff against empty tree.
            let diff = repo.diff_tree_to_tree(None, Some(&tree), None)?;
            diff.foreach(
                &mut |delta, _| {
                    if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                        files.push(path.to_owned());
                    }
                    true
                },
                None,
                None,
                None,
            )?;
        }

        files.sort();
        files.dedup();
        self.set_changed_files_loaded(files);
        Ok(())
    }

    /// Returns changed files that match Git pathspecs without marking the full
    /// changed-file list as loaded. Used by file-type search to let libgit2
    /// skip unrelated paths before Rust applies the final category check.
    pub fn changed_files_matching_pathspecs(
        &self,
        repo: &git2::Repository,
        pathspecs: &[String],
    ) -> Result<Vec<String>, git2::Error> {
        let commit = repo.find_commit(self.oid)?;
        let tree = commit.tree()?;
        let mut opts = DiffOptions::new();

        for pathspec in pathspecs {
            opts.pathspec(pathspec);
        }

        let diff = if commit.parent_count() > 0 {
            let parent = commit.parent(0)?;
            let parent_tree = parent.tree()?;
            repo.diff_tree_to_tree(Some(&parent_tree), Some(&tree), Some(&mut opts))?
        } else {
            repo.diff_tree_to_tree(None, Some(&tree), Some(&mut opts))?
        };

        let mut files = Vec::with_capacity(4);
        diff.foreach(
            &mut |delta, _| {
                if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                    files.push(path.to_owned());
                }
                true
            },
            None,
            None,
            None,
        )?;

        files.sort();
        files.dedup();
        Ok(files)
    }

    /// Rehydrates changed-file metadata from an external cache.
    ///
    /// This deliberately sets the same loaded flag as `load_changed_files()` so
    /// commits with zero changed files are not diffed repeatedly.
    pub fn set_changed_files_loaded(&mut self, mut files: Vec<String>) {
        files.sort();
        files.dedup();
        self.changed_files = files;
        self.files_loaded = true;
    }

    /// Returns `true` if `changed_files` has already been loaded.
    ///
    /// Uses the dedicated `files_loaded` flag — not `!changed_files.is_empty()` —
    /// so that root commits with an empty diff also return `true` after
    /// [`load_changed_files`] has been called.
    ///
    /// Useful for UI code that wants to show a spinner or placeholder until
    /// the diff is available, without triggering the load itself.
    #[inline]
    pub fn has_changed_files_loaded(&self) -> bool {
        self.files_loaded
    }

    /// Returns true when the commit matches a lowercase text query.
    #[inline]
    pub fn matches_text_query(&self, query: &str) -> bool {
        self.hash.starts_with(query)
            || self.summary_lower.contains(query)
            || self.author_lower.contains(query)
            || self.author_email_lower.contains(query)
    }

    /// Returns true when the commit author matches a lowercase query.
    #[inline]
    pub fn author_matches_query(&self, query: &str) -> bool {
        self.author_lower.contains(query)
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
        let url = sm.url().unwrap_or("").to_owned();
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

        infos.push(SubmoduleInfo {
            name,
            url,
            path,
            status,
        });
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
    pub fn repo(&self) -> &Repository {
        &self.repo
    }

    /// Returns the [`CpuPool`] detected at construction time.
    ///
    /// # Note — currently unused inside this crate
    ///
    /// See the note on [`HistoryReader::repo`].
    #[allow(dead_code)]
    pub fn cpu_pool(&self) -> CpuPool {
        self.cpu_pool
    }

    /// Returns up to [`LIST_COMMITS_MAX`] commits reachable from HEAD,
    /// sorted newest-first.
    ///
    /// `changed_files` is **not** populated here — call
    /// [`CommitInfo::load_changed_files`] on individual commits when needed.
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
            commits.push(CommitInfo::from_commit_fast(&commit));
            if commits.len() >= LIST_COMMITS_MAX {
                break;
            }
        }
        Ok(commits)
    }

    /// Streams commits in pages of `page_size`, calling `on_page` for each batch.
    ///
    /// Uses a single `collect_all_oids` pass (not `page_size`) as the basis for
    /// deciding whether to parallelize — a single revwalk open and N OID reads,
    /// replacing the old two-pass design (`probe_oid_count` up to 2 001 steps
    /// followed by a separate `collect_all_oids` of all N).
    ///
    /// `changed_files` is **not** populated — call
    /// [`CommitInfo::load_changed_files`] on individual commits when the diff
    /// is actually needed.
    pub fn list_commits_paginated(
        &self,
        page_size: usize,
        on_page: impl FnMut(Vec<CommitInfo>),
    ) -> Result<(), git2::Error> {
        if self.cpu_pool.is_parallel() {
            // Single revwalk pass: collect all OIDs, then decide in memory.
            // This replaces the old probe_oid_count(2001) + collect_all_oids(N)
            // pattern, saving one revwalk setup and up to 2 001 redundant OID
            // reads on every large-repository load.
            let oids = self.collect_all_oids()?;
            if oids.len() > MIN_COMMITS_FOR_PARALLEL {
                return self.list_commits_paginated_parallel(page_size, oids, on_page);
            }
            return self.list_commits_from_oids(page_size, oids, on_page);
        }
        self.list_commits_paginated_serial(page_size, on_page)
    }

    /// Materializes already-collected OIDs without opening a second revwalk.
    fn list_commits_from_oids(
        &self,
        page_size: usize,
        oids: Vec<git2::Oid>,
        mut on_page: impl FnMut(Vec<CommitInfo>),
    ) -> Result<(), git2::Error> {
        let page_size = page_size.max(1);
        let mut page = Vec::with_capacity(page_size);
        for oid in oids {
            let commit = self.repo.find_commit(oid)?;
            page.push(CommitInfo::from_commit_fast(&commit));
            if page.len() == page_size {
                on_page(std::mem::replace(&mut page, Vec::with_capacity(page_size)));
            }
        }
        if !page.is_empty() {
            on_page(page);
        }
        Ok(())
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
            page.push(CommitInfo::from_commit_fast(&commit));
            if page.len() >= page_size {
                on_page(std::mem::replace(&mut page, Vec::with_capacity(page_size)));
            }
        }
        if !page.is_empty() {
            on_page(page);
        }
        Ok(())
    }

    /// Collects all reachable OIDs from HEAD, newest-first.
    ///
    /// A conservative `with_capacity(4_096)` hint reduces reallocations on
    /// repos with thousands of commits without over-allocating on small ones
    /// (the Vec grows geometrically beyond 4 096 if needed).
    fn collect_all_oids(&self) -> Result<Vec<git2::Oid>, git2::Error> {
        let mut walk = self.repo.revwalk()?;
        push_head_safe(&mut walk, &self.repo)?;
        walk.set_sorting(git2::Sort::TIME)?;
        let mut oids = Vec::with_capacity(4_096);
        for oid in walk {
            oids.push(oid?);
        }
        Ok(oids)
    }

    /// Parallel paginated commit walk.
    ///
    /// Workers materialize page-sized OID ranges and send them to the caller as
    /// soon as they are ready. A small reorder buffer preserves the exact
    /// revwalk order without a global sort or an all-commits staging vector.
    ///
    /// # Zero-copy OID sharding
    ///
    /// The OID list is wrapped in `Arc<Vec<Oid>>` and shared across threads
    /// using index ranges — no per-shard copy of the OID bytes is made.
    /// For 50 000 commits this saves three extra 1 MB allocations (one per
    /// extra thread beyond the first).
    ///
    /// # Zero-clone page delivery
    ///
    /// Pages are yielded by draining `all` via `into_iter().take(page_size)`,
    /// so `CommitInfo` values (each carrying several `String` fields) are
    /// *moved*, not cloned, on page delivery.
    ///
    /// `changed_files` is intentionally **not** computed here — the parallel
    /// path spawns up to 4 threads each opening their own `Repository`; running
    /// a diff per commit per thread would multiply the already-expensive diff
    /// cost by the thread count, causing the 80 % CPU spike.
    fn list_commits_paginated_parallel(
        &self,
        page_size: usize,
        oids: Vec<git2::Oid>,
        mut on_page: impl FnMut(Vec<CommitInfo>),
    ) -> Result<(), git2::Error> {
        enum WorkerMessage {
            Page(usize, Vec<CommitInfo>),
            Error(String),
        }

        let page_size = page_size.max(1);
        let n_threads = self.cpu_pool.io_threads().max(2);
        let repo_path = self.repo.path().to_path_buf();
        let oid_count = oids.len();
        let page_count = oid_count.div_ceil(page_size);

        // Share the OID list across threads via Arc; git2::Oid is [u8; 20]
        // (Copy + Send + Sync), so Arc<Vec<Oid>> is sound without unsafe.
        let oids = Arc::new(oids);
        let next_page = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel::<WorkerMessage>();

        let handles: Vec<std::thread::JoinHandle<()>> = (0..n_threads)
            .map(|_| {
                let oids = Arc::clone(&oids);
                let next_page = Arc::clone(&next_page);
                let path = repo_path.clone();
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let repo = match Repository::open(&path) {
                        Ok(r) => r,
                        Err(error) => {
                            let _ = tx.send(WorkerMessage::Error(error.to_string()));
                            return;
                        }
                    };

                    loop {
                        let page_index = next_page.fetch_add(1, Ordering::Relaxed);
                        if page_index >= page_count {
                            break;
                        }
                        let start = page_index * page_size;
                        let end = (start + page_size).min(oid_count);
                        let mut results = Vec::with_capacity(end - start);
                        for &oid in &oids[start..end] {
                            match repo.find_commit(oid) {
                                Ok(commit) => results.push(CommitInfo::from_commit_fast(&commit)),
                                Err(error) => {
                                    let _ = tx.send(WorkerMessage::Error(error.to_string()));
                                    return;
                                }
                            }
                        }
                        if tx.send(WorkerMessage::Page(page_index, results)).is_err() {
                            return;
                        }
                    }
                })
            })
            .collect();
        drop(tx);

        let mut pending = BTreeMap::new();
        let mut next_to_emit = 0usize;
        while next_to_emit < page_count {
            match rx.recv() {
                Ok(WorkerMessage::Page(index, page)) => {
                    pending.insert(index, page);
                    while let Some(page) = pending.remove(&next_to_emit) {
                        on_page(page);
                        next_to_emit += 1;
                    }
                }
                Ok(WorkerMessage::Error(error)) => return Err(git2::Error::from_str(&error)),
                Err(_) => return Err(git2::Error::from_str("commit worker stopped unexpectedly")),
            }
        }

        for handle in handles {
            if handle.join().is_err() {
                return Err(git2::Error::from_str("commit worker panicked"));
            }
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
            // Do not pre-allocate SEARCH_COMMITS_MAX slots unconditionally:
            // for small repositories the Vec would be over-sized by orders of
            // magnitude.  Start with a modest hint and let it grow if needed.
            let mut results = Vec::with_capacity(256);
            for oid in walk {
                let oid = oid?;
                let commit = self.repo.find_commit(oid)?;
                results.push(CommitInfo::from_commit_fast(&commit));
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
            let oid_str = oid.to_string();
            let hash_match = oid_str.starts_with(&q);

            let commit = self.repo.find_commit(oid)?;
            let info = CommitInfo::from_commit_fast(&commit);

            if hash_match || info.matches_text_query(&q) {
                results.push(info);
                if results.len() >= SEARCH_COMMITS_MAX {
                    break;
                }
            }
        }
        Ok(results)
    }

    /// Extracts the underlying `git2::Repository` out of this reader.
    ///
    /// Used in `window.rs` to avoid opening the same repository path twice.
    ///
    /// # Note — currently unused inside this crate
    ///
    /// The active code path in `window.rs::load_repository` opens the repo
    /// independently via `git2::Repository::open`.  This accessor is preserved
    /// for a future refactor that routes all repository handles through
    /// `HistoryReader` to avoid redundant opens.
    #[allow(dead_code)]
    pub fn into_git2(self) -> Repository {
        self.repo
    }
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

    pub fn is_dir(&self) -> bool {
        matches!(self, TreeNode::Dir(_))
    }
    pub fn is_submodule(&self) -> bool {
        matches!(self, TreeNode::Submodule(_))
    }
}

// ── DirCache ───────────────────────────────────────────────────────────────────────────

/// Simple LRU cache for directory listings keyed by `(commit_hash, dir_path)`.
#[derive(Debug)]
pub struct DirCache {
    entries: VecDeque<((String, PathBuf), Arc<Vec<TreeNode>>)>,
}

impl DirCache {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(DIR_CACHE_MAX_ENTRIES),
        }
    }

    pub fn get(&mut self, hash: &str, dir: &Path) -> Option<Arc<Vec<TreeNode>>> {
        let pos = self
            .entries
            .iter()
            .position(|((h, d), _)| h == hash && d == dir)?;
        // Skip the remove+push_front dance when the entry is already at the
        // front (most-recently-used) — avoids a VecDeque shift for free hits.
        if pos == 0 {
            return Some(Arc::clone(&self.entries[0].1));
        }
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

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

impl Default for DirCache {
    fn default() -> Self {
        Self::new()
    }
}

// ── SnapshotResolver ──────────────────────────────────────────────────────────────────────────────

/// Resolves a commit hash (or any Git revision string) into a raw Git tree.
pub struct SnapshotResolver<'repo> {
    repo: &'repo Repository,
}

impl<'repo> SnapshotResolver<'repo> {
    pub fn new(repo: &'repo Repository) -> Self {
        Self { repo }
    }

    /// Resolves `revision` and materializes only the **direct children** of
    /// `dir` in the corresponding commit tree.
    ///
    /// Entries with missing or empty names are silently skipped.
    pub fn resolve_dir(&self, revision: &str, dir: &Path) -> Result<Vec<TreeNode>, git2::Error> {
        let commit = self.repo.revparse_single(revision)?.peel_to_commit()?;
        self.resolve_dir_oid(commit.id(), dir)
    }

    /// Resolves `oid` and materializes only the **direct children** of `dir`
    /// in the corresponding commit tree.
    pub fn resolve_dir_oid(
        &self,
        oid: git2::Oid,
        dir: &Path,
    ) -> Result<Vec<TreeNode>, git2::Error> {
        let commit = self.repo.find_commit(oid)?;
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
#[allow(dead_code)]
enum MaterializeOutcome {
    /// Walk completed without hitting any limit.
    Complete(Vec<TreeNode>),
    /// Walk was cut short at an entry or depth limit.
    Truncated(Vec<TreeNode>, String),
}

// ── SnapshotMaterializer ───────────────────────────────────────────────────────────────────────────────

/// Converts a raw Git tree object into a navigable list of [`TreeNode`]s.
pub struct SnapshotMaterializer<'repo> {
    repo: &'repo Repository,
}

impl<'repo> SnapshotMaterializer<'repo> {
    pub fn new(repo: &'repo Repository) -> Self {
        Self { repo }
    }

    /// Public entry point kept for API compatibility.
    ///
    /// Delegates to [`materialize_inner`] and maps [`MaterializeOutcome`] back
    /// to the original `Result<Vec<TreeNode>, git2::Error>` signature.
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

    /// Core recursive walk.
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
                        "entry limit ({limit}) reached — use resolve_dir for interactive browsing",
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
                    let subtree = self.repo.find_tree(entry.id())?;
                    let remaining = limit.saturating_sub(nodes.len());
                    match self.materialize_inner(&subtree, path, depth + 1, remaining)? {
                        MaterializeOutcome::Complete(mut children) => {
                            nodes.append(&mut children);
                        }
                        MaterializeOutcome::Truncated(mut children, reason) => {
                            nodes.append(&mut children);
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
    pub fn read_file(&self, revision: &str, path: &Path) -> Result<Vec<u8>, git2::Error> {
        let commit = self.repo.revparse_single(revision)?.peel_to_commit()?;
        self.read_file_oid(commit.id(), path)
    }

    /// Reads the raw byte content of a file at `path` in the given commit `oid`.
    pub fn read_file_oid(&self, oid: git2::Oid, path: &Path) -> Result<Vec<u8>, git2::Error> {
        let commit = self.repo.find_commit(oid)?;
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
