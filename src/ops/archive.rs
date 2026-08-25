// src/ops/archive.rs
//
// Built-in archive compression and extraction.
// Supports: .zip, .tar.gz, .tar.zst, .tar.bz2, .tar.xz

use std::path::{Path, PathBuf};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};

use flume::Sender;

/// Archive format variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
    TarZst,
    TarBz2,
    TarXz,
}

impl ArchiveFormat {
    /// Infer format from file extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_string_lossy().to_lowercase();
        if name.ends_with(".zip") { return Some(Self::Zip); }
        if name.ends_with(".tar.gz") || name.ends_with(".tgz") { return Some(Self::TarGz); }
        if name.ends_with(".tar.zst") || name.ends_with(".tzst") { return Some(Self::TarZst); }
        if name.ends_with(".tar.bz2") || name.ends_with(".tbz2") { return Some(Self::TarBz2); }
        if name.ends_with(".tar.xz") || name.ends_with(".txz") { return Some(Self::TarXz); }
        None
    }
}

/// Extract an archive at `src` into `dest_dir`.
/// Sends progress strings to `progress_tx` as extraction proceeds.
pub fn extract(
    src: &Path,
    dest_dir: &Path,
    format: ArchiveFormat,
    progress_tx: Option<Sender<String>>,
) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest_dir)?;

    let notify = |msg: &str| {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(msg.to_string());
        }
    };

    match format {
        ArchiveFormat::Zip => {
            let file = File::open(src)?;
            let mut archive = zip::ZipArchive::new(BufReader::new(file))?;
            for i in 0..archive.len() {
                let mut entry = archive.by_index(i)?;
                let out_path = dest_dir.join(sanitize_path(entry.name()));
                notify(&format!("Extracting {}", entry.name()));
                if entry.name().ends_with('/') {
                    std::fs::create_dir_all(&out_path)?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    let mut out = BufWriter::new(File::create(&out_path)?);
                    io::copy(&mut entry, &mut out)?;
                }
            }
        }
        ArchiveFormat::TarGz => {
            let file = File::open(src)?;
            let gz = flate2::read::GzDecoder::new(BufReader::new(file));
            let mut archive = tar::Archive::new(gz);
            for entry in archive.entries()? {
                let mut entry = entry?;
                notify(&format!("Extracting {:?}", entry.path()?));
                entry.unpack_in(dest_dir)?;
            }
        }
        ArchiveFormat::TarZst => {
            let file = File::open(src)?;
            let zst = zstd::stream::read::Decoder::new(BufReader::new(file))?;
            let mut archive = tar::Archive::new(zst);
            for entry in archive.entries()? {
                let mut entry = entry?;
                notify(&format!("Extracting {:?}", entry.path()?));
                entry.unpack_in(dest_dir)?;
            }
        }
        ArchiveFormat::TarBz2 => {
            let file = File::open(src)?;
            let bz = bzip2::read::BzDecoder::new(BufReader::new(file));
            let mut archive = tar::Archive::new(bz);
            for entry in archive.entries()? {
                let mut entry = entry?;
                notify(&format!("Extracting {:?}", entry.path()?));
                entry.unpack_in(dest_dir)?;
            }
        }
        ArchiveFormat::TarXz => {
            let file = File::open(src)?;
            let xz = xz2::read::XzDecoder::new(BufReader::new(file));
            let mut archive = tar::Archive::new(xz);
            for entry in archive.entries()? {
                let mut entry = entry?;
                notify(&format!("Extracting {:?}", entry.path()?));
                entry.unpack_in(dest_dir)?;
            }
        }
    }

    Ok(())
}

/// Compress a list of `sources` into `dest` archive using the given format.
pub fn compress(
    sources: Vec<PathBuf>,
    dest: &Path,
    format: ArchiveFormat,
    progress_tx: Option<Sender<String>>,
) -> anyhow::Result<()> {
    let notify = |msg: &str| {
        if let Some(ref tx) = progress_tx {
            let _ = tx.send(msg.to_string());
        }
    };

    match format {
        ArchiveFormat::Zip => {
            let file = BufWriter::new(File::create(dest)?);
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .unix_permissions(0o755);

            for src in &sources {
                add_to_zip(&mut zip, src, src.parent().unwrap_or(src), &options, &notify)?;
            }
            zip.finish()?;
        }
        _ => {
            // For tar-based formats, build tar stream then compress
            let file = BufWriter::new(File::create(dest)?);
            compress_tar(sources, file, format, &notify)?;
        }
    }

    Ok(())
}

fn compress_tar<W: Write>(
    sources: Vec<PathBuf>,
    writer: W,
    format: ArchiveFormat,
    notify: &impl Fn(&str),
) -> anyhow::Result<()> {
    match format {
        ArchiveFormat::TarGz => {
            let enc = flate2::write::GzEncoder::new(writer, flate2::Compression::default());
            build_tar(sources, enc, notify)
        }
        ArchiveFormat::TarZst => {
            let enc = zstd::stream::write::Encoder::new(writer, 3)?.auto_finish();
            build_tar(sources, enc, notify)
        }
        ArchiveFormat::TarBz2 => {
            let enc = bzip2::write::BzEncoder::new(writer, bzip2::Compression::default());
            build_tar(sources, enc, notify)
        }
        ArchiveFormat::TarXz => {
            let enc = xz2::write::XzEncoder::new(writer, 6);
            build_tar(sources, enc, notify)
        }
        _ => unreachable!(),
    }
}

fn build_tar<W: Write>(
    sources: Vec<PathBuf>,
    writer: W,
    notify: &impl Fn(&str),
) -> anyhow::Result<()> {
    let mut builder = tar::Builder::new(writer);
    for src in &sources {
        notify(&format!("Adding {}", src.display()));
        if src.is_dir() {
            builder.append_dir_all(src.file_name().unwrap_or_default(), src)?;
        } else {
            builder.append_path_with_name(src, src.file_name().unwrap_or_default())?;
        }
    }
    builder.finish()?;
    Ok(())
}

fn add_to_zip<W: Write + io::Seek>(
    zip: &mut zip::ZipWriter<W>,
    path: &Path,
    base: &Path,
    options: &zip::write::SimpleFileOptions,
    notify: &impl Fn(&str),
) -> anyhow::Result<()> {
    let name = path.strip_prefix(base).unwrap_or(path);
    if path.is_dir() {
        zip.add_directory(name.to_string_lossy(), *options)?;
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            add_to_zip(zip, &entry.path(), base, options, notify)?;
        }
    } else {
        notify(&format!("Adding {}", name.display()));
        zip.start_file(name.to_string_lossy(), *options)?;
        let mut f = File::open(path)?;
        io::copy(&mut f, zip)?;
    }
    Ok(())
}

/// Sanitize an archive entry name to prevent path traversal attacks.
fn sanitize_path(name: &str) -> PathBuf {
    name.split('/')
        .filter(|c| !c.is_empty() && *c != ".." && *c != ".")
        .collect()
}
