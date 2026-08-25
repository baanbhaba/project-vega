# ⚡ Vega — A Blazing-Fast File Manager for Linux

> A GPU-accelerated Dolphin replacement built entirely in **Rust + Slint**.  
> Speed-first. Aesthetics second. No KDE bloat. No Electron.

---

## Why Vega?

| | Vega | Dolphin |
|---|---|---|
| **Startup time** | ~10–25 ms | ~150–350 ms |
| **Idle RAM** | ~15–35 MB | ~80–160 MB |
| **100k file directory** | ~40–70 MB | ~200–400+ MB |
| **UI rendering** | GPU via Slint (60–144 FPS) | Qt Widgets (60 FPS) |
| **Dependency footprint** | Single binary | Full KDE Frameworks |

---

## Features

### ✅ Built
- **Streaming directory scanner** — Rayon parallel stat, chunked rendering, first frame in `<5ms`
- **Category color coding** — Code (Cyan), Media (Violet), Archives (Orange), Executables (Emerald), Documents (Amber)
- **Git status per file** — Branch name, Modified/Untracked/Staged/Conflict badges via `libgit2`
- **Real-time inotify watcher** — Directory listing updates the instant files change
- **Instant fuzzy filter** — `nucleo-matcher` engine (same as Helix editor), substring fast-path
- **Kanban + color tag store** — Non-invasive JSON sidecars in `~/.local/share/vega/`
- **Parallel file operations** — Copy/move with progress streaming, Freedesktop Trash, permanent delete
- **Built-in archives** — Extract & compress: `.zip`, `.tar.gz`, `.tar.zst`, `.tar.bz2`, `.tar.xz`
- **Checksums** — BLAKE3, SHA256, MD5
- **Regex bulk rename** — Capture groups, zero-padded counters (`{n:03}`), dry-run preview
- **Dark OLED UI** — Slint GPU frontend, slate/charcoal palette with accent colors

### 🔜 Roadmap
- Multi-tab support
- Dual-pane split view
- Sidebar collapsible tree
- Grid / Card view with thumbnails
- Kanban board UI
- Spacebar quick preview (code, images, markdown, PDF)
- Context menu (right-click)
- Command palette overlay (`Ctrl+P`)
- Disk space analyzer (ncdu-style)

---

## Stack

| Layer | Technology |
|---|---|
| Language | **Rust 1.97+** |
| UI Framework | **Slint 1.9** (GPU, OpenGL/Vulkan) |
| Async / Threading | **Tokio** + **Rayon** |
| File watching | **notify** (inotify/fanotify) |
| Git status | **git2** (libgit2 bindings) |
| Fuzzy search | **nucleo-matcher** |
| Archives | zip, tar, flate2, zstd, xz2, bzip2 |
| Checksums | blake3, sha2, md-5 |

---

## Build

```bash
# Install system deps (Fedora)
sudo dnf install -y fontconfig-devel freetype-devel libxkbcommon-devel \
     wayland-devel mesa-libGL-devel libinput-devel

# Build debug
cargo build

# Build optimized release (LTO, stripped binary)
cargo build --release

# Run
./target/release/vega

# Run in a specific directory
./target/release/vega /path/to/dir
```

---

## Project Structure

```
vega/
├── build.rs              # Slint UI compiler
├── ui/
│   └── main.slint        # GPU UI definition (dark OLED theme)
└── src/
    ├── main.rs           # App entry point + Slint wiring
    ├── core/
    │   ├── entry.rs      # FileEntry model + category detection
    │   ├── scanner.rs    # Streaming parallel directory scanner
    │   ├── watcher.rs    # inotify real-time watcher
    │   ├── filter.rs     # Instant fuzzy filter
    │   ├── git.rs        # Git status reader (libgit2)
    │   └── tags.rs       # Kanban + color tag metadata store
    ├── ops/
    │   ├── fileops.rs    # Parallel copy/move/trash/delete
    │   ├── archive.rs    # Archive compress/extract
    │   ├── checksum.rs   # BLAKE3/SHA256/MD5
    │   └── rename.rs     # Regex bulk rename engine
    └── utils/
        ├── human.rs      # Human sizes, relative dates, recency
        └── paths.rs      # Breadcrumbs, tilde, XDG places
```

---

## License

MIT
