// src/utils/paths.rs
//
// Path utilities: home directory resolution, breadcrumb decomposition,
// and safe path joining.

use std::path::{Path, PathBuf};

/// Expand `~` to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = dirs_next::home_dir() {
            return home;
        }
    }
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs_next::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

/// Replace the home directory prefix with `~` for display.
pub fn collapse_home(path: &Path) -> String {
    if let Some(home) = dirs_next::home_dir() {
        if let Ok(rel) = path.strip_prefix(&home) {
            return format!("~/{}", rel.display());
        }
    }
    path.display().to_string()
}

/// Decompose a path into breadcrumb segments for the toolbar.
/// Returns a list of (display_name, full_path) pairs.
pub fn breadcrumbs(path: &Path) -> Vec<(String, PathBuf)> {
    let mut crumbs = Vec::new();
    let mut components = Vec::new();

    for component in path.components() {
        components.push(component);
        let full: PathBuf = components.iter().collect();
        let name = match component {
            std::path::Component::RootDir => "/".to_string(),
            std::path::Component::Normal(n) => n.to_string_lossy().into_owned(),
            _ => continue,
        };
        crumbs.push((name, full));
    }

    crumbs
}

/// Return the standard XDG user directories as (name, path) pairs
/// for the sidebar's quick-access bookmarks.
pub fn xdg_places() -> Vec<(String, PathBuf)> {
    let mut places = Vec::new();
    let dirs = [
        ("Home", dirs_next::home_dir()),
        ("Desktop", dirs_next::desktop_dir()),
        ("Documents", dirs_next::document_dir()),
        ("Downloads", dirs_next::download_dir()),
        ("Pictures", dirs_next::picture_dir()),
        ("Music", dirs_next::audio_dir()),
        ("Videos", dirs_next::video_dir()),
    ];
    for (name, maybe_path) in dirs {
        if let Some(p) = maybe_path {
            places.push((name.to_string(), p));
        }
    }
    // Add Trash path
    if let Some(data) = dirs_next::data_local_dir() {
        places.push(("Trash".to_string(), data.join("Trash/files")));
    }
    places
}
