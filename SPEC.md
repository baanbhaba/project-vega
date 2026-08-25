# ⚡ PROJECT VEGA — MASTER ARCHITECTURE & SPECIFICATION

```
██╗   ██╗███████╗ ██████╗  █████╗ 
██║   ██║██╔════╝██╔════╝ ██╔══██╗
██║   ██║█████╗  ██║  ███╗███████║
╚██╗ ██╔╝██╔══╝  ██║   ██║██╔══██║
 ╚████╔╝ ███████╗╚██████╔╝██║  ██║
  ╚═══╝  ╚══════╝ ╚═════╝ ╚═╝  ╚═╝
  HIGH-PERFORMANCE GPU FILE MANAGER FOR LINUX
```

---

## 1. EXECUTIVE SUMMARY & MISSION

**Project Vega** is a next-generation, high-performance desktop file manager for Linux designed to directly replace **KDE Dolphin**. 

### The Core Problem with Modern Linux File Managers
* **Dolphin (KDE):** Feature-rich but bogged down by deep legacy dependencies (KIO worker daemons, Baloo desktop indexer, heavy Qt Widgets software rendering, synchronous metadata stalls on network mounts and dense folders).
* **Nautilus / Files (GNOME):** Minimalist to a fault, stripped of power features (no dual-pane, no configurable columns, rigid Adwaita constraints).
* **Thunar / Nemo / PCManFM:** Lightweight, but dated visual aesthetics, lacking modern workflows (Kanban boards, instant fuzzy search, integrated git status, spacebar quick previews).
* **Terminal Managers (Yazi, Ranger):** Incredibly fast and keyboard-driven, but lack first-class GUI capabilities, visual drag-and-drop, full mouse/tablet interactions, and rich multi-window layouts.

### Vega's Value Proposition
1. **Uncompromised Native Speed:** Written in pure Rust 2024 with direct OS syscalls, parallel Rayon directory traversals, and non-blocking streaming I/O.
2. **GPU-Accelerated Native GUI:** Built on **Slint** rendering directly via OpenGL/Vulkan with a constant **60–144 FPS** frame pacing and sub-20ms cold startups.
3. **Power-User Tooling Out of the Box:** Native Git status badges, Spacebar Quick Previews, Regex Bulk Renaming, built-in Multi-format Archive Engine, BLAKE3 Checksums, and an interactive **Kanban Board View**.
4. **Modern Minimalist Aesthetics:** Custom OLED Dark / Slate palette, category-based color differentiation, recency heatmaps, and clean typography.

---

## 2. ARCHITECTURAL OVERVIEW

```mermaid
flowchart TB
    subgraph UI ["GPU Native UI Layer (Slint / OpenGL / Vulkan)"]
        Toolbar["Toolbar & Path Input (Ctrl+L)"]
        Sidebar["Places + Collapsible Directory Tree"]
        ViewEngine["View Engine: Detail Table | Grid Cards | Kanban Board | Disk Analyzer"]
        QuickPreview["Spacebar Quick Preview (Syntax, Images, Markdown, PDF)"]
        CmdPalette["Command Palette (Ctrl+P / Ctrl+K)"]
        OpsQueueModal["File Operations Progress & Queue Modal"]
    end

    subgraph Channel ["Lock-Free Async Pipeline"]
        EventLoop["Slint Event Loop Thread"]
        FlumeSPSC["Flume MPMC / SPSC Ring Buffers"]
    end

    subgraph Core ["Rust High-Performance Engine"]
        Scanner["Streaming Scanner (Chunked dirent reads, Rayon pool)"]
        Watcher["Inotify Real-time FS Watcher (Debounced)"]
        GitEngine["libgit2 Status Engine (Thread-safe LRU Cache)"]
        FuzzyMatcher["Nucleo-Matcher (SIMD Fuzzy Search)"]
        TagStore["JSON Sidecar Tag & Kanban Metadata Engine"]
        FileOpsEngine["Parallel I/O Engine (Copy, Move, Trash, Archive, Checksum)"]
    end

    subgraph Kernel ["Linux Kernel & Filesystem"]
        DirentSys["getdents64 / io_uring Syscalls"]
        InotifySys["inotify Subsystem"]
        FS["EXT4 / Btrfs / ZFS / NFS / SMB"]
    end

    UI <--> EventLoop
    EventLoop <--> FlumeSPSC
    FlumeSPSC <--> Core
    Scanner --> DirentSys
    Watcher --> InotifySys
    FileOpsEngine --> FS
```

---

## 3. BENCHMARK & SYSTEM COMPARISON

| Metric | Vega (Rust + Slint) | KDE Dolphin (C++ / Qt) | Nautilus (C / GTK4) | Yazi (Rust TUI) |
| :--- | :--- | :--- | :--- | :--- |
| **Cold Startup Time** | **~10 – 25 ms** | ~180 – 350 ms | ~120 – 240 ms | ~5 – 10 ms |
| **Idle Memory (RAM)** | **~18 – 35 MB** | ~90 – 160 MB | ~65 – 120 MB | ~15 – 25 MB |
| **100,000 Files In-Memory** | **~45 – 70 MB** | ~220 – 450 MB | ~180 – 320 MB | ~40 – 60 MB |
| **First 100 Rows Render Time** | **< 4 ms** (Streaming) | ~140 ms (Full Stat) | ~95 ms | < 2 ms |
| **Fuzzy Search (50k items)** | **< 2 ms** (nucleo-matcher) | ~80 – 250 ms (Baloo) | ~120 ms | < 2 ms |
| **UI Render Engine** | GPU (Skia / FemtoVG) | Qt Widgets (Software/QPainter) | GTK GSK (Vulkan/GL) | Terminal TUI |
| **Desktop Integrations** | Wayland / X11 / XDG | Full KDE Plasma / KIO | Full GNOME / GVFS | Terminal Only |
| **Git Integration** | Built-in zero-process libgit2 | External KIO plugin | None / Extension | Built-in |
| **Kanban View Mode** | Built-in | None | None | None |

---

## 4. TECH STACK SPECIFICATION

### Backend / Core Engine (Rust 2024 Edition)
* **`tokio` (1.x):** Asynchronous runtime managing background task orchestration.
* **`rayon` (1.x):** Work-stealing CPU thread pool for parallel directory statting, hash computation, and bulk operations.
* **`flume` (0.11):** Lock-free MPMC channels connecting background scanner threads to the UI event loop.
* **`notify` (8.x):** Real-time `inotify` subsystem integration with configurable debouncing.
* **`git2` (0.20):** Direct native C bindings to `libgit2` (avoids spawning `git` CLI subprocesses).
* **`nucleo-matcher` (0.3):** Lock-free, SIMD-accelerated fuzzy matching engine (derived from Helix editor).
* **`trash` (5.x):** Freedesktop.org compliant trash specification implementation.
* **`blake3`, `sha2`, `md-5`:** Cryptographic and high-speed hashing algorithms.
* **`zip`, `tar`, `flate2`, `zstd`, `xz2`, `bzip2`:** Comprehensive streaming archive suite.
* **`open` (5.x):** Desktop portal / `xdg-open` launcher integration.

### Frontend / UI Layer (Slint 1.9+)
* **Declarative `.slint` Markup:** Clean UI definition compiled at build-time to optimized native Rust code.
* **GPU Backend:** FemtoVG / Skia rendering via OpenGL / Vulkan / Software fallback.
* **Virtualized Viewports:** `ScrollView` structures that only render visible rows on screen, scaling to hundreds of thousands of files without allocation overhead.
* **OLED-Tuned Design System:** True black (`#0d0d0d`) base with slate/charcoal surface elevations and vibrant category accents.

---

## 5. CORE ENGINE SUBSYSTEMS

### 5.1. FileEntry Model & Zero-Copy Categorization
Every filesystem object is represented in memory by `core::entry::FileEntry`:

```rust
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub extension: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub category: FileCategory,
    pub git_status: GitStatus,
    pub color_tag: Option<String>,
    pub kanban_tag: Option<String>,
    pub permissions: String,
    pub selected: bool,
}
```

#### Category Taxonomy & Color Mapping:
| Category | Extensions / Conditions | Accent Color | Hex |
| :--- | :--- | :--- | :--- |
| **Directory** | `is_dir == true` | Pure White / Off-White | `#f0f0f0` |
| **Code** | `.rs`, `.py`, `.js`, `.ts`, `.go`, `.c`, `.cpp`, `.zig`, etc. | Neon Cyan | `#61d2f0` |
| **Document** | `.pdf`, `.md`, `.txt`, `.docx`, `.odt`, `.epub`, etc. | Amber Gold | `#f0d261` |
| **Image** | `.png`, `.jpg`, `.jpeg`, `.svg`, `.webp`, `.gif`, `.avif`, etc. | Electric Violet | `#c061f0` |
| **Video** | `.mp4`, `.mkv`, `.mov`, `.webm`, `.avi`, `.m2ts`, etc. | Deep Purple | `#a061f0` |
| **Audio** | `.mp3`, `.flac`, `.wav`, `.ogg`, `.opus`, `.m4a`, etc. | Mint Green | `#61f0b4` |
| **Archive** | `.zip`, `.tar.gz`, `.tar.zst`, `.7z`, `.rar`, `.deb`, `.rpm` | Vibrant Orange | `#f0a061` |
| **Executable** | Unix `+x` bit set (ELF binaries, scripts) | Emerald Green | `#61f070` |
| **Data / Config** | `.json`, `.yaml`, `.toml`, `.csv`, `.sql`, `.db`, `.sqlite` | Coral Peach | `#f07e61` |
| **Unknown** | Unclassified formats | Muted Slate | `#707070` |

---

### 5.2. Streaming Directory Scanner (`core::scanner`)
Traditional file managers call `read_dir` synchronously, then run `stat()` sequentially on every item before rendering the first frame.

**Vega's 4-Stage Scanner Pipeline:**
1. **Dirent Harvest:** Read directory entries in raw kernel order.
2. **Rayon Parallel Statting:** Worker threads parse file metadata (`symlink_metadata`, Unix permissions, category tags) concurrently across available CPU cores.
3. **Early Chunk Dispatch:** The first **100 items** are immediately dispatched via a `flume` channel to Slint within `< 4ms`, rendering the top of the viewport instantly.
4. **Full Sort & Invalidation:** As remaining chunks stream in, the full in-memory list is sorted (Directories first, then Name/Size/Date/Type) without locking the UI thread.
5. **Instant Cancelability:** An `AtomicBool` cancellation token stops in-flight scanning immediately if the user clicks another directory.

---

### 5.3. Real-Time Filesystem Watcher (`core::watcher`)
* Integrates Linux `inotify` via `notify`.
* Uses a **50ms debounce buffer** to prevent event storms during bulk writes or git checkouts.
* Translates low-level events into clean domain events: `Created`, `Modified`, `Removed`, `Renamed`.
* Triggers targeted incremental refreshes rather than full directory rescans.

---

### 5.4. Native Git Status Integration (`core::git`)
* Direct C API binding via `libgit2` (bypasses subprocess overhead).
* Locates repository root by walking up directory ancestry.
* Extracts the active branch name (`git_branch`) and status flags:
  * `U` (Untracked) → Emerald Green (`#61f070`)
  * `M` (Modified) → Yellow Amber (`#f0d261`)
  * `S` (Staged) → Cyan (`#61d2f0`)
  * `!` (Conflict) → Crimson Red (`#f06161`)
* Caches statuses in a `parking_lot::Mutex<HashMap<PathBuf, RepoCache>>` and auto-invalidates on file operations.

---

### 5.5. Tag & Kanban Metadata Engine (`core::tags`)
Vega avoids altering user files or cluttering working directories with hidden dotfiles.

* **Storage Scheme:** All custom tags and Kanban stages are stored in:
  `~/.local/share/vega/tags/<hashed_dir_path>.json`
* **JSON Schema:**
  ```json
  {
    "/home/user/project/src/main.rs": {
      "color": "#61d2f0",
      "kanban": "In Progress",
      "labels": ["backend", "core"]
    }
  }
  ```
* **Performance:** Fast in-memory cache with immediate asynchronous background disk flush.

---

### 5.6. High-Speed Operations Engine (`ops::fileops`)
* **Parallel Copy / Move:** Chunked I/O stream with Rayon worker threads. Move operations prioritize atomic `std::fs::rename` (same filesystem O(1)), gracefully falling back to streaming copy + unlink across mount points.
* **Progress Telemetry:** Real-time speed calculations (MB/s), total bytes transferred, and per-file progress events streamed to the UI.
* **Freedesktop Trash Integration:** Sends items to `~/.local/share/Trash` with `.trashinfo` metadata for seamless restore and `Ctrl+Z` undo capabilities.

---

### 5.7. Checksum Engine (`ops::checksum`)
* **Supported Algorithms:** **BLAKE3** (primary, SIMD parallelized), **SHA256**, **MD5**.
* **1 MB Streaming Buffer:** Zero RAM spikes during large file (10GB+) hashing.
* **Fast Equality Short-Circuit:** Compares file sizes first before computing BLAKE3 hashes to verify duplicate files instantly.

---

### 5.8. Regex Bulk Renamer (`ops::rename`)
* **Regex Engine:** Powered by Rust's `regex` crate with full capture group support (`$1`, `$2`).
* **Counter Formatting:** Supports zero-padded index variables (e.g. `{n:03}` → `001, 002, 003`).
* **Scope Selector:** Apply rules to `Full Filename` or `Name Only` (preserving original extensions).
* **Safe Dry-Run Engine:** Generates a structured preview list of `(from, to)` paths to verify before applying atomic renames.

---

## 6. VIEW MODES SPECIFICATION

Vega supports 4 distinct view modes switchable instantly via keyboard (`F5`–`F8`) or toolbar buttons:

```mermaid
graph LR
    V1["1. Detailed Table (F5)"]
    V2["2. Grid / Card View (F6)"]
    V3["3. Kanban Board (F7)"]
    V4["4. Disk Space Analyzer (F8)"]
```

### 1. Detailed Table View (`F5`)
* Columnar layout: `Icon`, `Name`, `Git Status`, `Size`, `Modified Date`, `Permissions`, `Tag Badge`.
* Sortable column headers with ascending/descending toggles.
* Virtualized scrolling rendering up to 1,000,000 files with constant 60 FPS.

### 2. Grid / Card View (`F6`)
* Responsive multi-column card layout with large category icons and thumbnail previews.
* Shows filename, size, and category badge per card.

### 3. Kanban Board View (`F7`)
Transforms any folder into an interactive visual workflow board.
* **Mode A: Workflow Stages (Default):**
  * Columns: `Backlog` | `In Progress` | `Review` | `Done` (Customizable).
  * Drag-and-drop or right-click to shift files between stages.
* **Mode B: File-Type Categories:**
  * Columns: `Code` | `Media` | `Documents` | `Archives` | `Other`.
  * Auto-sorts files into type-based swimlanes.

### 4. Disk Space Analyzer (`F8` — NCDU Style)
* Interactive visual breakdown of disk space in the current directory.
* Color-coded proportional percentage bars (`[████████░░░░] 68%`).
* Instant visual identification of large files, hidden node_modules, build artifacts, or stale cache folders.

---

## 7. USER INTERFACE & DESIGN SYSTEM

### 7.1. Color Palette (OLED Minimalist)
* **Base Background:** `#0d0d0d` (OLED True Black)
* **Surface Panel:** `#161616` (Dark Charcoal)
* **Elevated Cards:** `#1e1e1e` (Slate Card Background)
* **Hover State:** `#252525`
* **Selection State:** `#2a2a3a` (Tuned Blue-Purple Tint)
* **Primary Accent:** `#7c6af0` (Vibrant Purple-Violet)
* **Accent Dim:** `#3d3578` (Badge background)
* **Border Lines:** `#2a2a2a`

### 7.2. Layout Architecture
* **Top Toolbar (42px):** Navigation arrows (`◀`, `▶`, `↑`), clickable Breadcrumbs / Path Input, Git Branch badge (`⎇ main`), View Mode switchers, Search input, Settings trigger.
* **Sidebar (220px):**
  * **Places:** Home, Desktop, Documents, Downloads, Pictures, Music, Videos, Trash.
  * **Mounted Drives:** Auto-discovered storage devices and network mounts.
  * **Collapsible Directory Tree:** Hierarchical folder tree for rapid navigation.
* **Main Viewport (Flexible):** Virtualized view displaying active mode.
* **Bottom Status Bar (24px):** Total item count, selection summary, free disk space on current mount.

---

## 8. KEYBOARD SHORTCUT MATRIX

| Action | Primary Shortcut | Secondary / Vim |
| :--- | :--- | :--- |
| **Open Item / Enter Dir** | `Enter` | `l` / Double-Click |
| **Go to Parent Directory** | `Alt+Up` / `Backspace` | `h` / `↑` Toolbar Button |
| **Navigate Back / Forward** | `Alt+Left` / `Alt+Right` | `[` / `]` |
| **Focus Path / Location Bar** | `Ctrl+L` | `/` |
| **Command Palette** | `Ctrl+P` / `Ctrl+K` | `:` |
| **Spacebar Quick Preview** | `Space` | `p` |
| **Instant Type-to-Filter** | Start typing alphanumeric | `/` |
| **Toggle Hidden Files** | `Ctrl+H` | `.` |
| **New Folder** | `Ctrl+Shift+N` | `F7` |
| **New Empty File** | `Ctrl+N` | — |
| **Rename (Inline / Modal)** | `F2` | `r` |
| **Bulk Rename Tool** | `Ctrl+F2` | `R` |
| **Copy / Cut / Paste** | `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | `y` / `d` / `p` |
| **Move to Trash** | `Delete` | `x` |
| **Permanent Delete** | `Shift+Delete` | `X` |
| **Undo Last File Operation** | `Ctrl+Z` | `u` |
| **Switch to Detail View** | `F5` / `Ctrl+1` | — |
| **Switch to Grid View** | `F6` / `Ctrl+2` | — |
| **Switch to Kanban View** | `F7` / `Ctrl+3` | — |
| **Switch to Disk Space Analyzer** | `F8` / `Ctrl+4` | — |
| **Open Integrated Terminal** | `F4` / `Ctrl+` ` ` | — |

---

## 9. IMPLEMENTATION ROADMAP

```mermaid
gantt
    title Project Vega Engineering Roadmap
    dateFormat  YYYY-MM-DD
    section Foundation
    Workspace Scaffold & Core Dependencies       :done,    m1_1, 2026-08-01, 2026-08-05
    FileEntry & Category Engine                  :done,    m1_2, 2026-08-05, 2026-08-10
    Streaming Rayon Directory Scanner            :done,    m1_3, 2026-08-10, 2026-08-15
    Slint OLED UI Shell & Virtualized Rows       :done,    m1_4, 2026-08-15, 2026-08-25
    section Navigation & Core Ops
    Double-click Navigation & Open Default App   :active,  m2_1, 2026-08-26, 2026-09-02
    History Stack (Back / Forward)               :active,  m2_2, 2026-08-28, 2026-09-05
    Column Sorting & Hidden Toggle               :         m2_3, 2026-09-05, 2026-09-10
    In-place Rename & New Folder/File Modals     :         m2_4, 2026-09-10, 2026-09-18
    section Power Features
    Fuzzy Type-to-Filter Bar                     :         m3_1, 2026-09-18, 2026-09-25
    Spacebar Floating Quick Preview              :         m3_2, 2026-09-25, 2026-10-05
    Command Palette Overlay (Ctrl+P)             :         m3_3, 2026-10-05, 2026-10-15
    section Multi-View Modes
    Interactive Kanban Board View                :         m4_1, 2026-10-15, 2026-10-28
    Grid / Card View with Thumbnails             :         m4_2, 2026-10-28, 2026-11-10
    Disk Space Analyzer (NCDU Mode)              :         m4_3, 2026-11-10, 2026-11-20
    section Polish & Packaging
    Dual-Pane Split View & Multi-Tab             :         m5_1, 2026-11-20, 2026-12-05
    AppImage / Flatpak / Arch PKGBUILD / RPM     :         m5_2, 2026-12-05, 2026-12-15
```

---

## 10. REPOSITORY INFORMATION

* **GitHub Repository:** [https://github.com/baanbhaba/project-vega](https://github.com/baanbhaba/project-vega)
* **License:** MIT License
* **Target Platforms:** Linux (Wayland & X11), x86_64, aarch64
* **Build Command:** `cargo build --release`
* **Binary Location:** `target/release/vega`

---
*Generated for Project Vega Core Development Team.*
