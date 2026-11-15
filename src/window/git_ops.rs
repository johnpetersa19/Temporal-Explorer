/* git_ops.rs
 *
 * Copyright 2026 John Peter Sá
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

//! Git operations that run outside the GTK main loop (clone, cherry-pick, restore).

use super::*;
use super::util::*;


pub(super) fn clone_git_repository(
    url: String,
    destination: PathBuf,
    cancel: Arc<AtomicBool>,
    progress: Arc<std::sync::atomic::AtomicUsize>,
    include_files: bool,
) -> Result<PathBuf, String> {
    fn clone_once(
        url: &str,
        destination: &Path,
        cancel: &Arc<AtomicBool>,
        progress: &Arc<std::sync::atomic::AtomicUsize>,
        include_files: bool,
    ) -> Result<(), git2::Error> {
        let config = git2::Config::open_default().ok();
        let ssh_keys: Vec<PathBuf> = std::env::var_os("HOME")
            .map(PathBuf::from)
            .into_iter()
            .flat_map(|home| {
                ["id_ed25519", "id_ecdsa", "id_rsa"]
                    .into_iter()
                    .map(move |name| home.join(".ssh").join(name))
            })
            .filter(|path| path.is_file())
            .collect();
        let mut ssh_attempt = 0usize;

        let mut callbacks = git2::RemoteCallbacks::new();
        let cancel_for_transfer = cancel.clone();
        let progress_for_transfer = progress.clone();
        callbacks.transfer_progress(move |stats| {
            let total = stats.total_objects();
            if total > 0 {
                // Keep 100% for the point at which checkout has actually finished.
                let percentage = (stats.received_objects() * 100 / total).min(99);
                progress_for_transfer.store(percentage, Ordering::Relaxed);
            }
            !cancel_for_transfer.load(Ordering::Relaxed)
        });
        callbacks.credentials(move |remote_url, username, allowed| {
            let username = username.unwrap_or("git");

            if allowed.contains(git2::CredentialType::SSH_KEY) {
                // Prefer the desktop session's ssh-agent. Flatpak exposes it through
                // --socket=ssh-auth, so encrypted keys never need to be read here.
                if ssh_attempt == 0 {
                    ssh_attempt += 1;
                    return git2::Cred::ssh_key_from_agent(username);
                }

                if let Some(private_key) = ssh_keys.get(ssh_attempt - 1) {
                    ssh_attempt += 1;
                    return git2::Cred::ssh_key(username, None, private_key, None);
                }
            }

            if allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
                if let Some(config) = config.as_ref() {
                    return git2::Cred::credential_helper(config, remote_url, Some(username));
                }
            }

            if allowed.contains(git2::CredentialType::USERNAME) {
                return git2::Cred::username(username);
            }

            if allowed.contains(git2::CredentialType::DEFAULT) {
                return git2::Cred::default();
            }

            Err(git2::Error::from_str("no supported credentials available"))
        });

        let mut fetch_options = git2::FetchOptions::new();
        fetch_options.remote_callbacks(callbacks);

        let mut builder = git2::build::RepoBuilder::new();
        builder.fetch_options(fetch_options);
        builder.bare(!include_files);
        builder.clone(url, destination).map(|_| ())
    }

    fn ssh_to_https(url: &str) -> Option<String> {
        if let Some(rest) = url.strip_prefix("ssh://") {
            let rest = rest
                .split_once('@')
                .map_or(rest, |(_, host_path)| host_path);
            let (host, path) = rest.split_once('/')?;
            return Some(format!("https://{host}/{path}"));
        }

        let (user_host, path) = url.split_once(':')?;
        let (_, host) = user_host.split_once('@')?;
        Some(format!("https://{host}/{path}"))
    }

    let result =
        clone_once(&url, &destination, &cancel, &progress, include_files).or_else(|ssh_error| {
            // Public repositories are often copied from hosting sites using their SSH
            // URL. If no SSH identity is configured, HTTPS can still open them without
            // turning the Open action into an authentication prompt.
            let Some(https_url) = ssh_to_https(&url) else {
                return Err(ssh_error);
            };
            if ssh_error.code() != git2::ErrorCode::Auth {
                return Err(ssh_error);
            }

            if destination.exists() {
                std::fs::remove_dir_all(&destination).map_err(|err| {
                    git2::Error::from_str(&format!("failed to clean partial clone: {err}"))
                })?;
            }
            progress.store(0, Ordering::Relaxed);
            clone_once(&https_url, &destination, &cancel, &progress, include_files)
        });

    result
        .map(|_| {
            progress.store(100, Ordering::Relaxed);
            destination
        })
        .map_err(|err| {
            if cancel.load(Ordering::Relaxed) {
                gettext("Clone operation cancelled")
            } else {
                format!("{}: {err}", gettext("Failed to clone repository"))
            }
        })
}

pub(super) fn cherry_pick_commits(repo_path: &Path, shas: &[String], signoff: bool) -> Result<usize, String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.to_string())?;

    if repo.is_bare() {
        return Err(gettext("Cannot cherry-pick in a bare repository"));
    }
    if repo.head_detached().map_err(|e| e.to_string())? {
        return Err(gettext("Cannot cherry-pick while HEAD is detached"));
    }
    if !repo.statuses(None).map_err(|e| e.to_string())?.is_empty() {
        return Err(gettext(
            "Commit or stash local changes before running cherry-pick",
        ));
    }

    let signature = repo.signature().map_err(|_| {
        gettext("Configure your Git user name and email before running cherry-pick")
    })?;
    let mut applied = 0usize;

    // History is displayed newest-first; replay selected commits oldest-first.
    for sha in shas.iter().rev() {
        let oid = git2::Oid::from_str(sha).map_err(|e| e.to_string())?;
        let commit = repo.find_commit(oid).map_err(|e| e.to_string())?;
        let head = repo
            .head()
            .and_then(|head| head.peel_to_commit())
            .map_err(|e| e.to_string())?;
        let mut index = repo
            .cherrypick_commit(&commit, &head, 0, None)
            .map_err(|e| e.to_string())?;

        if index.has_conflicts() {
            return Err(format!(
                "{} {}",
                gettext("Cherry-pick has conflicts at commit"),
                short_hash(sha)
            ));
        }

        let tree_id = index.write_tree_to(&repo).map_err(|e| e.to_string())?;
        let tree = repo.find_tree(tree_id).map_err(|e| e.to_string())?;
        let mut message = commit
            .message()
            .unwrap_or(&commit.summary().unwrap_or(""))
            .to_string();

        if signoff {
            let name = signature.name().unwrap_or("");
            let email = signature.email().unwrap_or("");
            message = format!(
                "{}\n\nSigned-off-by: {} <{}>",
                message.trim_end(),
                name,
                email
            );
        }

        repo.commit(
            Some("HEAD"),
            &signature,
            &signature,
            &message,
            &tree,
            &[&head],
        )
        .map_err(|e| e.to_string())?;
        applied += 1;
    }

    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout.force();
    repo.checkout_head(Some(&mut checkout))
        .map_err(|e| e.to_string())?;

    Ok(applied)
}

/// Restores the working tree from `revision` without moving HEAD or changing
/// the current branch. The index is deliberately left untouched, so the
/// restored snapshot appears as ordinary, unstaged working-tree changes.
pub(super) fn restore_project_files(repo_path: &Path, revision: &str) -> Result<(), String> {
    let repo = git2::Repository::open(repo_path).map_err(|e| e.to_string())?;

    if repo.is_bare() {
        return Err(gettext("Cannot restore files in a bare repository"));
    }
    if !repo.statuses(None).map_err(|e| e.to_string())?.is_empty() {
        return Err(gettext(
            "Commit or stash local changes before restoring project files",
        ));
    }

    let commit = repo
        .revparse_single(revision)
        .and_then(|object| object.peel_to_commit())
        .map_err(|e| e.to_string())?;
    let tree = commit.tree().map_err(|e| e.to_string())?;

    let mut checkout = git2::build::CheckoutBuilder::new();
    checkout
        .safe()
        .recreate_missing(true)
        .remove_untracked(false)
        .remove_ignored(false)
        .update_index(false);
    repo.checkout_tree(tree.as_object(), Some(&mut checkout))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod cherry_pick_tests {
    use super::{cherry_pick_commits, restore_project_files};
    use git2::{Repository, Signature};
    use std::path::Path;

    #[test]
    fn cherry_pick_replays_an_unattached_commit_onto_head() {
        let dir = tempfile::tempdir().expect("temporary repository");
        let repo = Repository::init(dir.path()).expect("initialize repository");
        let mut config = repo.config().expect("repository config");
        config.set_str("user.name", "Temporal Test").unwrap();
        config
            .set_str("user.email", "temporal@example.invalid")
            .unwrap();

        let signature = Signature::now("Temporal Test", "temporal@example.invalid").unwrap();
        let empty_tree_id = repo.treebuilder(None).unwrap().write().unwrap();
        let empty_tree = repo.find_tree(empty_tree_id).unwrap();
        let initial_id = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "Initial commit",
                &empty_tree,
                &[],
            )
            .unwrap();
        let initial = repo.find_commit(initial_id).unwrap();

        let blob_id = repo.blob(b"new content\n").unwrap();
        let mut builder = repo.treebuilder(Some(&empty_tree)).unwrap();
        builder.insert("new.txt", blob_id, 0o100644).unwrap();
        let feature_tree_id = builder.write().unwrap();
        let feature_tree = repo.find_tree(feature_tree_id).unwrap();
        let feature_id = repo
            .commit(
                None,
                &signature,
                &signature,
                "Add new file",
                &feature_tree,
                &[&initial],
            )
            .unwrap();

        drop(builder);
        drop(feature_tree);
        drop(initial);
        drop(empty_tree);
        drop(repo);

        let applied = cherry_pick_commits(dir.path(), &[feature_id.to_string()], false)
            .expect("cherry-pick succeeds");
        assert_eq!(applied, 1);

        let repo = Repository::open(dir.path()).unwrap();
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head.summary(), Some("Add new file"));
        assert!(head.tree().unwrap().get_name("new.txt").is_some());
    }

    #[test]
    fn restore_project_files_keeps_head_and_leaves_unstaged_changes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let signature = Signature::now("Test", "test@example.com").unwrap();

        std::fs::write(dir.path().join("file.txt"), "old\n").unwrap();
        std::fs::write(dir.path().join("old-only.txt"), "old\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.add_path(Path::new("old-only.txt")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let old = repo
            .commit(Some("HEAD"), &signature, &signature, "Old", &tree, &[])
            .unwrap();
        drop(tree);

        std::fs::write(dir.path().join("file.txt"), "new\n").unwrap();
        std::fs::remove_file(dir.path().join("old-only.txt")).unwrap();
        std::fs::write(dir.path().join("new-only.txt"), "new\n").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("file.txt")).unwrap();
        index.remove_path(Path::new("old-only.txt")).unwrap();
        index.add_path(Path::new("new-only.txt")).unwrap();
        let tree_id = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let parent = repo.find_commit(old).unwrap();
        let head = repo
            .commit(
                Some("HEAD"),
                &signature,
                &signature,
                "New",
                &tree,
                &[&parent],
            )
            .unwrap();
        drop(tree);
        drop(parent);
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();

        restore_project_files(dir.path(), &old.to_string()).unwrap();

        assert_eq!(repo.head().unwrap().target(), Some(head));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "old\n"
        );
        assert!(dir.path().join("old-only.txt").exists());
        assert!(!dir.path().join("new-only.txt").exists());
        let statuses = repo.statuses(None).unwrap();
        assert!(!statuses.is_empty());
        assert!(statuses.iter().all(|entry| !entry.status().is_index_new()
            && !entry.status().is_index_modified()
            && !entry.status().is_index_deleted()));
    }
}

