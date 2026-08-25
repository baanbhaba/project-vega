// src/ops/checksum.rs
//
// Fast multi-algorithm checksum computation.
// BLAKE3 is preferred (parallelized internally, orders of magnitude faster than MD5/SHA256).

use std::io::{self, Read};
use std::path::Path;

use blake3::Hasher as Blake3Hasher;
use md5::{Digest as _, Md5};
use sha2::{Digest as _, Sha256};

/// Supported checksum algorithms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumAlg {
    Blake3,
    Sha256,
    Md5,
}

/// Compute the checksum of a file at `path` using the specified algorithm.
/// Returns the hex-encoded digest string.
pub fn checksum_file(path: &Path, alg: ChecksumAlg) -> anyhow::Result<String> {
    let mut file = std::fs::File::open(path)?;
    match alg {
        ChecksumAlg::Blake3 => {
            let mut hasher = Blake3Hasher::new();
            // BLAKE3 can use memory mapping for large files internally
            let mut buf = vec![0u8; 1024 * 1024]; // 1 MB buffer
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 { break; }
                hasher.update(&buf[..n]);
            }
            Ok(hasher.finalize().to_hex().to_string())
        }
        ChecksumAlg::Sha256 => {
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 1024 * 1024];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 { break; }
                hasher.update(&buf[..n]);
            }
            Ok(hex::encode(hasher.finalize()))
        }
        ChecksumAlg::Md5 => {
            let mut hasher = Md5::new();
            let mut buf = vec![0u8; 1024 * 1024];
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 { break; }
                hasher.update(&buf[..n]);
            }
            Ok(hex::encode(hasher.finalize()))
        }
    }
}

/// Compare two files by computing and comparing their BLAKE3 hashes.
/// More reliable than size+mtime comparison for exact equality.
pub fn files_are_equal(a: &Path, b: &Path) -> anyhow::Result<bool> {
    // Quick size pre-check
    let size_a = std::fs::metadata(a)?.len();
    let size_b = std::fs::metadata(b)?.len();
    if size_a != size_b {
        return Ok(false);
    }
    let ha = checksum_file(a, ChecksumAlg::Blake3)?;
    let hb = checksum_file(b, ChecksumAlg::Blake3)?;
    Ok(ha == hb)
}
