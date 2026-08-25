// src/ops/rename.rs
//
// Bulk rename engine with full regex support, counter padding,
// and dry-run preview before applying changes.

use std::path::{Path, PathBuf};

use regex::Regex;

/// A planned rename operation (from → to).
#[derive(Debug, Clone)]
pub struct RenameOp {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Bulk rename configuration.
#[derive(Debug, Clone)]
pub struct BulkRenameConfig {
    /// Regex pattern to match against file names (without directory prefix).
    pub pattern: String,
    /// Replacement string. Supports regex capture groups ($1, $2, etc.)
    /// and the `{n}` counter placeholder (e.g. "photo_{n:03}.jpg").
    pub replacement: String,
    /// Whether the regex is case-insensitive.
    pub case_insensitive: bool,
    /// Whether to apply only to filenames (true) or include extension (false).
    pub name_only: bool,
}

/// Generate a preview list of rename operations without applying them.
/// Returns `Err` if the regex pattern is invalid.
pub fn preview_rename(
    files: &[PathBuf],
    config: &BulkRenameConfig,
) -> anyhow::Result<Vec<RenameOp>> {
    let re = build_regex(&config.pattern, config.case_insensitive)?;
    let mut ops = Vec::new();

    for (counter, path) in files.iter().enumerate() {
        let dir = path.parent().unwrap_or(Path::new(""));
        let filename = path.file_name().unwrap_or_default().to_string_lossy();

        let (subject, ext_part) = if config.name_only {
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let ext = path
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            (stem, ext)
        } else {
            (filename.into_owned(), String::new())
        };

        if !re.is_match(&subject) {
            // No match — skip this file (unchanged)
            continue;
        }

        // Replace counter placeholder {n} / {n:0Nd} before regex substitution
        let replacement_with_counter = apply_counter(&config.replacement, counter + 1);

        // Apply regex replacement
        let new_name = re
            .replace(&subject, replacement_with_counter.as_str())
            .into_owned();

        let new_filename = format!("{}{}", new_name, ext_part);

        if new_filename != filename.as_ref() {
            ops.push(RenameOp {
                from: path.clone(),
                to: dir.join(new_filename),
            });
        }
    }

    Ok(ops)
}

/// Apply a set of RenameOps returned from `preview_rename`.
pub fn apply_rename(ops: &[RenameOp]) -> Vec<anyhow::Error> {
    ops.iter()
        .filter_map(|op| {
            std::fs::rename(&op.from, &op.to)
                .map_err(|e| anyhow::anyhow!("{}: {}", op.from.display(), e))
                .err()
        })
        .collect()
}

fn build_regex(pattern: &str, case_insensitive: bool) -> anyhow::Result<Regex> {
    let mut builder = regex::RegexBuilder::new(pattern);
    builder.case_insensitive(case_insensitive);
    builder.build().map_err(|e| anyhow::anyhow!("Invalid regex: {}", e))
}

/// Replace `{n}` or `{n:0Nd}` (zero-padded counter) in the replacement string.
/// Example: `{n:03}` with counter=5 → `005`
fn apply_counter(replacement: &str, counter: usize) -> String {
    // Match {n} or {n:0Nd} where N is a digit specifying minimum width
    let re = Regex::new(r"\{n(?::0(\d+))?\}").unwrap();
    re.replace_all(replacement, |caps: &regex::Captures| {
        if let Some(width_str) = caps.get(1) {
            let width: usize = width_str.as_str().parse().unwrap_or(1);
            format!("{:0>width$}", counter, width = width)
        } else {
            counter.to_string()
        }
    })
    .into_owned()
}
