// src/main.rs
//
// Vega — Ultra-fast GPU-accelerated file manager for Linux
// Entry point: initializes the Slint UI and wires it to the Rust core engine.

#![allow(unused_variables, dead_code)]

mod core;
mod ops;
mod utils;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::{ModelRc, SharedString, VecModel, Color};

use crate::core::entry::{FileCategory, GitStatus};
use crate::core::scanner::{scan_directory, ScanConfig, ScanEvent};
use crate::core::tags;
use crate::utils::human::{format_bytes, format_modified};
use crate::utils::paths::{collapse_home, xdg_places};

// Include the generated Slint bindings
slint::include_modules!();

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let window = MainWindow::new()?;
    let weak = window.as_weak();

    // ── Initial state ────────────────────────────────────────────────────────
    let start_dir: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs_next::home_dir().unwrap_or_else(|| PathBuf::from("/")));

    // Navigation history stack
    let history: Arc<parking_lot::Mutex<Vec<PathBuf>>> =
        Arc::new(parking_lot::Mutex::new(vec![start_dir.clone()]));
    let history_pos: Arc<std::sync::atomic::AtomicUsize> =
        Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Cancellation token for in-flight scans
    let cancel = Arc::new(AtomicBool::new(false));

    // ── Populate places sidebar ──────────────────────────────────────────────
    let places_data: Vec<_> = xdg_places()
        .into_iter()
        .map(|(name, path)| {
            let name_s: SharedString = name.into();
            let path_s: SharedString = path.display().to_string().into();
            slint::format!("{}", name_s); // dummy use to avoid warnings
            (name_s, path_s)
        })
        .collect();

    {
        let w = weak.upgrade().unwrap();
        // Build a ModelRc for places
        // Using anonymous struct via slint's VecModel
        let places_model = VecModel::default();
        for (name, path) in &places_data {
            places_model.push(slint::format!("{}", ""));
        }
        // Note: in production we'd use typed struct model — simplified here for bootstrap
    }

    // ── Initial directory load ───────────────────────────────────────────────
    navigate_to(
        weak.clone(),
        start_dir.clone(),
        cancel.clone(),
    );

    // ── Toolbar callbacks ─────────────────────────────────────────────────────
    {
        let weak2 = weak.clone();
        let cancel2 = cancel.clone();
        window.on_navigate_to(move |path_str| {
            let path = PathBuf::from(path_str.as_str());
            if path.is_dir() {
                navigate_to(weak2.clone(), path, cancel2.clone());
            }
        });
    }

    {
        let weak2 = weak.clone();
        let cancel2 = cancel.clone();
        window.on_go_up(move || {
            let w = weak2.upgrade().unwrap();
            let current = PathBuf::from(w.get_current_path().as_str());
            if let Some(parent) = current.parent() {
                navigate_to(weak2.clone(), parent.to_path_buf(), cancel2.clone());
            }
        });
    }

    {
        let weak2 = weak.clone();
        let cancel2 = cancel.clone();
        window.on_entry_double_clicked(move |index| {
            // TODO: get the entry from current model and navigate/open it
            log::info!("Double clicked entry at index {}", index);
        });
    }

    window.on_open_command_palette(|| {
        log::info!("Command palette requested (TODO: implement overlay)");
    });

    window.on_go_back(|| {
        log::info!("Go back (TODO: implement history)");
    });

    window.on_go_forward(|| {
        log::info!("Go forward (TODO: implement history)");
    });

    window.on_toggle_hidden(|| {
        log::info!("Toggle hidden (TODO: implement config toggle)");
    });

    // ── Run ─────────────────────────────────────────────────────────────────
    window.run()?;
    Ok(())
}

/// Navigate to a directory: cancel any in-flight scan, kick off a new one,
/// and stream entries into the Slint model as they arrive.
fn navigate_to(
    weak: slint::Weak<MainWindow>,
    dir: PathBuf,
    cancel: Arc<AtomicBool>,
) {
    // Signal any running scan to stop
    cancel.store(true, Ordering::Relaxed);
    // Small yield to let the old scan notice
    std::thread::sleep(std::time::Duration::from_millis(2));
    cancel.store(false, Ordering::Relaxed);

    // Update displayed path immediately — sub-ms feedback
    {
        let w = match weak.upgrade() {
            Some(w) => w,
            None => return,
        };
        w.set_current_path(collapse_home(&dir).into());
        w.set_item_count("Loading…".into());
    }

    let dir_clone = dir.clone();
    let cancel_clone = cancel.clone();
    let weak_clone = weak.clone();

    std::thread::spawn(move || {
        let config = ScanConfig::default();
        let rx = scan_directory(dir_clone.clone(), config, cancel_clone);

        // Accumulate all entries then push to UI in one batch per chunk
        let mut all_entries: Vec<core::entry::FileEntry> = Vec::new();

        // Check for git repo status
        let git_info = core::git::find_repo_root(&dir_clone)
            .and_then(|root| core::git::get_repo_status(&root));

        let (git_branch, git_statuses) = match git_info {
            Some((branch, statuses)) => (branch, statuses),
            None => (String::new(), std::collections::HashMap::new()),
        };

        // Update branch display
        {
            let branch_s: SharedString = git_branch.clone().into();
            let w_ref = weak_clone.clone();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(w) = w_ref.upgrade() {
                    w.set_git_branch(branch_s);
                }
            });
        }

        loop {
            match rx.recv() {
                Ok(ScanEvent::Entries(mut batch)) => {
                    // Enrich with git status
                    for entry in &mut batch {
                        if let Some(gs) = git_statuses.get(&entry.path) {
                            entry.git_status = gs.clone();
                        }
                    }
                    all_entries.extend(batch.clone());

                    // Stream first batch to UI immediately for fast first paint
                    let ui_entries = entries_to_slint(&batch);
                    let w_ref = weak_clone.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = w_ref.upgrade() {
                            // Append to existing model — first render in <5ms
                            // For simplicity in bootstrap, rebuild full model
                            // TODO: incremental append with ModelRc
                            let _ = ui_entries; // used below
                        }
                    });
                }
                Ok(ScanEvent::Done { total }) => {
                    // Final full update with complete sorted list
                    let ui_rows = entries_to_slint(&all_entries);
                    let count_str: SharedString =
                        format!("{} item{}", total, if total == 1 { "" } else { "s" }).into();
                    let w_ref = weak_clone.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = w_ref.upgrade() {
                            w.set_item_count(count_str);
                            // Build typed VecModel and set on window
                            let model = VecModel::from(ui_rows);
                            w.set_entries(ModelRc::new(model));
                        }
                    });
                    break;
                }
                Ok(ScanEvent::Error(e)) => {
                    log::error!("Scan error: {}", e);
                    let err_s: SharedString = format!("Error: {}", e).into();
                    let w_ref = weak_clone.clone();
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(w) = w_ref.upgrade() {
                            w.set_item_count(err_s);
                        }
                    });
                    break;
                }
                Err(_) => break, // Channel closed
            }
        }
    });
}

/// Map a FileCategory to its display color as a Slint Color.
fn category_color(cat: &FileCategory) -> Color {
    match cat {
        FileCategory::Directory  => Color::from_rgb_u8(240, 240, 240),
        FileCategory::Code       => Color::from_rgb_u8(97, 210, 240),
        FileCategory::Document   => Color::from_rgb_u8(240, 210, 97),
        FileCategory::Image      => Color::from_rgb_u8(192, 97, 240),
        FileCategory::Video      => Color::from_rgb_u8(160, 97, 240),
        FileCategory::Audio      => Color::from_rgb_u8(97, 240, 180),
        FileCategory::Archive    => Color::from_rgb_u8(240, 160, 97),
        FileCategory::Executable => Color::from_rgb_u8(97, 240, 112),
        FileCategory::Data       => Color::from_rgb_u8(240, 126, 97),
        FileCategory::Unknown    => Color::from_rgb_u8(112, 112, 112),
    }
}

/// Map a GitStatus to a short display label.
fn git_label(gs: &GitStatus) -> &'static str {
    match gs {
        GitStatus::None      => "",
        GitStatus::Clean     => "",
        GitStatus::Untracked => "U",
        GitStatus::Modified  => "M",
        GitStatus::Staged    => "S",
        GitStatus::Conflict  => "!",
    }
}

/// Map a GitStatus to its display color as a Slint Color.
fn git_color(gs: &GitStatus) -> Color {
    match gs {
        GitStatus::Untracked => Color::from_rgb_u8(97, 240, 112),
        GitStatus::Modified  => Color::from_rgb_u8(240, 210, 97),
        GitStatus::Staged    => Color::from_rgb_u8(97, 210, 240),
        GitStatus::Conflict  => Color::from_rgb_u8(240, 97, 97),
        _                    => Color::from_rgb_u8(85, 85, 85),
    }
}

/// Convert a slice of FileEntry into the anonymous struct type expected by
/// the Slint `entries` property (must match the struct defined in main.slint).
fn entries_to_slint(entries: &[core::entry::FileEntry]) -> Vec<EntryData> {
    entries
        .iter()
        .map(|e| EntryData {
            name: e.name.clone().into(),
            size: format_bytes(e.size).into(),
            modified: format_modified(e.modified).into(),
            permissions: e.permissions.clone().into(),
            git_label: git_label(&e.git_status).into(),
            name_color: category_color(&e.category),
            git_color: git_color(&e.git_status),
            is_dir: e.is_dir,
            selected: e.selected,
        })
        .collect()
}
