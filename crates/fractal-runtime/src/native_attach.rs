//! Native attach is observation, never process ownership. Keep the witness alive through commit.
//! Challenges travel only over authenticated local IPC, never argv, environment or transcripts.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessEvidence {
    pub pid: u32,
    pub uid: u32,
    pub start_token: u64,
    pub boot_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptEvidence {
    pub path: PathBuf,
    pub device: u64,
    pub inode: u64,
    pub uid: u32,
    pub native_id: String,
    pub header_cwd: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachEvidence {
    pub process: ProcessEvidence,
    pub transcript: TranscriptEvidence,
    pub observed_cwd: Option<PathBuf>,
}

/// Secret bearer challenge; deliberately does not implement Debug.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofChallenge {
    pub socket_path: PathBuf,
    pub nonce: String,
    pub daemon: ProcessEvidence,
    /// Unix epoch milliseconds; checked in addition to the listener's monotonic deadline.
    pub expiry: u64,
}

/// One-shot native-process report. Never log or render this secret-bearing input.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NonceReport {
    pub nonce: String,
    pub evidence: AttachEvidence,
    /// Exact challenge expiry in Unix epoch milliseconds.
    pub expiry: u64,
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub use unix::{AttachWitness, ProofListener, inspect, report};

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("native attach requires Linux or macOS kernel process and Unix peer witnesses");
