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
use std::time::Duration;

use parking_lot::Mutex;
use slint::{Color, ModelRc, SharedString, VecModel};

use crate::core::entry::{FileCategory, FileEntry, GitStatus};
use crate::core::scanner::{scan_directory, ScanConfig, ScanEvent};
use crate::core::watcher::watch_directory;
use crate::utils::human::{format_bytes, format_modified};
use crate::utils::paths::{collapse_home, expand_tilde, xdg_places};

// Include the Slint generated bindings
slint::include_modules!();

/// Global application state
struct AppState {
    current_dir: PathBuf,
    history: Vec<PathBuf>,
    history_idx: usize,
    show_hidden: bool,
    all_entries: Vec<FileEntry>,
    displayed_entries: Vec<FileEntry>,
    selected_idx: Option<usize>,
    filter_query: String,
    cancel_token: Arc<AtomicBool>,
    watcher: Option<notify::RecommendedWatcher>,
}

impl AppState {
    fn new(start_dir: PathBuf) -> Self {
        Self {
            current_dir: start_dir.clone(),
            history: vec![start_dir],
            history_idx: 0,
            show_hidden: false,
            all_entries: Vec::new(),
            displayed_entries: Vec::new(),
            selected_idx: None,
            filter_query: String::new(),
            cancel_token: Arc::new(AtomicBool::new(false)),
            watcher: None,
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let window = MainWindow::new()?;
    let weak = window.as_weak();

    // Determine initial directory
    let start_dir: PathBuf = std::env::args()
        .nth(1)
        .map(|s| expand_tilde(&s))
        .unwrap_or_else(|| dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("/")));

    let start_dir = if start_dir.is_dir() {
        start_dir
    } else {
        dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("/"))
    };

    let state = Arc::new(Mutex::new(AppState::new(start_dir.clone())));

    // ── Populate sidebar places ──────────────────────────────────────────────
    {
        if let Some(w) = weak.upgrade() {
            let places_vec: Vec<PlaceData> = xdg_places()
                .into_iter()
                .map(|(name, path)| PlaceData {
                    name: name.into(),
                    path: path.display().to_string().into(),
                })
                .collect();
            let model = VecModel::from(places_vec);
            w.set_places(ModelRc::new(model));
        }
    }

    // ── Initial Navigation ───────────────────────────────────────────────────
    navigate_to(weak.clone(), state.clone(), start_dir, false);

    // ── Toolbar & Navigation Callbacks ───────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_navigate_to(move |path_str| {
            let expanded = expand_tilde(path_str.as_str());
            if expanded.is_dir() {
                navigate_to(weak_c.clone(), state_c.clone(), expanded, true);
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

    // ── Double Click: Open folder or launch file ─────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_entry_double_clicked(move |index| {
            let entry_opt = {
                let s = state_c.lock();
                if index >= 0 && (index as usize) < s.displayed_entries.len() {
                    Some(s.displayed_entries[index as usize].clone())
                } else {
                    None
                }
            };

            if let Some(entry) = entry_opt {
                if entry.is_dir {
                    navigate_to(weak_c.clone(), state_c.clone(), entry.path, true);
                } else {
                    log::info!("Opening file: {}", entry.path.display());
                    if let Err(e) = open::that(&entry.path) {
                        log::error!("Failed to open file {}: {}", entry.path.display(), e);
                    }
                }
            }
        });
    }

    // ── Single Click: Selection & Status Bar update ──────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_entry_clicked(move |index| {
            let mut s = state_c.lock();
            let idx = index as usize;
            if idx < s.displayed_entries.len() {
                s.selected_idx = Some(idx);
                let entry = &s.displayed_entries[idx];
                let selected_info = format!(
                    "{} ({} • {} • {})",
                    entry.name,
                    format_bytes(entry.size),
                    entry.permissions,
                    format_modified(entry.modified)
                );

                // Update selection state in Slint model
                for (i, e) in s.displayed_entries.iter_mut().enumerate() {
                    e.selected = i == idx;
                }
                let ui_rows = entries_to_slint(&s.displayed_entries);

                if let Some(w) = weak_c.upgrade() {
                    w.set_selected_info(selected_info.into());
                    w.set_entries(ModelRc::new(VecModel::from(ui_rows)));
                }
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

    // ── New Folder ───────────────────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_create_folder(move || {
            let dir = { state_c.lock().current_dir.clone() };
            let mut new_dir = dir.join("New Folder");
            let mut counter = 2;
            while new_dir.exists() {
                new_dir = dir.join(format!("New Folder ({})", counter));
                counter += 1;
            }

            if let Ok(_) = std::fs::create_dir(&new_dir) {
                log::info!("Created directory: {}", new_dir.display());
                navigate_to(weak_c.clone(), state_c.clone(), dir, false);
            }
        });
    }

    // ── Open Terminal ────────────────────────────────────────────────────────
    {
        let state_c = state.clone();
        window.on_open_terminal(move || {
            let dir = { state_c.lock().current_dir.clone() };
            // Try kitty first, then fallback to xdg-terminal-exec or xterm
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

    // ── Trash Selected ───────────────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_trash_selected(move || {
            let (target_path, current_dir) = {
                let s = state_c.lock();
                if let Some(idx) = s.selected_idx {
                    if idx < s.displayed_entries.len() {
                        (Some(s.displayed_entries[idx].path.clone()), s.current_dir.clone())
                    } else {
                        (None, s.current_dir.clone())
                    }
                } else {
                    (None, s.current_dir.clone())
                }
            };

            if let Some(path) = target_path {
                log::info!("Moving to trash: {}", path.display());
                match trash::delete(&path) {
                    Ok(_) => {
                        navigate_to(weak_c.clone(), state_c.clone(), current_dir, false);
                    }
                    Err(e) => {
                        log::error!("Failed to trash {}: {}", path.display(), e);
                    }
                }
            }
        });
    }

    // ── Refresh ──────────────────────────────────────────────────────────────
    {
        let weak_c = weak.clone();
        let state_c = state.clone();
        window.on_refresh(move || {
            let dir = { state_c.lock().current_dir.clone() };
            navigate_to(weak_c.clone(), state_c.clone(), dir, false);
        });
    }

    // ── Command Palette placeholder ──────────────────────────────────────────
    window.on_open_command_palette(|| {
        log::info!("Command palette requested");
    });

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
    let (cancel_token, show_hidden) = {
        let mut s = state.lock();

        // Signal in-flight scan to cancel
        s.cancel_token.store(true, Ordering::Relaxed);
        let new_token = Arc::new(AtomicBool::new(false));
        s.cancel_token = new_token.clone();

        s.current_dir = dir.clone();
        s.selected_idx = None;

        if push_history {
            if s.history.get(s.history_idx) != Some(&dir) {
                let keep_len = s.history_idx + 1;
                s.history.truncate(keep_len);
                s.history.push(dir.clone());
                s.history_idx = s.history.len() - 1;
            }
        }

        (new_token, s.show_hidden)
    };

    // Update UI headers immediately
    if let Some(w) = weak.upgrade() {
        w.set_current_path(collapse_home(&dir).into());
        w.set_item_count("Loading…".into());
        w.set_selected_info("".into());

        // Update disk info
        if let Some(disk) = get_disk_free(&dir) {
            w.set_disk_info(disk.into());
        }
    }

    // ── Setup Real-Time inotify Watcher ──────────────────────────────────────
    {
        let weak_watcher = weak.clone();
        let state_watcher = state.clone();
        let dir_watcher = dir.clone();

        match watch_directory(&dir) {
            Ok((watcher, rx)) => {
                state.lock().watcher = Some(watcher);

                std::thread::spawn(move || {
                    let mut last_event = std::time::Instant::now();
                    while let Ok(event) = rx.recv() {
                        // Debounce by 150ms
                        if last_event.elapsed() < Duration::from_millis(150) {
                            continue;
                        }
                        last_event = std::time::Instant::now();

                        // Check if we are still on the same directory
                        let is_current = {
                            state_watcher.lock().current_dir == dir_watcher
                        };
                        if !is_current {
                            break;
                        }

                        let w_ref = weak_watcher.clone();
                        let s_ref = state_watcher.clone();
                        let d_ref = dir_watcher.clone();

                        let _ = slint::invoke_from_event_loop(move || {
                            navigate_to(w_ref, s_ref, d_ref, false);
                        });
                    }
                });
            }
            Err(e) => {
                log::warn!("Could not watch directory {}: {}", dir.display(), e);
            }
        }
    }

    // ── Background Directory Scan Pipeline ───────────────────────────────────
    let dir_scan = dir.clone();
    let state_scan = state.clone();
    let weak_scan = weak.clone();

    std::thread::spawn(move || {
        let config = ScanConfig {
            show_hidden,
            ..ScanConfig::default()
        };
        let rx = scan_directory(dir_scan.clone(), config, cancel_token);
        let mut collected: Vec<FileEntry> = Vec::new();

        // Check Git status for badge
        let git_info = core::git::find_repo_root(&dir_scan)
            .and_then(|root| core::git::get_repo_status(&root));

        let (git_branch, git_statuses) = match git_info {
            Some((branch, statuses)) => (branch, statuses),
            None => (String::new(), std::collections::HashMap::new()),
        };

        let branch_s: SharedString = git_branch.into();
        let w_ref = weak_scan.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(w) = w_ref.upgrade() {
                w.set_git_branch(branch_s);
            }
        });

        loop {
            match rx.recv() {
                Ok(ScanEvent::Entries(mut batch)) => {
                    for entry in &mut batch {
                        if let Some(gs) = git_statuses.get(&entry.path) {
                            entry.git_status = gs.clone();
                        }
                    }
                    collected.extend(batch);
                }
                Ok(ScanEvent::Done { total }) => {
                    let mut s = state_scan.lock();
                    s.all_entries = collected.clone();

                    let query = s.filter_query.clone();
                    let displayed = if query.trim().is_empty() {
                        collected
                    } else {
                        core::filter::fuzzy_filter(&collected, &query)
                    };
                    s.displayed_entries = displayed.clone();

                    let ui_rows = entries_to_slint(&displayed);
                    let count_str: SharedString = if query.trim().is_empty() {
                        format!("{} item{}", total, if total == 1 { "" } else { "s" }).into()
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
            size:        format_bytes(e.size).into(),
            modified:    format_modified(e.modified).into(),
            permissions: e.permissions.clone().into(),
            git_label:   git_label(&e.git_status).into(),
            name_color:  category_color(&e.category),
            git_color:   git_color(&e.git_status),
            is_dir:      e.is_dir,
            selected:    e.selected,
        })
        .collect()
}
