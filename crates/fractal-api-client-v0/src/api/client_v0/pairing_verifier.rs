//! Daemon-local pairing verifiers shared by person and assistant pairing.
//! This belongs beside both pairing handlers, not in bearer credential hashing: low-entropy
//! codes need a secret that is deliberately absent from claims and logical backups.
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use hmac::{Hmac, Mac as _};
use sha2::{Digest as _, Sha256};

pub const MARKER: &str = "hmac-sha256-v1";
pub const CHALLENGE_TTL_MS: u128 = 300_000;

const MAC_DOMAIN: &[u8] = b"fractal/pairing-code-verifier/v1";

pub fn restart() -> super::ApiError {
    super::ApiError {
        status: super::StatusCode::FORBIDDEN,
        code: "pairing-restart-required".into(),
        message: "start pairing again".into(),
        details: Box::default(),
    }
}

/// Legacy verifiers are accepted only within the original bounded challenge window.
pub fn legacy_window_open(expires: u128, accepted_at: u128, now: u128) -> bool {
    expires > now && expires.checked_sub(accepted_at)
        .is_some_and(|lifetime| lifetime <= CHALLENGE_TTL_MS)
}

/// Load or durably create the daemon's 32-byte secret. All creators serialize on a
/// separate lock inode; the final key is published only after its contents are synced.
/// Never export this directory in logical backups or put the key into a claim.
pub fn load_key(state_dir: &Path) -> Result<[u8; 32]> {
    fs::create_dir_all(state_dir)?;
    let directory = state_dir.join("pairing-verifier");
    match fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => (),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(&directory)?;
    ensure!(metadata.is_dir() && metadata.mode() & 0o777 == 0o700
        && metadata.uid() == unsafe { libc::geteuid() },
        "pairing verifier directory must be daemon-owned and 0700");
    let lock = OpenOptions::new().read(true).write(true).create(true)
        .truncate(false).mode(0o600).custom_flags(libc::O_NOFOLLOW)
        .open(directory.join("lock"))?;
    lock.lock().context("lock pairing verifier creation")?;
    let path = directory.join("key");
    match OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW).open(&path) {
        Ok(mut file) => {
            let metadata = file.metadata()?;
            ensure!(metadata.is_file() && metadata.mode() & 0o777 == 0o600
                && metadata.uid() == unsafe { libc::geteuid() },
                "pairing verifier key must be a daemon-owned 0600 regular file");
            let mut key = [0_u8; 32];
            file.read_exact(&mut key)?;
            ensure!(file.read(&mut [0_u8; 1])? == 0, "invalid pairing verifier key length");
            Ok(key)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut key = [0_u8; 32];
            getrandom::fill(&mut key).map_err(|error| anyhow::anyhow!("{error}"))?;
            let mut staged = tempfile::Builder::new().prefix(".key-").tempfile_in(&directory)?;
            staged.write_all(&key)?;
            staged.as_file().sync_all()?;
            staged.persist_noclobber(&path)?;
            File::open(&directory)?.sync_all()?;
            File::open(state_dir)?.sync_all()?;
            Ok(key)
        }
        Err(error) => Err(error.into()),
    }
}

/// A non-secret key identifier distinguishes a replaced secret from a mistyped code.
/// It depends only on the random secret, never on the pairing code.
pub fn key_id(key: &[u8; 32]) -> String {
    hex::encode(Sha256::digest(key))
}

/// Compute HMAC-SHA256 for a low-entropy pairing code; not for bearer lookup indexes.
/// Input: domain, full prefixed pairing or assistant grant ID UTF-8 bytes, then code UTF-8 bytes,
/// each prefixed by its byte length as big-endian u32. No normalization is applied.
pub fn digest(key: &[u8; 32], pairing_id: &str, code: &str) -> String {
    let mac = pairing_mac(key, pairing_id, code);
    hex::encode(mac.finalize().into_bytes())
}

/// Verify the stored HMAC in constant time (malformed encodings are refused).
pub fn verify(key: &[u8; 32], pairing_id: &str, code: &str, expected: &str) -> bool {
    let mut decoded = [0_u8; 32];
    if hex::decode_to_slice(expected, &mut decoded).is_err() {
        return false;
    }
    pairing_mac(key, pairing_id, code).verify_slice(&decoded).is_ok()
}

fn pairing_mac(key: &[u8; 32], pairing_id: &str, code: &str) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts a 32-byte key");
    for field in [MAC_DOMAIN, pairing_id.as_bytes(), code.as_bytes()] {
        let length = u32::try_from(field.len()).expect("pairing MAC field fits in u32");
        mac.update(&length.to_be_bytes());
        mac.update(field);
    }
    mac
}

/// Random 128-bit pairing identity, independent of the device name and code.
pub fn random_id() -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| anyhow::anyhow!("{error}"))?;
    Ok(hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_window_requires_an_unexpired_bounded_lifetime() {
        assert!(legacy_window_open(300_001, 1, 2));
        assert!(!legacy_window_open(300_002, 1, 2));
        assert!(!legacy_window_open(300_001, 1, 300_001));
        assert!(!legacy_window_open(300_001, 1, 300_002));
        assert!(!legacy_window_open(1, 2, 0));
    }

    #[test]
    fn keyed_codes_and_random_identity() {
        let key = [42; 32];
        let pairing_id = "pairing/000102030405060708090a0b0c0d0e0f";
        let verifier = digest(&key, pairing_id, "ABCDEFGH");
        assert!(verify(&key, pairing_id, "ABCDEFGH", &verifier));
        assert!(!verify(&key, pairing_id, "ABCDEFGJ", &verifier));
        assert!(!verify(&[43; 32], pairing_id, "ABCDEFGH", &verifier));
        assert_ne!(random_id().unwrap(), random_id().unwrap());
    }

    #[test]
    fn verifier_is_bound_to_the_full_pairing_identity() {
        let key = [42; 32];
        let verifier = digest(&key, "pairing/abc", "ABCDEFGH");
        assert!(!verify(&key, "pairing/def", "ABCDEFGH", &verifier));
        assert!(!verify(&key, "assistant-grant/abc", "ABCDEFGH", &verifier));
        assert_ne!(digest(&key, "pairing/a", "bc"), digest(&key, "pairing/ab", "c"));
    }

    #[test]
    fn domain_separated_verifier_known_answer() {
        // HMAC-SHA256 over domain, full pairing ID and code bytes, each prefixed
        // by its u32 big-endian byte length (32, 40 and 8 respectively).
        assert_eq!(
            digest(&[42; 32], "pairing/000102030405060708090a0b0c0d0e0f", "ABCDEFGH"),
            "73c6aa76a74f9f2d85a1fcaf5aecc16212c2ff1d4f5e19922a563d41abb1bc7e",
        );
    }

    #[test]
    fn concurrent_creation_publishes_one_private_secret() {
        let root = tempfile::tempdir().unwrap();
        let barrier = std::sync::Barrier::new(8);
        let keys = std::thread::scope(|scope| {
            let threads: Vec<_> = (0..8).map(|_| scope.spawn(|| {
                barrier.wait();
                load_key(root.path()).unwrap()
            })).collect();
            threads.into_iter().map(|thread| thread.join().unwrap()).collect::<Vec<_>>()
        });
        assert!(keys.iter().all(|key| key == &keys[0]));
        assert_eq!(fs::metadata(root.path().join("pairing-verifier")).unwrap().mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(root.path().join("pairing-verifier/key")).unwrap().mode() & 0o777, 0o600);
        assert_eq!(load_key(root.path()).unwrap(), keys[0]);
    }
}
