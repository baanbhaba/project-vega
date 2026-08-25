// src/core/tags.rs
//
// Lightweight tag & Kanban stage metadata store.
// Persists per-directory metadata in a tiny JSON sidecar file
// stored at ~/.local/share/vega/tags/<hashed_path>.json
// so that the user's filesystem is never modified.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use once_cell::sync::Lazy;
use parking_lot::Mutex;

/// Per-file tag metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileTags {
    /// User-chosen color tag (e.g. "red", "blue", or a hex color "#FF5733").
    pub color: Option<String>,
    /// Kanban stage (e.g. "Backlog", "Doing", "Done", "Review").
    pub kanban: Option<String>,
    /// Arbitrary user-defined labels.
    pub labels: Vec<String>,
}

/// In-memory store: absolute path → FileTags.
static TAG_STORE: Lazy<Mutex<HashMap<PathBuf, FileTags>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Directory where tag JSON files are stored.
fn store_dir() -> PathBuf {
    dirs_next::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("vega")
        .join("tags")
}

/// Derive a deterministic filename for a directory's tag file.
fn tag_file_for(dir: &Path) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    dir.hash(&mut h);
    store_dir().join(format!("{:016x}.json", h.finish()))
}

/// Load tags for all files in `dir` from disk into memory.
/// No-ops silently if the file does not exist yet.
pub fn load_tags_for(dir: &Path) {
    let path = tag_file_for(dir);
    if !path.exists() {
        return;
    }
    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            if let Ok(map) = serde_json::from_str::<HashMap<PathBuf, FileTags>>(&contents) {
                let mut store = TAG_STORE.lock();
                for (k, v) in map {
                    store.insert(k, v);
                }
            }
        }
        Err(e) => log::warn!("Failed to load tags from {:?}: {}", path, e),
    }
}

/// Persist tags for all files in `dir` to disk.
pub fn save_tags_for(dir: &Path) {
    let store = TAG_STORE.lock();
    // Collect only entries whose path is under `dir`
    let subset: HashMap<&PathBuf, &FileTags> = store
        .iter()
        .filter(|(k, _)| k.starts_with(dir))
        .collect();

    if subset.is_empty() {
        return;
    }

    let path = tag_file_for(dir);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match serde_json::to_string_pretty(&subset) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::warn!("Failed to save tags to {:?}: {}", path, e);
            }
        }
        Err(e) => log::warn!("Failed to serialize tags: {}", e),
    }
}

/// Get the tags for a specific file.
pub fn get_tags(file: &Path) -> FileTags {
    TAG_STORE.lock().get(file).cloned().unwrap_or_default()
}

/// Set tags for a specific file and persist to disk immediately.
pub fn set_tags(file: &Path, tags: FileTags) {
    let dir = file.parent().unwrap_or(file);
    TAG_STORE.lock().insert(file.to_path_buf(), tags);
    save_tags_for(dir);
}

/// Set the Kanban stage for a file.
pub fn set_kanban(file: &Path, stage: Option<String>) {
    let mut tags = get_tags(file);
    tags.kanban = stage;
    set_tags(file, tags);
}

/// Set the color tag for a file.
pub fn set_color(file: &Path, color: Option<String>) {
    let mut tags = get_tags(file);
    tags.color = color;
    set_tags(file, tags);
}
