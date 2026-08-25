// src/ops/fileops.rs
//
// High-speed parallel file operations with progress reporting.
// Copy/Move uses a multi-threaded pipeline with a per-file progress channel.
// Trash uses the Freedesktop trash specification via the `trash` crate.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use rayon::prelude::*;

/// Progress event emitted during a file operation.
#[derive(Debug, Clone)]
pub struct Progress {
    pub current_file: PathBuf,
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
}

/// Result of a file operation (per-file).
#[derive(Debug, Clone)]
pub enum OpResult {
    Progress(Progress),
    Done,
    Error { path: PathBuf, reason: String },
}

/// Copy a list of source paths to a destination directory.
/// Streams progress events through the returned receiver.
/// Set `cancel.store(true, Ordering::Relaxed)` to abort.
pub fn copy_files(
    sources: Vec<PathBuf>,
    dest_dir: PathBuf,
    cancel: Arc<AtomicBool>,
) -> flume::Receiver<OpResult> {
    let (tx, rx) = flume::bounded(64);

    std::thread::spawn(move || {
        let total_bytes = compute_total_size(&sources);
        let bytes_done = Arc::new(AtomicU64::new(0));
        let files_total = sources.len();

        for (i, src) in sources.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }

            let dest = dest_dir.join(src.file_name().unwrap_or_default());

            let result = if src.is_dir() {
                copy_dir_recursive(src, &dest, &cancel)
            } else {
                copy_single_file(src, &dest)
            };

            match result {
                Ok(size) => {
                    bytes_done.fetch_add(size, Ordering::Relaxed);
                    let _ = tx.send(OpResult::Progress(Progress {
                        current_file: src.clone(),
                        files_done: i + 1,
                        files_total,
                        bytes_done: bytes_done.load(Ordering::Relaxed),
                        bytes_total: total_bytes,
                    }));
                }
                Err(e) => {
                    let _ = tx.send(OpResult::Error {
                        path: src.clone(),
                        reason: e.to_string(),
                    });
                }
            }
        }

        let _ = tx.send(OpResult::Done);
    });

    rx
}

/// Move a list of source paths to a destination directory.
/// Attempts rename first (same filesystem, O(1)), falls back to copy+delete.
pub fn move_files(
    sources: Vec<PathBuf>,
    dest_dir: PathBuf,
    cancel: Arc<AtomicBool>,
) -> flume::Receiver<OpResult> {
    let (tx, rx) = flume::bounded(64);

    std::thread::spawn(move || {
        let files_total = sources.len();

        for (i, src) in sources.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }

            let dest = dest_dir.join(src.file_name().unwrap_or_default());

            // Try atomic rename first (same filesystem)
            let result = std::fs::rename(src, &dest).or_else(|_| {
                // Cross-device: copy then delete
                if src.is_dir() {
                    copy_dir_recursive(src, &dest, &cancel)?;
                    std::fs::remove_dir_all(src).map(|_| 0u64)
                } else {
                    copy_single_file(src, &dest)?;
                    let _ = std::fs::remove_file(src);
                    Ok(0u64)
                }
            });

            match result {
                Ok(_) => {
                    let _ = tx.send(OpResult::Progress(Progress {
                        current_file: src.clone(),
                        files_done: i + 1,
                        files_total,
                        bytes_done: (i + 1) as u64,
                        bytes_total: files_total as u64,
                    }));
                }
                Err(e) => {
                    let _ = tx.send(OpResult::Error {
                        path: src.clone(),
                        reason: e.to_string(),
                    });
                }
            }
        }

        let _ = tx.send(OpResult::Done);
    });

    rx
}

/// Send files to Freedesktop Trash (recoverable delete).
pub fn trash_files(paths: Vec<PathBuf>) -> Vec<anyhow::Error> {
    paths
        .into_par_iter()
        .filter_map(|p| trash::delete(&p).err().map(|e| anyhow::anyhow!("{}: {}", p.display(), e)))
        .collect()
}

/// Permanently delete files (irreversible).
pub fn delete_files_permanent(paths: Vec<PathBuf>) -> Vec<anyhow::Error> {
    paths
        .into_par_iter()
        .filter_map(|p| {
            let result = if p.is_dir() {
                std::fs::remove_dir_all(&p)
            } else {
                std::fs::remove_file(&p)
            };
            result.err().map(|e| anyhow::anyhow!("{}: {}", p.display(), e))
        })
        .collect()
}

/// Create a new directory.
pub fn create_directory(path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path).map_err(|e| anyhow::anyhow!("mkdir failed: {}", e))
}

/// Create a new empty file.
pub fn create_file(path: &Path) -> anyhow::Result<()> {
    std::fs::File::create(path)?;
    Ok(())
}

/// Rename a single file/directory.
pub fn rename_entry(from: &Path, to: &Path) -> anyhow::Result<()> {
    std::fs::rename(from, to).map_err(|e| anyhow::anyhow!("rename failed: {}", e))
}

// ── Internals ──────────────────────────────────────────────────────────────

fn copy_single_file(src: &Path, dest: &Path) -> anyhow::Result<u64> {
    let size = std::fs::copy(src, dest)?;
    Ok(size)
}

fn copy_dir_recursive(
    src: &Path,
    dest: &Path,
    cancel: &Arc<AtomicBool>,
) -> anyhow::Result<u64> {
    std::fs::create_dir_all(dest)?;
    let mut total = 0u64;
    for entry in std::fs::read_dir(src)? {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let entry = entry?;
        let src_child = entry.path();
        let dest_child = dest.join(entry.file_name());
        if src_child.is_dir() {
            total += copy_dir_recursive(&src_child, &dest_child, cancel)?;
        } else {
            total += copy_single_file(&src_child, &dest_child)?;
        }
    }
    Ok(total)
}

fn compute_total_size(paths: &[PathBuf]) -> u64 {
    paths
        .par_iter()
        .map(|p| {
            if p.is_dir() {
                dir_size(p)
            } else {
                p.metadata().map(|m| m.len()).unwrap_or(0)
            }
        })
        .sum()
}

fn dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
        .sum()
}
