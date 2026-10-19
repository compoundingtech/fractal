//! The part of `backup` that moved up from st3-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use crate::client::Endpoint;
use crate::store::{CheckpointManifest, Store};
use crate::message_body::BodyAttachment as _;
pub use crate::store::backup_format::{
    FORMAT, Header, MessageBodyRecord, Record, RestoreReport, TableDigest, VERSION,
    VERSION_WITH_BODIES,
};
pub use crate::store::backup_format::{MAX_RECORD_BYTES, PAGE};
pub use st3_store::backup::*;
pub use st3_store::backup::{create_from_database, restore, verify_file};

/// Download to a private temporary file, then publish only a complete, checksummed archive.
pub async fn create(endpoint: &Endpoint, destination: &Path) -> Result<()> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    ensure!(!destination.exists(), "backup file already exists");
    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = tempfile::NamedTempFile::new_in(directory)?;
    let mut output = tokio::fs::File::from_std(temporary.reopen()?);
    let (client, url) = match endpoint {
        Endpoint::Unix(socket) => (
            reqwest::Client::builder()
                .unix_socket(socket.as_path())
                .build()?,
            "http://localhost/v1/backup".to_owned(),
        ),
        Endpoint::Http(base) => (reqwest::Client::new(), format!("{base}/v1/backup")),
    };
    let response = client.get(url).send().await?;
    let status = response.status();
    ensure!(
        status.is_success(),
        "backup export failed ({status}): {}",
        response.text().await?
    );
    let mut stream = response.bytes_stream();
    while let Some(bytes) = stream.next().await {
        output.write_all(&bytes?).await?;
    }
    output.sync_all().await?;
    drop(output);
    let temporary = tokio::task::spawn_blocking(move || -> Result<_> {
        verify_file(temporary.path())?;
        Ok(temporary)
    })
    .await??;
    temporary.persist_noclobber(destination)?;
    Ok(())
}
