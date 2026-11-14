//! Read-only catalog generation tokens shared by native readers and authoring fences.

use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const CONTROL_DIR: &str = ".st2";
pub const GENERATION_FILE: &str = "catalog-generation";

pub(crate) fn read_generation(catalog: &Path) -> Result<Option<u64>> {
    let Some((control, _control_path)) = retained_control(catalog)? else {
        return Ok(None);
    };
    read_generation_from_control(&control)
}

pub(crate) fn read_generation_token(catalog: &Path) -> Result<Option<u64>> {
    read_generation(catalog)
}

pub(crate) fn read_generation_from_control(control: &File) -> Result<Option<u64>> {
    let control_path = crate::directory::retained_dir_path(control)?;
    let path = control_path.join(GENERATION_FILE);
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("open catalog generation"),
    };
    let metadata = file.metadata()?;
    anyhow::ensure!(
        metadata.is_file(),
        "catalog generation is not a real regular file: {}",
        path.display()
    );
    use std::io::Read as _;
    let mut value = String::new();
    file.read_to_string(&mut value)?;
    let value = value
        .strip_suffix('\n')
        .context("catalog generation is missing its newline terminator")?;
    Ok(Some(value.parse().context("parse catalog generation")?))
}

pub(crate) fn retained_control(catalog: &Path) -> Result<Option<(File, PathBuf)>> {
    let root = crate::directory::open_dir_beneath(catalog, catalog)?;
    let file = match crate::directory::openat_dir_nofollow(
        &root,
        std::ffi::OsStr::new(CONTROL_DIR),
    ) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("open catalog control directory"),
    };
    let path = crate::directory::retained_dir_path(&file)?;
    Ok(Some((file, path)))
}
