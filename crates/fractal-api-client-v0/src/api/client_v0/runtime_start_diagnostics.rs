//! Local ledger diagnostics, not a public-agent projection or a repair endpoint.
use super::*;
use crate::store::runtime_start_diagnostics::RuntimeStartDiagnosticsStore as _;

pub(super) async fn list(
    State(state): State<AppState>,
    Extension(snapshot): Extension<ClientSnapshot>,
    Extension(session): Extension<ClientSession>,
    Query(query): Query<ClientListQuery>,
) -> Result<ClientPageResponse, ApiError> {
    require_scope(&session, "read.projections")?;
    client_snapshot_page(&state, snapshot, "runtime-starts", &query, |state, _| {
        state.store.runtime_start_diagnostics()
    })
    .await
}

#[cfg(test)]
mod tests;
