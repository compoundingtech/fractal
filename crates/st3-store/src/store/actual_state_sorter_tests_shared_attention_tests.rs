#![allow(unused_imports)]
use crate::store::*;
use std::fs;

pub const SUBJECT: &str = "resource/sorter-alder";

#[cfg(target_os = "linux")]
pub fn written_bytes() -> u64 {
    std::fs::read_to_string("/proc/thread-self/io")
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix("wchar: "))
        .unwrap()
        .parse()
        .unwrap()
}
