/* util.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Small formatting and GTK helpers shared by the window modules.

use super::*;


/// Remove all children from a `gtk::Box`.
#[allow(dead_code)]
pub(super) fn commit_touches_path(
    repo: &git2::Repository,
    commit: &git2::Commit<'_>,
    path: &std::path::Path,
    is_dir: bool,
) -> bool {
    let Ok(tree) = commit.tree() else {
        return false;
    };

    let parent_tree = commit.parent(0).ok().and_then(|parent| parent.tree().ok());

    let Ok(diff) = repo.diff_tree_to_tree(parent_tree.as_ref(), Some(&tree), None) else {
        return false;
    };

    for delta in diff.deltas() {
        let old_path = delta.old_file().path();
        let new_path = delta.new_file().path();

        let matches = |candidate: Option<&std::path::Path>| -> bool {
            let Some(candidate) = candidate else {
                return false;
            };

            if is_dir {
                candidate == path || candidate.starts_with(path)
            } else {
                candidate == path
            }
        };

        if matches(old_path) || matches(new_path) {
            return true;
        }
    }

    false
}

pub(super) fn format_git_mode(mode: i32) -> String {
    match mode {
        0o040000 => "040000 · Directory".to_string(),
        0o100644 => "100644 · Read/write file".to_string(),
        0o100755 => "100755 · Executable file".to_string(),
        0o120000 => "120000 · Symbolic link".to_string(),
        0o160000 => "160000 · Git submodule".to_string(),
        other => format!("{other:o} · Git mode"),
    }
}

pub(super) fn format_file_size(size: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

    let size_f = size as f64;

    if size_f >= GIB {
        format!("{:.1} GB", size_f / GIB)
    } else if size_f >= MIB {
        format!("{:.1} MB", size_f / MIB)
    } else if size_f >= KIB {
        format!("{:.1} KB", size_f / KIB)
    } else {
        format!("{size} B")
    }
}

pub(super) fn is_thumbnail_image_path(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "tif" | "tiff" | "heic" | "avif"
    )
}

pub(super) fn texture_for_tree_file(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
    path: &Path,
    max_bytes: usize,
) -> Option<gdk::Texture> {
    let entry = tree.get_path(path).ok()?;
    if entry.kind() != Some(git2::ObjectType::Blob) {
        return None;
    }

    let blob = repo.find_blob(entry.id()).ok()?;
    if blob.size() > max_bytes {
        return None;
    }

    let bytes = glib::Bytes::from_owned(blob.content().to_vec());
    gdk::Texture::from_bytes(&bytes).ok()
}

pub(super) fn clear_box(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

pub(super) fn collect_branch_commit_index(
    repo: &git2::Repository,
) -> (Vec<String>, HashMap<String, HashSet<String>>) {
    let mut branch_names = Vec::new();
    let mut index: HashMap<String, HashSet<String>> = HashMap::new();

    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else {
        return (branch_names, index);
    };

    for branch_result in branches {
        let Ok((branch, _)) = branch_result else {
            continue;
        };

        let Ok(Some(name)) = branch.name() else {
            continue;
        };

        let Some(target) = branch.get().target() else {
            continue;
        };

        let name = name.to_string();
        let mut hashes = HashSet::new();

        if let Ok(mut walk) = repo.revwalk() {
            if walk.push(target).is_ok() {
                for oid in walk.flatten() {
                    hashes.insert(oid.to_string());
                }
            }
        }

        branch_names.push(name.clone());
        index.insert(name, hashes);
    }

    branch_names.sort();
    branch_names.dedup();

    (branch_names, index)
}

/// Returns the canonical short prefix of a full commit hash.
#[inline]
pub(super) fn short_hash(hash: &str) -> &str {
    &hash[..SHORT_HASH_LEN.min(hash.len())]
}

/// Returns the short hash used in visible UI labels.
#[inline]
pub(super) fn display_hash(hash: &str) -> &str {
    short_hash(hash)
}

const SHORT_HASH_LEN: usize = 8;

impl TemporalExplorerWindow {
    pub(super) fn format_timestamp(ts: i64) -> String {
        glib::DateTime::from_unix_local(ts)
            .and_then(|dt| dt.format("%Y-%m-%d %H:%M"))
            .map(|s| s.to_string())
            .unwrap_or_else(|_| ts.to_string())
    }
}
