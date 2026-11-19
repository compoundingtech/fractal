//! Allocated-block high-water for engine-declared exclusive output roots. Shared stores,
//! foreign filesystems, symlink roots and multiply-linked files cannot certify attribution.
//! Host statfs remains the authority for live floors, including open-unlinked files.
use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutputRoot {
    pub path: PathBuf,
    pub inode: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExclusiveRoots {
    pub roots: Vec<OutputRoot>,
    pub filesystem: u64,
    pub baseline_bytes: u64,
    pub high_water_growth_bytes: u64,
}

impl ExclusiveRoots {
    /// The caller must own these roots exclusively for this execution generation. Paths are
    /// engine policy, never inferred from cwd or an untrusted payload declaration.
    pub fn begin(paths: &[PathBuf], filesystem: u64) -> Result<Self> {
        ensure!(!paths.is_empty() && paths.len() <= 16,
            "disk measurement requires one to sixteen exclusive output roots");
        let mut canonical = Vec::with_capacity(paths.len());
        for path in paths {
            ensure!(path.is_absolute(), "exclusive disk root must be absolute");
            let metadata = std::fs::symlink_metadata(path)?;
            ensure!(metadata.is_dir() && !metadata.file_type().is_symlink(),
                "exclusive disk root must be a real directory");
            let path = path.canonicalize()?;
            ensure!(!canonical.iter().any(|existing: &OutputRoot|
                existing.path.starts_with(&path) || path.starts_with(&existing.path)),
                "exclusive disk roots must not overlap");
            canonical.push(OutputRoot { inode: std::fs::symlink_metadata(&path)?.ino(), path });
        }
        let mut roots = Self { roots: canonical, filesystem,
            baseline_bytes: 0, high_water_growth_bytes: 0 };
        roots.baseline_bytes = roots.allocated_bytes()?;
        Ok(roots)
    }

    pub fn observe(&mut self) -> Result<u64> {
        let allocated = self.allocated_bytes()?;
        self.high_water_growth_bytes = self.high_water_growth_bytes
            .max(allocated.saturating_sub(self.baseline_bytes));
        Ok(self.high_water_growth_bytes)
    }

    fn allocated_bytes(&self) -> Result<u64> {
        let mut total = 0_u64;
        for root in &self.roots {
            // A replaced root is not an observation of the originally declared output tree.
            let metadata = std::fs::symlink_metadata(&root.path)?;
            ensure!(metadata.ino() == root.inode && metadata.is_dir()
                && !metadata.file_type().is_symlink(),
                "exclusive disk root was replaced");
            for entry in walkdir::WalkDir::new(&root.path).follow_links(false) {
                let entry = entry.context("traversing exclusive disk outputs")?;
                let metadata = std::fs::symlink_metadata(entry.path())?;
                ensure!(metadata.dev() == self.filesystem,
                    "exclusive disk outputs crossed their admitted filesystem");
                ensure!(metadata.is_dir() || metadata.nlink() == 1,
                    "multiply-linked disk outputs do not prove exclusive allocation");
                total = metadata.blocks().checked_mul(512)
                    .and_then(|bytes| total.checked_add(bytes))
                    .context("exclusive disk allocated bytes overflow")?;
            }
        }
        Ok(total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_allocated_blocks_and_preserves_high_water_after_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let filesystem = directory.path().metadata().unwrap().dev();
        let mut measured = ExclusiveRoots::begin(&[directory.path().into()], filesystem).unwrap();
        let file = directory.path().join("output");
        std::fs::write(&file, vec![1_u8; 128 * 1024]).unwrap();
        let growth = measured.observe().unwrap();
        let allocated = file.metadata().unwrap().blocks() * 512;
        assert!(allocated > 0);
        assert!(growth >= allocated);
        std::fs::remove_file(file).unwrap();
        assert_eq!(measured.observe().unwrap(), growth);
        let serialized = serde_json::to_string(&measured).unwrap();
        let mut reopened: ExclusiveRoots = serde_json::from_str(&serialized).unwrap();
        assert_eq!(reopened.observe().unwrap(), growth);
    }

    #[test]
    fn never_counts_a_symlink_target_or_traverses_a_symlink_root() {
        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let filesystem = directory.path().metadata().unwrap().dev();
        std::fs::write(outside.path().join("unowned"), vec![1_u8; 128 * 1024]).unwrap();
        let link = directory.path().join("link");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert!(ExclusiveRoots::begin(&[link], filesystem).is_err());
        let mut measured = ExclusiveRoots::begin(&[directory.path().into()], filesystem).unwrap();
        assert_eq!(measured.observe().unwrap(), 0);
    }

    #[test]
    fn shared_hardlinks_and_overlapping_roots_are_not_certified() {
        let directory = tempfile::tempdir().unwrap();
        let filesystem = directory.path().metadata().unwrap().dev();
        let nested = directory.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        assert!(ExclusiveRoots::begin(&[directory.path().into(), nested.clone()], filesystem).is_err());
        let source = directory.path().join("output");
        std::fs::write(&source, "data").unwrap();
        std::fs::hard_link(source, nested.join("alias")).unwrap();
        assert!(ExclusiveRoots::begin(&[directory.path().into()], filesystem).is_err());
        assert!(ExclusiveRoots::begin(&[nested], filesystem.wrapping_add(1)).is_err());
    }

    #[test]
    fn sparse_logical_length_is_not_charged_as_allocated_growth() {
        let directory = tempfile::tempdir().unwrap();
        let filesystem = directory.path().metadata().unwrap().dev();
        let mut measured = ExclusiveRoots::begin(&[directory.path().into()], filesystem).unwrap();
        std::fs::File::create(directory.path().join("sparse")).unwrap()
            .set_len(64 * 1024 * 1024).unwrap();
        assert!(measured.observe().unwrap() < 64 * 1024 * 1024);
    }

    #[test]
    fn directory_replacement_cannot_reuse_the_old_allocation_baseline() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("outputs");
        std::fs::create_dir(&root).unwrap();
        let filesystem = root.metadata().unwrap().dev();
        let mut measured = ExclusiveRoots::begin(std::slice::from_ref(&root), filesystem).unwrap();
        std::fs::rename(&root, directory.path().join("previous-outputs")).unwrap();
        std::fs::create_dir(root).unwrap();
        assert!(measured.observe().is_err());
    }
}
