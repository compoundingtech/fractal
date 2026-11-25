//! Real Home/Store protocol contracts share one canonical core crate identity.
use std::{path::Path, sync::Arc};

use fractal_light_node_core::session::{RunEffects, contract_tests};
use fractal_store::{light_node::StoreRunEffects, store::Store};

fn graph_effects(path: &Path, home: &str) -> Arc<dyn RunEffects> {
    Arc::new(StoreRunEffects::new(Arc::new(Store::open(path, home.to_owned()).unwrap())))
}

macro_rules! protocol_contracts {
    ($($name:ident),+ $(,)?) => {$(
        #[tokio::test]
        async fn $name() { contract_tests::$name(graph_effects).await; }
    )+};
}

protocol_contracts! {
    session_observer_counts_actual_authenticated_io_only,
    pinned_signers_are_required_even_with_the_shared_fleet_secret,
    reconnect_recovers_lost_receipt_ack_without_readmitting_or_reidentifying_work,
    unknown_versions_and_stale_home_epochs_fail_before_session_admission,
    paired_epoch_rollover_delivers_terminal_custody_before_retired_only_resume,
    requester_retains_original_receipt_for_freshly_signed_retries_across_restart,
    copied_pinned_key_without_member_proof_is_refused_at_both_handshake_verifiers,
    captured_upgrade_and_frame_cannot_authorize_a_new_socket_or_modified_payload,
    live_channel_rejects_duplicate_signed_sequence_and_unsigned_authority,
    channel_proofs_bind_direction_nonce_request_and_frame_payload,
    fleet_secret_and_copied_member_key_cannot_forge_a_live_authority_frame,
    replayed_http_proof_without_fresh_channel_proof_cannot_reserve_the_seat,
    finalization_retry_requires_real_progress_and_never_retries_unknown_custody,
}

#[test]
fn protocol_frames_are_bounded_and_unknown_shapes_are_not_accepted() {
    contract_tests::protocol_frames_are_bounded_and_unknown_shapes_are_not_accepted();
}
