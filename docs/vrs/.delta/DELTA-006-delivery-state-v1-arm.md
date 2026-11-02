# DELTA-006: retained native delivery-state translation

Status: open

The canonical ledger owns `st.delivery-ledger.v1`.
`crates/fractal-drivers/src/delivery_ledger.rs` calls
`migrations::delivery_state::recover` only from `Ledger::open`'s missing-ledger arm.
The translation reads a pre-ledger `delivery-state.json` and carries its in-flight
attempt as asserted evidence. It authorizes no resend or stronger observed receipt.
This remains reachable from Fractal's native Codex and OpenCode drivers after the
catalog CLI retires.

The live requirement is [native delivery ownership](../../st3/invariants.md):
ambiguity about a staged attempt must hold for inspection rather than duplicate
transport. The translator remains necessary while an older record or an asserted
entry can still carry that evidence.

## Deletion prerequisites

For **every admitted host**, retain read-only observations showing both counts at
zero for **seven consecutive days**:

1. No pre-ledger `delivery-state.json` beside any relevant Codex/OpenCode state directory.
2. No ledger entry carrying an `Asserted` phase.

`migrations::delivery_state::resolution_signal` is the shared read-only producer.
It takes the actual per-harness state directories and counts the legacy paths and
`delivery_ledger::asserted_entries` without consuming them. The test
`the_resolution_signal_counts_each_clause_without_consuming_it` pins both counts
and repeat measurement. Missing or inaccessible hosts are not zero observations.
Fractal's doctor does not call this producer or print its advisory; its silence is
not evidence for either clause. The retired catalog CLI's state-tree recipe does
not inventory current Fractal driver state.

A **ratified rollback decision** must also retire the pre-ledger release line as a
supported rollback target. Record that decision under `docs/vrs/.decisions/` and
cite its exact artifact here before deletion. No such decision is established by
this reference. A rolled-back binary can write another old record; the test
`a_rollback_then_roll_forward_does_not_see_the_record_written_in_between` pins the
boundary that makes the decision necessary.

## Eventual removal

Only after the fleet observations and rollback decision are evidenced, remove the
`migrations/delivery_state` module and its `Ledger::open` seam, then remove the
unused `Attestation`, `Entry.attestation`, `asserted_entries`, and resolution-signal
code and proofs. The local absent-old-record test proves only that local no-op
case; it does not establish fleet-wide absence. Root CLI retirement supplies none
of the deletion evidence above.
