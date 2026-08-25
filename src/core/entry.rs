// src/core/entry.rs
//
// Compact, cache-friendly representation of a filesystem entry.
// Designed to minimize heap allocation and maximize CPU cache locality
// during large directory traversals.

use std::path::PathBuf;
use std::time::SystemTime;
use serde::{Deserialize, Serialize};

/// Broad category of a file used for color-coding and Kanban grouping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileCategory {
    Directory,
    Code,       // .rs .py .js .ts .go .c .cpp .h .java .rb etc.
    Document,   // .pdf .md .txt .docx .odt etc.
    Image,      // .png .jpg .svg .gif .webp etc.
    Video,      // .mp4 .mkv .mov .avi etc.
    Audio,      // .mp3 .flac .wav .ogg etc.
    Archive,    // .zip .tar .gz .zst .7z .rar etc.
    Executable, // ELF / scripts with +x bit
    Data,       // .json .yaml .toml .csv .db etc.
    Unknown,
}

/// Git status of a file relative to its repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GitStatus {
    #[default]
    None,           // Not in a git repo or ignored
    Untracked,      // New file, not yet added
    Modified,       // Tracked, has changes
    Staged,         // Added to index
    Conflict,       // Merge conflict
    Clean,          // Tracked, no changes
}

/// A single file system entry — the primary data unit in Vega.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Full absolute path of the entry.
    pub path: PathBuf,

    /// Filename (not full path) for display — pre-extracted to avoid repeated allocation.
    pub name: String,

    /// File extension in lowercase (e.g. "rs", "png"). Empty for directories.
    pub extension: String,

    /// Whether this is a directory.
    pub is_dir: bool,

    /// Whether this is a symlink.
    pub is_symlink: bool,

    /// File size in bytes. 0 for directories (sum computed lazily on demand).
    pub size: u64,

    /// Last modification time.
    pub modified: Option<SystemTime>,

    /// Detected broad category for color-coding and grouping.
    pub category: FileCategory,

    /// Git status if inside a repository.
    pub git_status: GitStatus,

    /// User-assigned custom color tag (optional).
    pub color_tag: Option<String>,

    /// User-assigned Kanban stage (e.g. "Backlog", "Done").
    pub kanban_tag: Option<String>,

    /// Unix permissions as a display string (e.g. "rwxr-xr-x").
    pub permissions: String,

    /// Whether the entry is currently selected in the UI.
    #[serde(skip)]
    pub selected: bool,
}

impl FileEntry {
    /// Construct a FileEntry from a DirEntry.
    /// Metadata is read immediately; heavy ops (MIME, thumbnails) are deferred.
    pub fn from_path(path: PathBuf) -> anyhow::Result<Self> {
        let meta = std::fs::symlink_metadata(&path)?;
        let is_symlink = meta.file_type().is_symlink();
        let is_dir = if is_symlink {
            // Follow symlink to check if target is a directory
            std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false)
        } else {
            meta.is_dir()
        };

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let extension = if is_dir {
            String::new()
        } else {
            path.extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };

        let size = if is_dir { 0 } else { meta.len() };
        let modified = meta.modified().ok();
        let category = categorize(&extension, is_dir, &meta);
        let permissions = format_permissions(&meta);

        Ok(FileEntry {
            path,
            name,
            extension,
            is_dir,
            is_symlink,
            size,
            modified,
            category,
            git_status: GitStatus::None,
            color_tag: None,
            kanban_tag: None,
            permissions,
            selected: false,
        })
    }
}

/// Determine a FileCategory from extension, is_dir flag, and metadata.
fn categorize(ext: &str, is_dir: bool, meta: &std::fs::Metadata) -> FileCategory {
    if is_dir {
        return FileCategory::Directory;
    }

    // Check executable bit on Linux
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.mode() & 0o111 != 0 && !matches!(ext, "sh" | "py" | "rb" | "pl") {
            return FileCategory::Executable;
        }
    }

    match ext {
        // Code
        "rs" | "py" | "js" | "ts" | "jsx" | "tsx" | "go" | "c" | "cpp" | "cc" | "h"
        | "hpp" | "java" | "kt" | "swift" | "rb" | "php" | "cs" | "fs" | "ml" | "lua"
        | "zig" | "nim" | "dart" | "scala" | "r" | "jl" | "ex" | "exs" | "clj" | "hs"
        | "elm" | "vue" | "svelte" | "sh" | "bash" | "fish" | "zsh" | "ps1" | "nix" => FileCategory::Code,

        // Documents
        "pdf" | "md" | "txt" | "rst" | "org" | "tex" | "docx" | "doc" | "odt"
        | "rtf" | "epub" | "mobi" | "pages" => FileCategory::Document,

        // Images
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" | "tiff"
        | "tif" | "avif" | "heic" | "raw" | "cr2" | "nef" | "xcf" | "psd" => FileCategory::Image,

        // Video
        "mp4" | "mkv" | "mov" | "avi" | "webm" | "flv" | "wmv" | "m4v" | "ogv"
        | "3gp" | "m2ts" => FileCategory::Video,

        // Audio
        "mp3" | "flac" | "wav" | "ogg" | "opus" | "m4a" | "aac" | "wma" | "aiff"
        | "ape" | "dsf" => FileCategory::Audio,

        // Archives
        "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "lz4"
        | "cab" | "iso" | "img" | "deb" | "rpm" | "apk" => FileCategory::Archive,

        // Data / Config
        "json" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "env"
        | "csv" | "tsv" | "xml" | "sql" | "db" | "sqlite" | "proto" | "lock" => FileCategory::Data,

        _ => FileCategory::Unknown,
    }
}

/// Format Unix permissions as a human-readable string (e.g. "rwxr-xr-x").
fn format_permissions(meta: &std::fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mode = meta.mode();
        let chars = |shift: u32| {
            let bits = (mode >> shift) & 0o7;
            format!(
                "{}{}{}",
                if bits & 0o4 != 0 { "r" } else { "-" },
                if bits & 0o2 != 0 { "w" } else { "-" },
                if bits & 0o1 != 0 { "x" } else { "-" },
            )
        };
        format!("{}{}{}", chars(6), chars(3), chars(0))
    }
    #[cfg(not(unix))]
    {
        String::from("?????????")
    }
}
