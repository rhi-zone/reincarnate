//! Content-addressed cache for emit and check results.
//!
//! **Emit cache** — keyed by sha256(binary || manifest || source || pipeline_config).
//! If the key matches a stored entry and the output directories still exist, the
//! full pipeline is skipped and the cached result is returned.
//!
//! **Check cache** — keyed by sha256 of all files in the output directory.
//! If the key matches a stored entry, the TypeScript checker is skipped.
//!
//! Both caches are stored under `~/.cache/reincarnate/` as JSON files.  All
//! operations are best-effort: any I/O error silently falls back to a cache
//! miss so that the normal code path runs.

use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use reincarnate_core::pipeline::{CheckerOutput, Diagnostic};

// ── Cache directory ───────────────────────────────────────────────────────────

fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("reincarnate"))
}

fn cache_path(kind: &str, key: &str) -> Option<PathBuf> {
    Some(cache_dir()?.join(format!("{kind}-{key}.json")))
}

// ── Low-level hashing ─────────────────────────────────────────────────────────

/// SHA-256 a single file by streaming it in 64 KiB chunks.
/// Returns lowercase hex, or `None` if the file cannot be opened.
fn sha256_file(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Some(format!("{:x}", hasher.finalize()))
}

/// SHA-256 all files under `dir`, sorted by relative path.
/// Returns lowercase hex, or an error if the directory cannot be walked.
fn sha256_dir_sorted(dir: &Path) -> anyhow::Result<String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_files(dir, &mut paths)?;
    paths.sort();

    let mut hasher = Sha256::new();
    for path in &paths {
        // Hash the relative path so renames are detected.
        let rel = path.strip_prefix(dir).unwrap_or(path);
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        // Hash the file contents.
        let mut file = std::fs::File::open(path)?;
        let mut buf = [0u8; 65536];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        hasher.update(b"\0");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out)?;
        } else {
            out.push(path);
        }
    }
    Ok(())
}

// ── Cache keys ────────────────────────────────────────────────────────────────

/// Compute the emit cache key for a given manifest.
///
/// Key components (each individually sha256'd, then concatenated as 192-char hex):
/// - Binary: `std::env::current_exe()` sha256, or sentinel `"dev"` on failure.
/// - Manifest file sha256.
/// - Source file sha256.
/// - Pipeline config string (preset + sorted skip-passes + fixpoint flag).
pub fn emit_cache_key(
    manifest_path: &Path,
    source_path: &Path,
    preset: &str,
    skip_passes: &[String],
    fixpoint: bool,
) -> String {
    let binary_hash = std::env::current_exe()
        .ok()
        .and_then(|p| sha256_file(&p))
        .unwrap_or_else(|| "dev".repeat(16)); // 48 chars, never matches a real hash

    let manifest_hash = sha256_file(manifest_path).unwrap_or_else(|| "dev".repeat(16));

    let source_hash = sha256_file(source_path).unwrap_or_else(|| "dev".repeat(16));

    let mut sorted_passes = skip_passes.to_vec();
    sorted_passes.sort();
    let config_str = format!("{preset}|{}|{fixpoint}", sorted_passes.join(","));
    let config_hash = {
        let mut h = Sha256::new();
        h.update(config_str.as_bytes());
        format!("{:x}", h.finalize())
    };

    // Final key: feed all four 64-char hashes into a single sha256.
    let mut h = Sha256::new();
    h.update(binary_hash.as_bytes());
    h.update(manifest_hash.as_bytes());
    h.update(source_hash.as_bytes());
    h.update(config_hash.as_bytes());
    format!("{:x}", h.finalize())
}

/// Compute the check cache key from the output directory contents.
/// Returns an error if the directory cannot be walked (caller treats as miss).
pub fn check_cache_key(output_dir: &Path) -> anyhow::Result<String> {
    sha256_dir_sorted(output_dir)
}

// ── Cache entry types ─────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize)]
pub struct EmitCacheEntry {
    pub key: String,
    pub output_dirs: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
    pub created_at: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct CheckCacheEntry {
    pub key: String,
    pub checker_output: CheckerOutput,
    pub created_at: String,
}

// ── Read / write helpers ──────────────────────────────────────────────────────

fn try_read_cache<T: for<'de> serde::Deserialize<'de>>(path: &Path) -> Option<T> {
    let data = std::fs::read(path).ok()?;
    serde_json::from_slice(&data).ok()
}

fn try_write_cache<T: serde::Serialize>(path: &Path, value: &T) {
    let Ok(data) = serde_json::to_vec(value) else {
        return;
    };
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, &data).is_err() {
        return;
    }
    let _ = std::fs::rename(&tmp, path);
}

// ── Public API ────────────────────────────────────────────────────────────────

pub fn lookup_emit_cache(key: &str) -> Option<EmitCacheEntry> {
    try_read_cache(&cache_path("emit", key)?)
}

pub fn store_emit_cache(key: &str, entry: &EmitCacheEntry) {
    if let Some(path) = cache_path("emit", key) {
        try_write_cache(&path, entry);
    }
}

pub fn lookup_check_cache(key: &str) -> Option<CheckCacheEntry> {
    try_read_cache(&cache_path("check", key)?)
}

pub fn store_check_cache(key: &str, entry: &CheckCacheEntry) {
    if let Some(path) = cache_path("check", key) {
        try_write_cache(&path, entry);
    }
}
