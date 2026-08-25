// src/core/watcher.rs
//
// Real-time filesystem watcher powered by `notify` (inotify on Linux).
// Emits change events back to the UI layer so the directory listing
// refreshes instantly without a manual reload.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

/// Events the watcher emits upward to the UI/app state.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
    Renamed { from: PathBuf, to: Option<PathBuf> },
}

/// Starts watching `dir` (non-recursively) and returns a receiver for WatchEvents.
/// The watcher is returned so it stays alive; drop it to stop watching.
pub fn watch_directory(
    dir: &Path,
) -> anyhow::Result<(RecommendedWatcher, mpsc::Receiver<WatchEvent>)> {
    let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<Event>>();
    let (event_tx, event_rx) = mpsc::channel::<WatchEvent>();

    // Build watcher with a 50 ms debounce to avoid spamming during bulk operations
    let config = Config::default().with_poll_interval(Duration::from_millis(50));
    let mut watcher = RecommendedWatcher::new(raw_tx, config)?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;

    // Translate raw notify events to our typed WatchEvent on a background thread
    std::thread::spawn(move || {
        while let Ok(result) = raw_rx.recv() {
            match result {
                Ok(event) => {
                    use notify::EventKind::*;
                    let paths = event.paths;
                    let watch_event = match event.kind {
                        Create(_) => paths.into_iter().next().map(WatchEvent::Created),
                        Modify(_) => paths.into_iter().next().map(WatchEvent::Modified),
                        Remove(_) => paths.into_iter().next().map(WatchEvent::Removed),
                        notify::EventKind::Rename(_) => {
                            let mut iter = paths.into_iter();
                            let from = iter.next();
                            let to = iter.next();
                            from.map(|f| WatchEvent::Renamed { from: f, to })
                        }
                        _ => None,
                    };
                    if let Some(ev) = watch_event {
                        if event_tx.send(ev).is_err() {
                            break; // Receiver dropped
                        }
                    }
                }
                Err(e) => log::warn!("Watcher error: {}", e),
            }
        }
    });

    Ok((watcher, event_rx))
}
