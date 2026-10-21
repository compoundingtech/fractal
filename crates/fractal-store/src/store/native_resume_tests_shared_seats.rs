#![allow(unused_imports)]
use crate::native_resume::*;
use std::fs;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use sha2::{Digest as _, Sha256};
use std::os::unix::fs::{MetadataExt as _, symlink};
use std::os::fd::AsRawFd as _;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt as _;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;

pub fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}
