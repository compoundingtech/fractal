# Fresh CI re-attestation

The protected CI adapter accepts repeatable `--re-attest RECORD_ID` selectors
alongside its run, attempt and artifact selectors. These are immutable degraded
check records, never caller statements. Configuration, repository, roles, keys,
workflow and recipe pins still come from the private operator profile.

The observer fetches predecessor refs from the fixed repository and verifies
that each contains one explicitly unsigned degraded CI check. It derives the
old repository/run/attempt/job identity from that record, then reads the original
attempt's run metadata and job objects from authenticated GitHub. The run must
belong to the configured repository and completed Workspace CI workflow path;
an unrelated workflow with the same check name cannot supply a predecessor.
It requires an exact job ID,
check name, conclusion, serialized job digest and completion timestamp match.
One run/job read pair per distinct old attempt supplies all its selected jobs.

The current run must independently pass the existing source and workflow trust
boundary. A link needs identical change identity, whole-tree digest, check name,
command and opaque input map. It also needs a different run or attempt and job,
with the fresh job starting after the old check completed. Reusing the original
job, changing an input, or inventing an old result cannot produce a link. There
is at most one predecessor per current required check. All links are validated
before any new observation is written.

Only a newly produced Full record gets `re_attests`. Signing failure preserves
the fresh facts at degraded tier with no completed re-attestation link. Replay
retains the original result and never promotes it. An already recorded Full job
cannot acquire a different link; request the link on its first observation or
run a fresh check. Test-tier services never produce Full re-attestations.

The data-only export includes linked predecessor notes and their source commits
so offline verification can validate the chain. The predecessor stays degraded;
its record is never modified. This is an explicit selector API, not automatic
rerun scheduling or shared fallback coordination. Production controllers still
need to publish refs and provide the correct predecessor selectors.

The response adds `re_attestations` samples and `re_attested_records`. Each sample
measures from the authenticated degraded check's completion to the signed Full
note's issuance time. This includes scheduling delay after the rerun completed;
using rerun completion alone would understate time to trusted evidence. Git note
issuance has one-second precision; elapsed values are expressed in milliseconds.
The batch's `re_attestation_elapsed_ms` is the maximum new sample, or null when
none exists. Replays contribute no new samples. Full-trust share and signing
latency keep their existing fresh-record denominators. Production aggregation,
risk-class deadlines and their enforcing policy remain separate work.
