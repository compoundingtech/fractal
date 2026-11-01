// LIVE-MIGRATION BRIDGE arn:lmig:smalltalk:2026-11-01-socket-names-fractal — DELETE at contraction
//! Legacy names are symlinks to one listener, preserving peer credentials and authorization.
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, symlink};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use axum::Router;
use fractal_api_core::api as core;

pub struct ReservedUnixSocket {
    reserved: core::ReservedUnixSocket,
    aliases: [Option<SocketAlias>; 2],
}

struct SocketAlias {
    path: PathBuf,
    target: &'static str,
    identity: Option<(u64, u64)>,
    _reservation: File,
}

impl Drop for SocketAlias {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.file_type().is_symlink()
            && self.identity == Some((metadata.dev(), metadata.ino()))
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

async fn reserve_alias(socket: &Path, existing: Option<&SocketAlias>) -> Result<Option<SocketAlias>> {
    let (primary, legacy) = match socket.file_name().and_then(|name| name.to_str()) {
        Some("fractal.sock") => ("fractal.sock", "st3.sock"),
        Some("fractal-client.sock") => ("fractal-client.sock", "st3-client.sock"),
        _ => return Ok(None),
    };
    let parent = socket.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let path = fs::canonicalize(parent)?.join(legacy);
    if existing.is_some_and(|alias| alias.path == path) {
        return Ok(None);
    }
    // Do not canonicalize the alias itself: it may already point at the primary.
    // This lock also fences older daemons while the alias is not yet published.
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".reservation.lock");
    let reservation = OpenOptions::new().read(true).write(true).create(true)
        .truncate(false).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(PathBuf::from(lock_path))?;
    reservation.try_lock().with_context(|| format!("reserve legacy Unix socket {}", path.display()))?;
    match tokio::net::UnixStream::connect(&path).await {
        Ok(_) => anyhow::bail!("refusing to replace live Unix socket listener at {}", path.display()),
        Err(error) if matches!(error.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {}
        Err(error) => return Err(error).with_context(|| format!("probe legacy Unix socket {}", path.display())),
    }
    Ok(Some(SocketAlias { path, target: primary, identity: None, _reservation: reservation }))
}

pub async fn reserve_unix_socket(socket: &Path, state_socket: Option<&Path>) -> Result<ReservedUnixSocket> {
    let bound_socket = crate::config::socket_binding_path(socket);
    let bound_state = state_socket.map(crate::config::socket_binding_path);
    let changed = (bound_socket.as_ref() != socket).then_some((socket, bound_socket.as_ref()))
        .or_else(|| state_socket.zip(bound_state.as_deref()).filter(|(original, bound)| original != bound));
    let socket = bound_socket.as_ref();
    let state_socket = bound_state.as_deref();
    let primary_alias = reserve_alias(socket, None).await?;
    // Old clients discover even a custom control socket through the state/run name.
    let discovery_alias = match state_socket {
        Some(path) => reserve_alias(path, primary_alias.as_ref()).await?,
        None => None,
    };
    let mut aliases = [primary_alias, discovery_alias];
    let reserved = core::reserve_unix_socket(socket, state_socket).await?;
    if let Some((original, bound)) = changed {
        eprintln!("Fractal: socket path {} is {} bytes and exceeds sun_path; serving only legacy name {}",
            original.display(), original.as_os_str().as_bytes().len(), bound.display());
    }
    for alias in aliases.iter_mut().flatten() {
        match fs::symlink_metadata(&alias.path) {
            Ok(metadata) => {
                use std::os::unix::fs::FileTypeExt as _;
                anyhow::ensure!(metadata.file_type().is_socket() || metadata.file_type().is_symlink(),
                    "refusing to replace non-socket legacy path {}", alias.path.display());
                fs::remove_file(&alias.path)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        symlink(alias.target, &alias.path)?;
        let metadata = fs::symlink_metadata(&alias.path)?;
        alias.identity = Some((metadata.dev(), metadata.ino()));
    }
    Ok(ReservedUnixSocket { reserved, aliases })
}

pub async fn serve_reserved_unix_with_ready(
    reserved: ReservedUnixSocket,
    app: Router,
    bound_store: Option<Arc<crate::store::Store>>,
    ready: impl FnOnce(),
) -> Result<()> {
    let ReservedUnixSocket { reserved, aliases: _aliases } = reserved;
    core::serve_reserved_unix_with_ready(reserved, app, bound_store, ready).await
}

pub async fn serve_unix_with_ready(socket: &Path, app: Router, ready: impl FnOnce()) -> Result<()> {
    let reserved = reserve_unix_socket(socket, None).await?;
    serve_reserved_unix_with_ready(reserved, app, None, ready).await
}

pub async fn serve_unix(socket: &Path, app: Router) -> Result<()> {
    serve_unix_with_ready(socket, app, || {}).await
}

pub async fn serve_unix_bound_with_ready(
    socket: &Path, state_socket: &Path, app: Router,
    store: Arc<crate::store::Store>, ready: impl FnOnce(),
) -> Result<()> {
    let reserved = reserve_unix_socket(socket, Some(state_socket)).await?;
    serve_reserved_unix_with_ready(reserved, app, Some(store), ready).await
}

pub async fn serve_unix_bound(
    socket: &Path, state_socket: &Path, app: Router, store: Arc<crate::store::Store>,
) -> Result<()> {
    serve_unix_bound_with_ready(socket, state_socket, app, store, || {}).await
}

#[cfg(test)]
mod tests;
// LIVE-MIGRATION END arn:lmig:smalltalk:2026-11-01-socket-names-fractal
