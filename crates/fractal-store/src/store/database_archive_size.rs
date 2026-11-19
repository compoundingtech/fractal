//! Separate local wire-archive capacity accounting. This never creates an archive or moves data.
//! File lengths are not content-integrity/catalogue-coverage proof, nor physical allocated extents.
use super::*;
use std::collections::HashMap;
use std::fs;
use serde::{Deserialize, Serialize};
use crate::slo::{ArchiveBytes, ArchiveSizeStatus, DatabaseArchiveSize};

const SAMPLES: &str = "wire_archive_size_samples/v1";
const HOUR: u128 = 60 * 60 * 1000;
const DAY: u128 = 24 * HOUR;
const MAX_ENTRIES: usize = 1_000_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Sample { at: u128, scope: String, bytes: Option<u64> }

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Category { Segment, Index, Overlay, Staging, Orphan, Backup }

fn add(total: &mut u64, bytes: u64) -> Result<()> {
    *total = total.checked_add(bytes).context("archive capacity overflow")?;
    Ok(())
}

fn totals(bytes: ArchiveBytes) -> Result<(u64, u64)> {
    let mut archive = bytes.segment_file_bytes;
    add(&mut archive, bytes.index_bytes)?;
    add(&mut archive, bytes.overlay_bytes)?;
    let mut capacity = archive;
    for value in [bytes.staging_bytes, bytes.orphan_bytes, bytes.backup_bytes] { add(&mut capacity, value)?; }
    Ok((archive, capacity))
}

fn segment_category(path: &Path) -> Category {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap_or("");
    if name.ends_with(".wire-segment") { return Category::Staging; }
    if let Some(hash) = name.strip_suffix(".wire")
        && hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        Category::Segment
    } else { Category::Orphan }
}

/// Refuse symlinks, unknown root categories and incomplete stat/list results. Hardlinks count once.
fn measure(root: &Path) -> Result<Option<ArchiveBytes>> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(metadata.is_dir() && !metadata.file_type().is_symlink(), "archive root is not a real directory");
    let mut pending = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let category = match entry.file_name().to_str() {
            Some("segments") => Category::Segment, Some("index") => Category::Index,
            Some("overlays") => Category::Overlay, Some("staging") => Category::Staging,
            Some("orphans") => Category::Orphan, Some("backups") => Category::Backup,
            _ => anyhow::bail!("unrecognized archive capacity category"),
        };
        anyhow::ensure!(entry.file_type()?.is_dir(), "archive category is not a real directory");
        pending.push((entry.path(), category));
    }
    let mut files = HashMap::new();
    let mut visited = 0_usize;
    while let Some((directory, category)) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            visited += 1;
            anyhow::ensure!(visited <= MAX_ENTRIES, "archive capacity inventory exceeds entry limit");
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            anyhow::ensure!(!metadata.file_type().is_symlink(), "archive capacity inventory refuses symlinks");
            if metadata.is_dir() { pending.push((entry.path(), category)); continue; }
            anyhow::ensure!(metadata.is_file(), "archive capacity inventory contains a special file");
            let category = if category == Category::Segment { segment_category(&entry.path()) } else { category };
            #[cfg(unix)]
            let identity = {
                use std::os::unix::fs::MetadataExt;
                (metadata.dev(), metadata.ino())
            };
            #[cfg(not(unix))]
            let identity = anyhow::bail!("archive hardlink accounting is unsupported on this platform");
            let length = metadata.len();
            if let Some((previous_category, previous_length)) = files.get_mut(&identity) {
                anyhow::ensure!(*previous_length == length, "archive file changed during inventory");
                *previous_category = std::cmp::min(*previous_category, category);
            } else { files.insert(identity, (category, length)); }
        }
    }
    let mut result = ArchiveBytes { files: files.len() as u64, ..Default::default() };
    for (category, length) in files.into_values() {
        add(match category {
            Category::Segment => &mut result.segment_file_bytes, Category::Index => &mut result.index_bytes,
            Category::Overlay => &mut result.overlay_bytes, Category::Staging => &mut result.staging_bytes,
            Category::Orphan => &mut result.orphan_bytes, Category::Backup => &mut result.backup_bytes,
        }, length)?;
    }
    Ok(Some(result))
}

fn growth(samples: &[Sample], now: u128, scope: &str) -> Option<(i64, u128, u128)> {
    let day_ago = now.checked_sub(DAY)?;
    let (index, baseline) = samples.iter().enumerate()
        .filter(|(_, sample)| sample.at.abs_diff(day_ago) <= HOUR / 2 && sample.at < now && sample.scope == scope && sample.bytes.is_some())
        .min_by_key(|(_, sample)| (sample.at.abs_diff(day_ago), sample.at))?;
    let covered = &samples[index..];
    if covered.iter().any(|sample| sample.scope != scope || sample.bytes.is_none())
        // A lost hourly write must break coverage even when its unknown sample could not persist.
        || covered.windows(2).any(|pair| pair[1].at <= pair[0].at || pair[1].at - pair[0].at > 90 * 60 * 1000) { return None; }
    let current = covered.last()?;
    if current.at != now { return None; }
    let span = now.checked_sub(baseline.at)?;
    let delta = i128::from(current.bytes?) - i128::from(baseline.bytes?);
    Some((i64::try_from(delta * DAY as i128 / i128::try_from(span).ok()?).ok()?, span, baseline.at))
}

impl Store {
    /// Sample the dedicated `wire-archive` directory beside this graph. Its absence is not zero.
    /// This read-only sampler does not activate archive writers, create directories or remove files.
    pub fn record_database_archive_size(&self, now: u128) -> DatabaseArchiveSize {
        let parent = self.path.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or(Path::new("."));
        let root = fs::canonicalize(parent).map(|parent| parent.join("wire-archive"));
        let scope = root.as_ref().map(|root| hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()))).unwrap_or_default();
        match root.map_err(anyhow::Error::from).and_then(|root| self.archive_size_at(&root, now, &scope)) {
            Ok(size) => size,
            Err(error) => DatabaseArchiveSize { measured_at_unix_ms: now, scope, status: ArchiveSizeStatus::Unavailable,
                bytes: None, archive_bytes: None, capacity_bytes: None, growth_bytes_per_day: None,
                growth_span_ms: None, baseline_at_unix_ms: None, error: Some(error.to_string()) },
        }
    }

    fn archive_size_at(&self, root: &Path, now: u128, scope: &str) -> Result<DatabaseArchiveSize> {
        let previous: Option<String> = self.readers.get().query_row("SELECT value FROM meta WHERE key=?1", [SAMPLES], |row| row.get(0)).optional()?;
        let mut samples: Vec<Sample> = previous.map(|text| serde_json::from_str(&text)).transpose()?.unwrap_or_default();
        // A copied/moved graph carries meta history, not activation of its new archive scope.
        let known_scope = samples.iter().any(|sample| sample.scope == scope);
        // A moved scope or clock rollback starts a fresh derived observation series.
        if samples.iter().any(|sample| sample.at > now || sample.scope != scope) { samples.clear(); }
        samples.retain(|sample| now - sample.at <= 49 * HOUR);
        let measured = measure(root);
        let (status, bytes, error) = match measured {
            Ok(Some(bytes)) => (ArchiveSizeStatus::Available, Some(bytes), None),
            Ok(None) if !known_scope => (ArchiveSizeStatus::NotActivated, None, None),
            Ok(None) => (ArchiveSizeStatus::Unavailable, None, Some("previously measured archive root is missing".into())),
            Err(error) => (ArchiveSizeStatus::Unavailable, None, Some(error.to_string())),
        };
        let totals = bytes.map(totals).transpose()?;
        let archive_bytes = totals.map(|(archive, _)| archive);
        if status != ArchiveSizeStatus::NotActivated {
            samples.push(Sample { at: now, scope: scope.into(), bytes: archive_bytes });
            let excess = samples.len().saturating_sub(128);
            samples.drain(..excess);
            self.connection.write().execute(
                /* sqlite-large-value: allow-for-now lv-h0197f4bfefef owner=fractal_store::store::database_archive_size TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![SAMPLES, serde_json::to_string(&samples)?],
            )?;
        }
        let growth = growth(&samples, now, scope);
        Ok(DatabaseArchiveSize { measured_at_unix_ms: now, scope: scope.into(), status, bytes, archive_bytes,
            capacity_bytes: totals.map(|(_, capacity)| capacity), growth_bytes_per_day: growth.map(|(rate, _, _)| rate),
            growth_span_ms: growth.map(|(_, span, _)| span), baseline_at_unix_ms: growth.map(|(_, _, at)| at), error })
    }
}

#[cfg(test)]
mod tests;
