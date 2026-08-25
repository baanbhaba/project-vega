// src/core/git.rs
//
// Fast Git status reader for directories inside a git repository.
// Uses `git2` (libgit2 Rust bindings) — no spawning `git` subprocesses.
// Results are cached per repository root to avoid repeated re-reads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use git2::{Repository, Status, StatusOptions};
use parking_lot::Mutex;
use once_cell::sync::Lazy;

use crate::core::entry::GitStatus;

/// Cache mapping repository root → (branch name, file status map).
/// `parking_lot::Mutex` is used for fast uncontended access.
static REPO_CACHE: Lazy<Mutex<HashMap<PathBuf, RepoCache>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

struct RepoCache {
    pub branch: String,
    /// Maps absolute file path → GitStatus.
    pub statuses: HashMap<PathBuf, GitStatus>,
}

/// Returns the git branch name and a status map for all files in `repo_root`.
/// Results are cached; call `invalidate_cache` after commits/checkouts.
pub fn get_repo_status(repo_root: &Path) -> Option<(String, HashMap<PathBuf, GitStatus>)> {
    {
        let cache = REPO_CACHE.lock();
        if let Some(entry) = cache.get(repo_root) {
            return Some((entry.branch.clone(), entry.statuses.clone()));
        }
    }

    // Not cached yet — read from libgit2
    let repo = Repository::open(repo_root).ok()?;
    let branch = current_branch(&repo);

    let mut opts = StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(false)
        .include_ignored(false)
        .include_unmodified(false);

    let statuses_raw = repo.statuses(Some(&mut opts)).ok()?;
    let mut statuses: HashMap<PathBuf, GitStatus> = HashMap::new();

    for entry in statuses_raw.iter() {
        let path = match entry.path() {
            Some(p) => repo_root.join(p),
            None => continue,
        };
        let gs = translate_status(entry.status());
        statuses.insert(path, gs);
    }

    let cache_entry = RepoCache {
        branch: branch.clone(),
        statuses: statuses.clone(),
    };
    REPO_CACHE.lock().insert(repo_root.to_path_buf(), cache_entry);

    Some((branch, statuses))
}

/// Find the git repository root for a given directory path (walks up the tree).
pub fn find_repo_root(dir: &Path) -> Option<PathBuf> {
    Repository::discover(dir)
        .ok()
        .and_then(|repo| repo.workdir().map(|p| p.to_path_buf()))
}

/// Invalidate the cache for a repo root (call after file ops or git operations).
pub fn invalidate_cache(repo_root: &Path) {
    REPO_CACHE.lock().remove(repo_root);
}

fn current_branch(repo: &Repository) -> String {
    repo.head()
        .ok()
        .and_then(|h| h.shorthand().map(String::from))
        .unwrap_or_else(|| "HEAD".into())
}

fn translate_status(s: Status) -> GitStatus {
    if s.contains(Status::CONFLICTED) {
        GitStatus::Conflict
    } else if s.contains(Status::INDEX_NEW)
        || s.contains(Status::INDEX_MODIFIED)
        || s.contains(Status::INDEX_DELETED)
        || s.contains(Status::INDEX_RENAMED)
        || s.contains(Status::INDEX_TYPECHANGE)
    {
        GitStatus::Staged
    } else if s.contains(Status::WT_MODIFIED) || s.contains(Status::WT_DELETED) {
        GitStatus::Modified
    } else if s.contains(Status::WT_NEW) {
        GitStatus::Untracked
    } else {
        GitStatus::Clean
    }
}
