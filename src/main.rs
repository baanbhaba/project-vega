// src/main.rs
//
// Vega — Ultra-fast GPU-accelerated file manager for Linux
// Hardware-accelerated Slint UI wired to a multi-threaded Rust systems engine.

#![allow(unused_variables, dead_code)]

mod core;
mod ops;
mod utils;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use slint::{Color, ModelRc, SharedString, VecModel};

use crate::core::entry::{FileCategory, FileEntry, GitStatus};
use crate::core::scanner::{scan_directory, sort_entries, ScanConfig, ScanEvent, SortBy, SortDir};
use crate::core::watcher::{watch_directory, WatchEvent};
use crate::utils::human::{format_bytes, format_modified};
use crate::utils::paths::{collapse_home, expand_tilde};

// Include the Slint generated bindings
slint::include_modules!();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardMode {
    Copy,
    Cut,
}

#[derive(Debug, Clone)]
struct ClipboardItem {
    paths: Vec<PathBuf>,
    mode: ClipboardMode,
}

struct WatcherHandle {
    dir: PathBuf,
    _watcher: notify::RecommendedWatcher,
}

/// Global application state
struct AppState {
    current_dir: PathBuf,
    history: Vec<PathBuf>,
    history_idx: usize,
    show_hidden: bool,
    sort_by: SortBy,
    sort_dir: SortDir,
    all_entries: Vec<FileEntry>,
    displayed_entries: Vec<FileEntry>,
    selected_idx: Option<usize>,
    filter_query: String,
    cancel_token: Arc<AtomicBool>,
    watcher: Option<WatcherHandle>,
    clipboard: Option<ClipboardItem>,
}

impl AppState {
    fn new(start_dir: PathBuf) -> Self {
        Self {
            current_dir: start_dir.clone(),
            history: vec![start_dir],
            history_idx: 0,
            show_hidden: false,
            sort_by: SortBy::Name,
            sort_dir: SortDir::Ascending,
            all_entries: Vec::new(),
            displayed_entries: Vec::new(),
            selected_idx: None,
            filter_query: String::new(),
            cancel_token: Arc::new(AtomicBool::new(false)),
            watcher: None,
            clipboard: None,
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let window = MainWindow::new()?;
    let _ = slint::set_xdg_app_id("Vega");
    let weak = window.as_weak();

    // Determine initial directory (safely filtering "%U" from desktop launchers)
    let start_dir: PathBuf = std::env::args()
        .nth(1)
        .filter(|s| !s.trim().is_empty() && s != "%U" && s != "%u")
        .map(|s| expand_tilde(&s))
        .unwrap_or_else(|| dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("/")));

    let start_dir = if start_dir.is_dir() {
        start_dir
    } else {
        dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    };

    let state = Arc::new(Mutex::new(AppState::new(start_dir.clone())));

    // ── Populate sidebar places ──────────────────────────────────────────────
    if let Some(w) = weak.upgrade() {
        let places = build_places();
        w.set_places(ModelRc::new(VecModel::from(places)));
        w.set_sort_column("name".into());
        w.set_sort_direction("asc".into());
    }

    // ── Activation Handler with Strict 300ms Debounce ────────────────────────
    let last_activation = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));
    let activate_entry = {
        let weak_a = weak.clone();
        let state_a = state.clone();
        let last_act = last_activation.clone();
        Arc::new(move |idx: usize| {
            let mut la = last_act.lock();
            if la.elapsed() < Duration::from_millis(300) {
                return; // Suppress duplicate triggers from rapid click + double-click events
            }
            *la = Instant::now();
            drop(la);

            let entry_opt = {
                let s = state_a.lock();
                s.displayed_entries.get(idx).cloned()
            };
            if let Some(entry) = entry_opt {
                if entry.is_dir {
                    navigate_to(weak_a.clone(), state_a.clone(), entry.path, true);
                } else {
                    log::info!("Opening file: {}", entry.path.display());
                    if open::that(&entry.path).is_err() {
                        let _ = std::process::Command::new("kitty")
                            .arg("-e")
                            .arg("nvim")
                            .arg(&entry.path)
                            .spawn();
                    }
                }
            }
        })
    };

    // ── Initial Navigation ───────────────────────────────────────────────────
    navigate_to(weak.clone(), state.clone(), start_dir, false);

    // ── Toolbar & Navigation Callbacks ───────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_navigate_to(move |path_str| {
            let path_s = path_str.as_str().trim();
            if path_s.is_empty() {
                return;
            }
            let expanded = expand_tilde(path_s);
            let target = if expanded.is_dir() {
                Some(expanded)
            } else {
                let current = state_c.lock().current_dir.clone();
                let joined = current.join(path_s);
                if joined.is_dir() {
                    Some(joined)
                } else {
                    None
                }
            };

            if let Some(dir) = target {
                navigate_to(weak_c.clone(), state_c.clone(), dir, true);
            } else if let Some(w) = weak_c.upgrade() {
                let current = state_c.lock().current_dir.clone();
                w.set_current_path(collapse_home(&current).into());
                w.set_selected_info("Directory not found".into());
            }
        });
    }

    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_go_back(move || {
            let target = {
                let mut s = state_c.lock();
                if s.history_idx > 0 {
                    s.history_idx -= 1;
                    Some(s.history[s.history_idx].clone())
                } else {
                    None
                }
            };
            if let Some(dir) = target {
                navigate_to(weak_c.clone(), state_c.clone(), dir, false);
            }
        });
    }

    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_go_forward(move || {
            let target = {
                let mut s = state_c.lock();
                if s.history_idx + 1 < s.history.len() {
                    s.history_idx += 1;
                    Some(s.history[s.history_idx].clone())
                } else {
                    None
                }
            };
            if let Some(dir) = target {
                navigate_to(weak_c.clone(), state_c.clone(), dir, false);
            }
        });
    }

    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_go_up(move || {
            let parent = {
                let s = state_c.lock();
                s.current_dir.parent().map(|p| p.to_path_buf())
            };
            if let Some(p) = parent {
                navigate_to(weak_c.clone(), state_c.clone(), p, true);
            }
        });
    }

    // ── Click & Double Click Handling ────────────────────────────────────────
    let last_click_state = Arc::new(Mutex::new((None::<usize>, Instant::now())));

    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        let lcs = last_click_state.clone();
        let act = activate_entry.clone();
        window.on_entry_clicked(move |index| {
            let idx = index as usize;
            let now = Instant::now();

            let is_double_click = {
                let mut l = lcs.lock();
                let is_dc = l.0 == Some(idx) && now.duration_since(l.1) < Duration::from_millis(380);
                if is_dc {
                    *l = (None, now);
                } else {
                    *l = (Some(idx), now);
                }
                is_dc
            };

            if is_double_click {
                act(idx);
                return;
            }

            // Single click: update selection and status bar without rebuilding model
            let mut s = state_c.lock();
            if idx < s.displayed_entries.len() {
                s.selected_idx = Some(idx);
                let entry = &s.displayed_entries[idx];
                let is_archive = !entry.is_dir && ops::archive::ArchiveFormat::from_path(&entry.path).is_some();
                let selected_info = format!(
                    "{} ({} • {} • {})",
                    entry.name,
                    if entry.is_dir { "Folder".into() } else { format_bytes(entry.size) },
                    entry.permissions,
                    format_modified(entry.modified)
                );
                if let Some(w) = weak_c.upgrade() {
                    w.set_selected_index(idx as i32);
                    w.set_selected_name(entry.name.clone().into());
                    w.set_is_selected_archive(is_archive);
                    w.set_selected_info(selected_info.into());
                }
            }
        });
    }

    {
        let act = activate_entry.clone();
        window.on_entry_double_clicked(move |index| {
            if index >= 0 {
                act(index as usize);
            }
        });
    }

    // ── Keyboard Selection & Activation ──────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_select_prev(move || {
            let mut s = state_c.lock();
            let len = s.displayed_entries.len();
            if len == 0 {
                return;
            }
            let new_idx = match s.selected_idx {
                Some(i) if i > 0 => i - 1,
                _ => len - 1,
            };
            s.selected_idx = Some(new_idx);
            let entry = &s.displayed_entries[new_idx];
            let is_archive = !entry.is_dir && ops::archive::ArchiveFormat::from_path(&entry.path).is_some();
            let selected_info = format!(
                "{} ({} • {} • {})",
                entry.name,
                if entry.is_dir { "Folder".into() } else { format_bytes(entry.size) },
                entry.permissions,
                format_modified(entry.modified)
            );
            if let Some(w) = weak_c.upgrade() {
                w.set_selected_index(new_idx as i32);
                w.set_selected_name(entry.name.clone().into());
                w.invoke_ensure_visible(new_idx as i32);
                w.set_is_selected_archive(is_archive);
                w.set_selected_info(selected_info.into());
            }
        });
    }

    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_select_next(move || {
            let mut s = state_c.lock();
            let len = s.displayed_entries.len();
            if len == 0 {
                return;
            }
            let new_idx = match s.selected_idx {
                Some(i) if i + 1 < len => i + 1,
                _ => 0,
            };
            s.selected_idx = Some(new_idx);
            let entry = &s.displayed_entries[new_idx];
            let is_archive = !entry.is_dir && ops::archive::ArchiveFormat::from_path(&entry.path).is_some();
            let selected_info = format!(
                "{} ({} • {} • {})",
                entry.name,
                if entry.is_dir { "Folder".into() } else { format_bytes(entry.size) },
                entry.permissions,
                format_modified(entry.modified)
            );
            if let Some(w) = weak_c.upgrade() {
                w.set_selected_index(new_idx as i32);
                w.set_selected_name(entry.name.clone().into());
                w.invoke_ensure_visible(new_idx as i32);
                w.set_is_selected_archive(is_archive);
                w.set_selected_info(selected_info.into());
            }
        });
    }

    {
        let act = activate_entry.clone();
        let state_c = state.clone();
        window.on_activate_selected(move || {
            if let Some(idx) = state_c.lock().selected_idx {
                act(idx);
            }
        });
    }

    {
        let act = activate_entry.clone();
        let state_c = state.clone();
        window.on_activate_first(move || {
            if !state_c.lock().displayed_entries.is_empty() {
                act(0);
            }
        });
    }

    // ── Column Sorting ───────────────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_sort_by(move |col| {
            let mut s = state_c.lock();
            let col_name = col.as_str();
            let new_sort = match col_name {
                "size" => SortBy::Size,
                "modified" => SortBy::Modified,
                _ => SortBy::Name,
            };

            if s.sort_by == new_sort {
                s.sort_dir = match s.sort_dir {
                    SortDir::Ascending => SortDir::Descending,
                    SortDir::Descending => SortDir::Ascending,
                };
            } else {
                s.sort_by = new_sort;
                s.sort_dir = SortDir::Ascending;
            }

            let sort_by = s.sort_by;
            let sort_dir = s.sort_dir;
            sort_entries(&mut s.all_entries, sort_by, sort_dir);

            let query = s.filter_query.clone();
            let displayed = if query.trim().is_empty() {
                s.all_entries.clone()
            } else {
                core::filter::fuzzy_filter(&s.all_entries, &query)
            };
            s.displayed_entries = displayed.clone();
            s.selected_idx = None;

            let ui_rows = entries_to_slint(&displayed);
            let dir_str = match s.sort_dir {
                SortDir::Ascending => "asc",
                SortDir::Descending => "desc",
            };

            if let Some(w) = weak_c.upgrade() {
                w.set_sort_column(col_name.into());
                w.set_sort_direction(dir_str.into());
                w.set_selected_index(-1);
                w.set_selected_name("".into());
                w.set_entries(ModelRc::new(VecModel::from(ui_rows)));
            }
        });
    }

    // ── Filter & Search: Real-time fuzzy filtering ───────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_filter_changed(move |query_str| {
            let query = query_str.as_str().to_string();
            let mut s = state_c.lock();
            s.filter_query = query.clone();

            let filtered = if query.trim().is_empty() {
                s.all_entries.clone()
            } else {
                core::filter::fuzzy_filter(&s.all_entries, &query)
            };

            s.displayed_entries = filtered.clone();
            s.selected_idx = None;

            let total_all = s.all_entries.len();
            let count_str: SharedString = if query.trim().is_empty() {
                format!("{} item{}", total_all, if total_all == 1 { "" } else { "s" }).into()
            } else {
                format!("{} of {} item{}", filtered.len(), total_all, if total_all == 1 { "" } else { "s" }).into()
            };

            let ui_rows = entries_to_slint(&s.displayed_entries);
            if let Some(w) = weak_c.upgrade() {
                w.set_item_count(count_str);
                w.set_selected_index(-1);
                w.set_selected_name("".into());
                w.set_selected_info("".into());
                w.set_entries(ModelRc::new(VecModel::from(ui_rows)));
            }
        });
    }

    // ── Toggle Hidden Files ──────────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_toggle_hidden(move || {
            let dir = {
                let mut s = state_c.lock();
                s.show_hidden = !s.show_hidden;
                s.current_dir.clone()
            };
            navigate_to(weak_c.clone(), state_c.clone(), dir, false);
        });
    }

    // ── Create Folder with Custom Name ───────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_create_folder(move |name_str| {
            let name = name_str.as_str().trim();
            if name.is_empty() {
                return;
            }
            let dir = { state_c.lock().current_dir.clone() };
            let new_dir = dir.join(name);

            match std::fs::create_dir(&new_dir) {
                Ok(_) => {
                    log::info!("Created directory: {}", new_dir.display());
                    reload_directory(weak_c.clone(), state_c.clone(), dir);
                }
                Err(e) => {
                    log::error!("Failed to create directory {}: {}", new_dir.display(), e);
                    if let Some(w) = weak_c.upgrade() {
                        w.set_selected_info(format!("Error: {}", e).into());
                    }
                }
            }
        });
    }

    // ── Create File with Custom Name ─────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_create_file(move |name_str| {
            let name = name_str.as_str().trim();
            if name.is_empty() {
                return;
            }
            let dir = { state_c.lock().current_dir.clone() };
            let new_file = dir.join(name);

            match std::fs::File::create(&new_file) {
                Ok(_) => {
                    log::info!("Created file: {}", new_file.display());
                    reload_directory(weak_c.clone(), state_c.clone(), dir);
                }
                Err(e) => {
                    log::error!("Failed to create file {}: {}", new_file.display(), e);
                    if let Some(w) = weak_c.upgrade() {
                        w.set_selected_info(format!("Error: {}", e).into());
                    }
                }
            }
        });
    }

    // ── Rename Selected Item ─────────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_rename_item(move |new_name_str| {
            let new_name = new_name_str.as_str().trim();
            if new_name.is_empty() {
                return;
            }
            let (target_opt, current_dir) = {
                let s = state_c.lock();
                let target = s.selected_idx.and_then(|i| s.displayed_entries.get(i).cloned());
                (target, s.current_dir.clone())
            };

            if let Some(target) = target_opt {
                let new_path = current_dir.join(new_name);
                match std::fs::rename(&target.path, &new_path) {
                    Ok(_) => {
                        log::info!("Renamed {} -> {}", target.path.display(), new_path.display());
                        reload_directory(weak_c.clone(), state_c.clone(), current_dir);
                    }
                    Err(e) => {
                        log::error!("Failed to rename {}: {}", target.path.display(), e);
                        if let Some(w) = weak_c.upgrade() {
                            w.set_selected_info(format!("Rename error: {}", e).into());
                        }
                    }
                }
            }
        });
    }

    // ── Trash Selected ───────────────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_trash_selected(move || {
            let (target_path, current_dir) = {
                let s = state_c.lock();
                let target = s.selected_idx.and_then(|i| s.displayed_entries.get(i).map(|e| e.path.clone()));
                (target, s.current_dir.clone())
            };

            if let Some(path) = target_path {
                log::info!("Moving to trash: {}", path.display());
                match trash::delete(&path) {
                    Ok(_) => {
                        reload_directory(weak_c.clone(), state_c.clone(), current_dir);
                    }
                    Err(e) => {
                        log::error!("Failed to trash {}: {}", path.display(), e);
                        if let Some(w) = weak_c.upgrade() {
                            w.set_selected_info(format!("Trash error: {}", e).into());
                        }
                    }
                }
            }
        });
    }

    // ── Permanent Delete ─────────────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_permanent_delete(move || {
            let (target_opt, current_dir) = {
                let s = state_c.lock();
                let target = s.selected_idx.and_then(|i| s.displayed_entries.get(i).cloned());
                (target, s.current_dir.clone())
            };

            if let Some(entry) = target_opt {
                let res = if entry.is_dir {
                    std::fs::remove_dir_all(&entry.path)
                } else {
                    std::fs::remove_file(&entry.path)
                };
                match res {
                    Ok(_) => {
                        if let Some(w) = weak_c.upgrade() {
                            w.set_selected_info(format!("Deleted: {}", entry.name).into());
                        }
                        reload_directory(weak_c.clone(), state_c.clone(), current_dir);
                    }
                    Err(e) => {
                        if let Some(w) = weak_c.upgrade() {
                            w.set_selected_info(format!("Delete error: {}", e).into());
                        }
                    }
                }
            }
        });
    }

    // ── Duplicate Selected Item ──────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_duplicate_selected(move || {
            let (target_opt, current_dir) = {
                let s = state_c.lock();
                let target = s.selected_idx.and_then(|i| s.displayed_entries.get(i).cloned());
                (target, s.current_dir.clone())
            };

            if let Some(entry) = target_opt {
                let stem = entry.path.file_stem().unwrap_or_default().to_string_lossy();
                let ext = entry.path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                let mut counter = 1;
                let mut new_path = current_dir.join(format!("{} (copy){}", stem, ext));
                while new_path.exists() {
                    counter += 1;
                    new_path = current_dir.join(format!("{} (copy {}){}", stem, counter, ext));
                }

                if entry.is_dir {
                    let cancel = Arc::new(AtomicBool::new(false));
                    let _ = ops::fileops::copy_files(vec![entry.path], current_dir.clone(), cancel);
                } else {
                    let _ = std::fs::copy(&entry.path, &new_path);
                }
                reload_directory(weak_c.clone(), state_c.clone(), current_dir);
            }
        });
    }

    // ── Copy Path to System Clipboard (wl-copy / xclip) ──────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_copy_path(move || {
            let target_opt = {
                let s = state_c.lock();
                s.selected_idx.and_then(|i| s.displayed_entries.get(i).map(|e| e.path.display().to_string()))
            };

            if let Some(path_str) = target_opt {
                copy_to_system_clipboard(&path_str);
                if let Some(w) = weak_c.upgrade() {
                    w.set_selected_info(format!("Copied path: {}", path_str).into());
                }
            }
        });
    }

    // ── Archive: Extract Selected ────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_extract_archive(move || {
            let (target_opt, current_dir) = {
                let s = state_c.lock();
                let target = s.selected_idx.and_then(|i| s.displayed_entries.get(i).map(|e| e.path.clone()));
                (target, s.current_dir.clone())
            };

            if let Some(src) = target_opt {
                if let Some(fmt) = ops::archive::ArchiveFormat::from_path(&src) {
                    let stem = src.file_stem().unwrap_or_default().to_string_lossy().to_string();
                    let clean_stem = stem.trim_end_matches(".tar").to_string();
                    let dest = current_dir.join(&clean_stem);
                    let weak_a = weak_c.clone();
                    let state_a = state_c.clone();
                    let dir_a = current_dir.clone();

                    if let Some(w) = weak_c.upgrade() {
                        w.set_selected_info("Extracting archive…".into());
                    }

                    std::thread::spawn(move || {
                        match ops::archive::extract(&src, &dest, fmt, None) {
                            Ok(_) => {
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = weak_a.upgrade() {
                                        w.set_selected_info(format!("Extracted to {}", clean_stem).into());
                                    }
                                    reload_directory(weak_a, state_a, dir_a);
                                });
                            }
                            Err(e) => {
                                log::error!("Extraction error: {}", e);
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = weak_a.upgrade() {
                                        w.set_selected_info(format!("Extract error: {}", e).into());
                                    }
                                });
                            }
                        }
                    });
                }
            }
        });
    }

    // ── Archive: Compress Selected to ZIP ────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_compress_selected(move || {
            let (target_opt, current_dir) = {
                let s = state_c.lock();
                let target = s.selected_idx.and_then(|i| s.displayed_entries.get(i).cloned());
                (target, s.current_dir.clone())
            };

            if let Some(entry) = target_opt {
                let zip_name = format!("{}.zip", entry.name);
                let dest = current_dir.join(&zip_name);
                let weak_a = weak_c.clone();
                let state_a = state_c.clone();
                let dir_a = current_dir.clone();

                if let Some(w) = weak_c.upgrade() {
                    w.set_selected_info("Compressing to ZIP…".into());
                }

                std::thread::spawn(move || {
                    match ops::archive::compress(vec![entry.path], &dest, ops::archive::ArchiveFormat::Zip, None) {
                        Ok(_) => {
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(w) = weak_a.upgrade() {
                                    w.set_selected_info(format!("Created {}", zip_name).into());
                                }
                                reload_directory(weak_a, state_a, dir_a);
                            });
                        }
                        Err(e) => {
                            log::error!("Compression error: {}", e);
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(w) = weak_a.upgrade() {
                                    w.set_selected_info(format!("Compress error: {}", e).into());
                                }
                            });
                        }
                    }
                });
            }
        });
    }

    // ── Show Properties Inspector ────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_show_properties(move || {
            let target_opt = {
                let s = state_c.lock();
                s.selected_idx.and_then(|i| s.displayed_entries.get(i).cloned())
            };

            if let Some(entry) = target_opt {
                let symlink_str = if entry.is_symlink {
                    std::fs::read_link(&entry.path)
                        .map(|p| format!(" -> {}", p.display()))
                        .unwrap_or_default()
                } else {
                    String::new()
                };

                let type_str = if entry.is_dir {
                    "Directory".to_string()
                } else {
                    format!("{:?} (.{})", entry.category, entry.extension)
                };

                let size_str = if entry.is_dir {
                    "Directory".to_string()
                } else {
                    format!("{} ({} bytes)", format_bytes(entry.size), entry.size)
                };

                let mod_str = format_modified(entry.modified);
                let full_path = format!("{}{}", entry.path.display(), symlink_str);

                if let Some(w) = weak_c.upgrade() {
                    w.set_props_name(entry.name.into());
                    w.set_props_path(full_path.into());
                    w.set_props_type(type_str.into());
                    w.set_props_size(size_str.into());
                    w.set_props_modified(mod_str.into());
                    w.set_props_permissions(entry.permissions.into());
                    w.set_props_checksum("Click Calculate to compute".into());
                    w.set_props_visible(true);
                }
            }
        });
    }

    // ── Compute BLAKE3 Checksum ──────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_compute_checksum(move || {
            let target_opt = {
                let s = state_c.lock();
                s.selected_idx.and_then(|i| s.displayed_entries.get(i).map(|e| e.path.clone()))
            };

            if let Some(path) = target_opt {
                let weak_cs = weak_c.clone();
                if let Some(w) = weak_c.upgrade() {
                    w.set_props_checksum("Computing BLAKE3…".into());
                }

                std::thread::spawn(move || {
                    let hash = ops::checksum::checksum_file(&path, ops::checksum::ChecksumAlg::Blake3)
                        .unwrap_or_else(|e| format!("Error: {}", e));

                    let hash_s: SharedString = hash.into();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = weak_cs.upgrade() {
                            w.set_props_checksum(hash_s);
                        }
                    });
                });
            }
        });
    }

    // ── Clipboard (Copy / Cut / Paste) ───────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_copy_selected(move || {
            let mut s = state_c.lock();
            let entry_opt = s.selected_idx.and_then(|idx| {
                s.displayed_entries.get(idx).map(|e| (e.path.clone(), e.name.clone()))
            });
            if let Some((path, name)) = entry_opt {
                s.clipboard = Some(ClipboardItem {
                    paths: vec![path],
                    mode: ClipboardMode::Copy,
                });
                let info_s: SharedString = format!("Copied: {}", name).into();
                if let Some(w) = weak_c.upgrade() {
                    w.set_has_clipboard(true);
                    w.set_selected_info(info_s);
                }
            }
        });
    }

    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_cut_selected(move || {
            let mut s = state_c.lock();
            let entry_opt = s.selected_idx.and_then(|idx| {
                s.displayed_entries.get(idx).map(|e| (e.path.clone(), e.name.clone()))
            });
            if let Some((path, name)) = entry_opt {
                s.clipboard = Some(ClipboardItem {
                    paths: vec![path],
                    mode: ClipboardMode::Cut,
                });
                let info_s: SharedString = format!("Cut: {}", name).into();
                if let Some(w) = weak_c.upgrade() {
                    w.set_has_clipboard(true);
                    w.set_selected_info(info_s);
                }
            }
        });
    }

    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_paste_clipboard(move || {
            let (clipboard_item, current_dir) = {
                let s = state_c.lock();
                (s.clipboard.clone(), s.current_dir.clone())
            };

            if let Some(clip) = clipboard_item {
                for src in &clip.paths {
                    if let Some(file_name) = src.file_name() {
                        let mut dest = current_dir.join(file_name);
                        // If same destination in Copy mode, generate " (copy)"
                        if clip.mode == ClipboardMode::Copy && dest.exists() {
                            let stem = src.file_stem().unwrap_or_default().to_string_lossy();
                            let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
                            let mut counter = 1;
                            dest = current_dir.join(format!("{} (copy){}", stem, ext));
                            while dest.exists() {
                                counter += 1;
                                dest = current_dir.join(format!("{} (copy {}){}", stem, counter, ext));
                            }
                        }

                        match clip.mode {
                            ClipboardMode::Copy => {
                                if src.is_dir() {
                                    let cancel = Arc::new(AtomicBool::new(false));
                                    let _ = ops::fileops::copy_files(vec![src.clone()], current_dir.clone(), cancel);
                                } else {
                                    let _ = std::fs::copy(src, &dest);
                                }
                            }
                            ClipboardMode::Cut => {
                                let _ = std::fs::rename(src, &dest);
                            }
                        }
                    }
                }

                if clip.mode == ClipboardMode::Cut {
                    state_c.lock().clipboard = None;
                    if let Some(w) = weak_c.upgrade() {
                        w.set_has_clipboard(false);
                    }
                }
                reload_directory(weak_c.clone(), state_c.clone(), current_dir);
            }
        });
    }

    // ── Open Terminal ────────────────────────────────────────────────────────
    {
        let state_c = state.clone();
        window.on_open_terminal(move || {
            let dir = { state_c.lock().current_dir.clone() };
            let _ = std::process::Command::new("kitty")
                .arg("--directory")
                .arg(&dir)
                .spawn()
                .or_else(|_| {
                    std::process::Command::new("x-terminal-emulator")
                        .current_dir(&dir)
                        .spawn()
                })
                .or_else(|_| {
                    std::process::Command::new("xterm")
                        .current_dir(&dir)
                        .spawn()
                });
        });
    }

    // ── Refresh ──────────────────────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_refresh(move || {
            let dir = { state_c.lock().current_dir.clone() };
            reload_directory(weak_c.clone(), state_c.clone(), dir);
        });
    }

    // ── Run Slint Event Loop ─────────────────────────────────────────────────
    window.run()?;
    Ok(())
}

/// Navigate to a directory: updates history, cancels any in-flight scan,
/// sets up real-time inotify watcher, and streams entries into Slint.
fn navigate_to(
    weak: slint::Weak<MainWindow>,
    state: Arc<Mutex<AppState>>,
    dir: PathBuf,
    push_history: bool,
) {
    let (cancel_token, show_hidden, sort_by, sort_dir) = {
        let mut s = state.lock();

        s.cancel_token.store(true, Ordering::Relaxed);
        let new_token = Arc::new(AtomicBool::new(false));
        s.cancel_token = new_token.clone();

        s.current_dir = dir.clone();
        s.selected_idx = None;
        s.filter_query.clear();

        if push_history {
            if s.history.get(s.history_idx) != Some(&dir) {
                let keep_len = s.history_idx + 1;
                s.history.truncate(keep_len);
                s.history.push(dir.clone());
                s.history_idx = s.history.len() - 1;
            }
        }

        (new_token, s.show_hidden, s.sort_by, s.sort_dir)
    };

    // Update UI headers immediately
    if let Some(w) = weak.upgrade() {
        w.set_current_path(collapse_home(&dir).into());
        w.set_raw_current_path(dir.display().to_string().into());
        w.set_item_count("Loading…".into());
        w.set_selected_index(-1);
        w.set_selected_name("".into());
        w.set_entries(ModelRc::new(VecModel::default()));
        w.set_is_selected_archive(false);
        w.set_selected_info("".into());
        w.set_git_branch("".into());

        if let Some(disk) = get_disk_free(&dir) {
            w.set_disk_info(disk.into());
        }
    }

    // Setup Real-Time inotify Watcher
    setup_watcher(&dir, weak.clone(), state.clone());

    // Background Directory Scan Pipeline
    scan_and_render(weak, state, dir, show_hidden, sort_by, sort_dir, cancel_token);
}

/// Reload current directory without resetting history or recreating the inotify watcher
fn reload_directory(
    weak: slint::Weak<MainWindow>,
    state: Arc<Mutex<AppState>>,
    dir: PathBuf,
) {
    let (cancel_token, show_hidden, sort_by, sort_dir) = {
        let mut s = state.lock();
        if s.current_dir != dir {
            return;
        }
        s.cancel_token.store(true, Ordering::Relaxed);
        let new_token = Arc::new(AtomicBool::new(false));
        s.cancel_token = new_token.clone();
        (new_token, s.show_hidden, s.sort_by, s.sort_dir)
    };

    scan_and_render(weak, state, dir, show_hidden, sort_by, sort_dir, cancel_token);
}

/// Core scanner and renderer used by both navigation and inotify reloads
fn scan_and_render(
    weak: slint::Weak<MainWindow>,
    state: Arc<Mutex<AppState>>,
    dir: PathBuf,
    show_hidden: bool,
    sort_by: SortBy,
    sort_dir: SortDir,
    cancel_token: Arc<AtomicBool>,
) {
    let dir_scan = dir.clone();
    let state_scan = state.clone();
    let weak_scan = weak.clone();

    std::thread::spawn(move || {
        let config = ScanConfig {
            show_hidden,
            sort_by,
            sort_dir,
            ..ScanConfig::default()
        };
        let rx = scan_directory(dir_scan.clone(), config, cancel_token);
        let mut collected: Vec<FileEntry> = Vec::new();

        loop {
            match rx.recv() {
                Ok(ScanEvent::Entries(batch)) => {
                    collected.extend(batch);
                }
                Ok(ScanEvent::Done { total }) => {
                    let mut s = state_scan.lock();
                    if s.current_dir != dir_scan {
                        return;
                    }
                    s.all_entries = collected.clone();

                    let query = s.filter_query.clone();
                    let displayed = if query.trim().is_empty() {
                        collected
                    } else {
                        core::filter::fuzzy_filter(&collected, &query)
                    };
                    s.displayed_entries = displayed.clone();

                    let folders_count = s.displayed_entries.iter().filter(|e| e.is_dir).count();
                    let files_count = s.displayed_entries.len().saturating_sub(folders_count);

                    let ui_rows = entries_to_slint(&displayed);
                    let count_str: SharedString = if query.trim().is_empty() {
                        format!("{} item{} ({} folder{}, {} file{})",
                            total, if total == 1 { "" } else { "s" },
                            folders_count, if folders_count == 1 { "" } else { "s" },
                            files_count, if files_count == 1 { "" } else { "s" }
                        ).into()
                    } else {
                        format!("{} of {} item{}", displayed.len(), total, if total == 1 { "" } else { "s" }).into()
                    };

                    let w_ref = weak_scan.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = w_ref.upgrade() {
                            w.set_item_count(count_str);
                            w.set_entries(ModelRc::new(VecModel::from(ui_rows)));
                        }
                    });

                    // Asynchronously check Git status so large repos never block UI render
                    let weak_git = weak_scan.clone();
                    let state_git = state_scan.clone();
                    let dir_git = dir_scan.clone();
                    std::thread::spawn(move || {
                        if let Some(root) = core::git::find_repo_root(&dir_git) {
                            if let Some((branch, statuses)) = core::git::get_repo_status(&root) {
                                let branch_s: SharedString = branch.into();
                                let w_b = weak_git.clone();
                                let _ = slint::invoke_from_event_loop(move || {
                                    if let Some(w) = w_b.upgrade() {
                                        w.set_git_branch(branch_s);
                                    }
                                });

                                let mut s = state_git.lock();
                                if s.current_dir == dir_git {
                                    for entry in &mut s.displayed_entries {
                                        if let Some(gs) = statuses.get(&entry.path) {
                                            entry.git_status = gs.clone();
                                        }
                                    }
                                    let ui_rows = entries_to_slint(&s.displayed_entries);
                                    let w_e = weak_git.clone();
                                    let _ = slint::invoke_from_event_loop(move || {
                                        if let Some(w) = w_e.upgrade() {
                                            w.set_entries(ModelRc::new(VecModel::from(ui_rows)));
                                        }
                                    });
                                }
                            }
                        }
                    });
                    break;
                }
                Ok(ScanEvent::Error(err_msg)) => {
                    log::error!("Scan error: {}", err_msg);
                    let err_s: SharedString = format!("Error: {}", err_msg).into();
                    let w_ref = weak_scan.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = w_ref.upgrade() {
                            w.set_item_count(err_s);
                        }
                    });
                    break;
                }
                Err(_) => break,
            }
        }
    });
}

/// Setup real-time watcher on a directory with strict debouncing and recursion guards
fn setup_watcher(
    dir: &Path,
    weak: slint::Weak<MainWindow>,
    state: Arc<Mutex<AppState>>,
) {
    let mut s = state.lock();
    if let Some(ref handle) = s.watcher {
        if handle.dir == dir {
            return;
        }
    }

    s.watcher = None;

    match watch_directory(dir) {
        Ok((watcher, rx)) => {
            s.watcher = Some(WatcherHandle {
                dir: dir.to_path_buf(),
                _watcher: watcher,
            });
            drop(s);

            let weak_c = weak.clone();
            let state_c = state.clone();
            let dir_c = dir.to_path_buf();

            std::thread::spawn(move || {
                let mut last_event = Instant::now();
                while let Ok(event) = rx.recv() {
                    match event {
                        WatchEvent::Created(_) | WatchEvent::Removed(_) | WatchEvent::Renamed { .. } => {}
                        WatchEvent::Modified(_) => continue,
                    }

                    if last_event.elapsed() < Duration::from_millis(300) {
                        continue;
                    }
                    last_event = Instant::now();

                    let is_current = {
                        state_c.lock().current_dir == dir_c
                    };
                    if !is_current {
                        break;
                    }

                    let w_ref = weak_c.clone();
                    let s_ref = state_c.clone();
                    let d_ref = dir_c.clone();

                    let _ = slint::invoke_from_event_loop(move || {
                        reload_directory(w_ref, s_ref, d_ref);
                    });
                }
            });
        }
        Err(e) => {
            log::warn!("Could not watch directory {}: {}", dir.display(), e);
        }
    }
}

/// Copy string to the Wayland/X11 system clipboard using wl-copy / xclip
fn copy_to_system_clipboard(text: &str) {
    use std::io::Write;
    let _ = std::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            child.wait()
        })
        .or_else(|_| {
            std::process::Command::new("xclip")
                .arg("-selection")
                .arg("clipboard")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(text.as_bytes());
                    }
                    child.wait()
                })
        });
}

/// Query free and total disk space using libc statvfs
fn get_disk_free(path: &Path) -> Option<String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } == 0 {
        let free_bytes = stat.f_bavail as u64 * stat.f_frsize as u64;
        let total_bytes = stat.f_blocks as u64 * stat.f_frsize as u64;
        Some(format!("Disk: {} free / {} total", format_bytes(free_bytes), format_bytes(total_bytes)))
    } else {
        None
    }
}

/// Build sidebar places with intuitive icons, including mounted drives
fn build_places() -> Vec<PlaceData> {
    let mut places = Vec::new();
    let home = dirs_next::home_dir();

    if let Some(ref h) = home {
        places.push(PlaceData {
            name: "Home".into(),
            path: h.display().to_string().into(),
            icon: "🏠".into(),
        });

        let projects = h.join("projects");
        if projects.is_dir() {
            places.push(PlaceData {
                name: "Projects".into(),
                path: projects.display().to_string().into(),
                icon: "💻".into(),
            });
        }

        let dotfiles = h.join("dotfiles");
        if dotfiles.is_dir() {
            places.push(PlaceData {
                name: "Dotfiles".into(),
                path: dotfiles.display().to_string().into(),
                icon: "⚙️".into(),
            });
        }
    }

    if let Some(p) = dirs_next::desktop_dir() {
        places.push(PlaceData {
            name: "Desktop".into(),
            path: p.display().to_string().into(),
            icon: "🖥️".into(),
        });
    }

    if let Some(p) = dirs_next::document_dir() {
        places.push(PlaceData {
            name: "Documents".into(),
            path: p.display().to_string().into(),
            icon: "📄".into(),
        });
    }

    if let Some(p) = dirs_next::download_dir() {
        places.push(PlaceData {
            name: "Downloads".into(),
            path: p.display().to_string().into(),
            icon: "📥".into(),
        });
    }

    if let Some(p) = dirs_next::picture_dir() {
        places.push(PlaceData {
            name: "Pictures".into(),
            path: p.display().to_string().into(),
            icon: "🖼️".into(),
        });
    }

    if let Some(p) = dirs_next::audio_dir() {
        places.push(PlaceData {
            name: "Music".into(),
            path: p.display().to_string().into(),
            icon: "🎵".into(),
        });
    }

    if let Some(p) = dirs_next::video_dir() {
        places.push(PlaceData {
            name: "Videos".into(),
            path: p.display().to_string().into(),
            icon: "🎬".into(),
        });
    }

    // Mounted removable drives in /run/media/$USER or /media/$USER
    let user = std::env::var("USER").unwrap_or_default();
    if !user.is_empty() {
        for base in &[format!("/run/media/{}", user), format!("/media/{}", user)] {
            let p = Path::new(base);
            if p.is_dir() {
                if let Ok(entries) = std::fs::read_dir(p) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            let name = entry.file_name().to_string_lossy().into_owned();
                            places.push(PlaceData {
                                name: format!("Drive: {}", name).into(),
                                path: path.display().to_string().into(),
                                icon: "💽".into(),
                            });
                        }
                    }
                }
            }
        }
    }

    if let Some(data) = dirs_next::data_local_dir() {
        let trash_dir = data.join("Trash/files");
        places.push(PlaceData {
            name: "Trash".into(),
            path: trash_dir.display().to_string().into(),
            icon: "🗑️".into(),
        });
    }

    places.push(PlaceData {
        name: "File System".into(),
        path: "/".into(),
        icon: "💾".into(),
    });

    places
}

/// Choose an icon for a file entry based on category, directory, or symlink
fn category_icon(cat: &FileCategory, is_dir: bool, is_symlink: bool) -> &'static str {
    if is_symlink {
        "🔗"
    } else if is_dir {
        "📁"
    } else {
        match cat {
            FileCategory::Code => "💻",
            FileCategory::Document => "📄",
            FileCategory::Image => "🖼️",
            FileCategory::Video => "🎬",
            FileCategory::Audio => "🎵",
            FileCategory::Archive => "📦",
            FileCategory::Executable => "⚙️",
            FileCategory::Data => "📊",
            FileCategory::Directory => "📁",
            FileCategory::Unknown => "📄",
        }
    }
}

/// Map FileCategory to its accent Color.
fn category_color(cat: &FileCategory) -> Color {
    match cat {
        FileCategory::Directory  => Color::from_rgb_u8(240, 240, 240),
        FileCategory::Code       => Color::from_rgb_u8(97,  210, 240),
        FileCategory::Document   => Color::from_rgb_u8(240, 210, 97),
        FileCategory::Image      => Color::from_rgb_u8(192, 97,  240),
        FileCategory::Video      => Color::from_rgb_u8(160, 97,  240),
        FileCategory::Audio      => Color::from_rgb_u8(97,  240, 180),
        FileCategory::Archive    => Color::from_rgb_u8(240, 160, 97),
        FileCategory::Executable => Color::from_rgb_u8(97,  240, 112),
        FileCategory::Data       => Color::from_rgb_u8(240, 126, 97),
        FileCategory::Unknown    => Color::from_rgb_u8(112, 112, 112),
    }
}

/// Map GitStatus to a short label string.
fn git_label(gs: &GitStatus) -> &'static str {
    match gs {
        GitStatus::None | GitStatus::Clean => "",
        GitStatus::Untracked => "U",
        GitStatus::Modified  => "M",
        GitStatus::Staged    => "S",
        GitStatus::Conflict  => "!",
    }
}

/// Map GitStatus to its Color.
fn git_color(gs: &GitStatus) -> Color {
    match gs {
        GitStatus::Untracked => Color::from_rgb_u8(97,  240, 112),
        GitStatus::Modified  => Color::from_rgb_u8(240, 210, 97),
        GitStatus::Staged    => Color::from_rgb_u8(97,  210, 240),
        GitStatus::Conflict  => Color::from_rgb_u8(240, 97,  97),
        _                    => Color::from_rgb_u8(85,  85,  85),
    }
}

/// Convert FileEntry slice into Slint EntryData structs.
fn entries_to_slint(entries: &[FileEntry]) -> Vec<EntryData> {
    entries
        .iter()
        .map(|e| EntryData {
            name:        e.name.clone().into(),
            icon:        category_icon(&e.category, e.is_dir, e.is_symlink).into(),
            size:        if e.is_dir { "—".into() } else { format_bytes(e.size).into() },
            modified:    format_modified(e.modified).into(),
            permissions: e.permissions.clone().into(),
            git_label:   git_label(&e.git_status).into(),
            name_color:  category_color(&e.category),
            git_color:   git_color(&e.git_status),
            is_dir:      e.is_dir,
        })
        .collect()
}
