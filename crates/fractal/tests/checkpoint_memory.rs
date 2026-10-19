//! A checkpoint pass holds a fixed working set plus one slot, whatever the size of the history
//! (#2492, docs/st3/checkpoint-capture.md). The counting allocator is process-wide, so this is a
//! test binary of its own with one test.
//!
//! It counts the Rust heap. SQLite's own caches are bounded by configuration (each connection's
//! `cache_size`, and `temp_store = FILE` for its sorter), not counted here.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call forwards to the system allocator unchanged; the counters only observe.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            grow(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            grow(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(pointer, layout, size) };
        if !moved.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            grow(size);
        }
        moved
    }
}

fn grow(size: usize) {
    let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The most live heap `work` reached above what was live when it started.
fn peak_during<T>(work: impl FnOnce() -> T) -> (T, usize) {
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let result = work();
    (result, PEAK.load(Ordering::Relaxed).saturating_sub(baseline))
}

const MIB: usize = 1024 * 1024;
/// The old capture, which held the set, raised live heap by 14 MiB at `SMALL` claims and 54 MiB
/// at `LARGE`, so it fails both assertions below.
const BUDGET: usize = 8 * MIB;
/// Small enough for a debug build in CI, where the proof and the trim cost milliseconds a claim,
/// and a whole number of subjects.
const SMALL: usize = 4_800;
const LARGE: usize = 4 * SMALL;

/// `claims` diagnostics from one writer, one envelope each, sixteen to a subject and four to a
/// slot (subject and code), so the rule drops three of every four. Bulk statements: one per
/// table, as `production_history` in the trim tests explains.
fn history(store: &st3::store::Store, claims: usize) {
    let mut connection = store.connection.lock().unwrap();
    let transaction = connection.transaction().unwrap();
    for sql in [
        "INSERT INTO batches(id, origin, replica_sequence, previous_hash, hash, accepted_at_unix_ms)
         SELECT printf('batch/elm/%d', n + 1), 'elm', n + 1, NULL, printf('batch-%d', n), printf('%d', 1000 + n)
         FROM history",
        "INSERT INTO claims(id, batch_id, subject, kind, origin, actor, body, predecessors, accepted_at_unix_ms)
         SELECT printf('%064x', n), printf('batch/elm/%d', n + 1), printf('daemon/elm-%d', n / 16),
                'daemon.diagnostic', 'elm', NULL,
                printf('{\"fields\":{\"code\":\"code-%d\",\"severity\":\"warning\",\"reason\":\"%s\"}}',
                       n % 4, printf('%.200c', 'x')),
                '[]', printf('%d', 1000 + n)
         FROM history",
        "INSERT INTO replica_envelopes(writer, sequence, envelope_hash, previous_hash, accepted_at_unix_ms,
                                       payload, batch_id, relay, receipt_state, received_at_unix_ms)
         SELECT 'elm', n + 1, printf('envelope-%d', n), NULL, printf('%d', 1000 + n), x'00',
                printf('batch/elm/%d', n + 1), 'elm', 'validated', printf('%d', 1000 + n)
         FROM history",
        "INSERT INTO replica_records(record_ref, writer, sequence, envelope_hash, position, raw, state,
                                     claim_id, updated_at_unix_ms)
         SELECT printf('record/%d', n), 'elm', n + 1, printf('envelope-%d', n), 0, x'00', 'valid',
                printf('%064x', n), printf('%d', 1000 + n)
         FROM history",
    ] {
        transaction
            .execute(
                &format!(
                    "WITH RECURSIVE history(n) AS (
                         SELECT 0 WHERE ?1>0 UNION ALL SELECT n+1 FROM history WHERE n+1<?1
                     ) {sql}"
                ),
                [claims],
            )
            .unwrap();
    }
    transaction.commit().unwrap();
}

struct Pass {
    capture_and_plan: usize,
    proof: usize,
    trim: usize,
    dropped: usize,
    seconds: f64,
}

fn pass(claims: usize) -> Pass {
    let directory = tempfile::tempdir().unwrap();
    let store = st3::store::Store::open(&directory.path().join("claims.sqlite3"), "alder").unwrap();
    history(&store, claims);
    let scratch = directory.path().join("checkpoint");
    let cut = 10 * 86_400_000;
    let checkpoint = smallclaims::store::checkpoint_name(cut);
    let started = std::time::Instant::now();
    smallclaims::store::checkpoint_staging::without_reference_plan(|| {
        let ((staged, plan), capture_and_plan) = peak_during(|| {
            let staged = store.checkpoint_staged_set(cut, None, &scratch).unwrap();
            let plan = store.plan_staged_checkpoint(&staged).unwrap();
            (staged, plan)
        });
        assert_eq!(plan.sealed_claims, claims);
        let (proof, proof_peak) =
            peak_during(|| store.prove_staged_checkpoint_plan(&staged, &plan, &scratch).unwrap());
        assert!(proof.passed, "{:?}", proof.mismatches);
        let ((), trim) = peak_during(|| {
            let mut actions = Vec::new();
            store.trim_staged_checkpoint(&checkpoint, &staged, &plan, &mut actions).unwrap();
        });
        Pass {
            capture_and_plan,
            proof: proof_peak,
            trim,
            dropped: plan.dropped_claims,
            seconds: started.elapsed().as_secs_f64(),
        }
    })
}

#[test]
fn a_checkpoint_pass_holds_a_fixed_budget_whatever_the_history() {
    let small = pass(SMALL);
    let large = pass(LARGE);
    for (claims, pass) in [(SMALL, &small), (LARGE, &large)] {
        eprintln!(
            "checkpoint pass of {claims} claims: capture+plan {:.1} MiB, proof {:.1} MiB, trim {:.1} MiB, {} dropped, {:.1} s",
            pass.capture_and_plan as f64 / MIB as f64,
            pass.proof as f64 / MIB as f64,
            pass.trim as f64 / MIB as f64,
            pass.dropped,
            pass.seconds,
        );
    }
    // The writer's newest claim is the newest of its slot too, so its guard keeps nothing more.
    assert_eq!(small.dropped, SMALL / 4 * 3, "the rule drops three of every four");
    assert_eq!(large.dropped, LARGE / 4 * 3, "the rule drops three of every four");
    for (stage, small, large) in [
        ("capture and plan", small.capture_and_plan, large.capture_and_plan),
        ("proof", small.proof, large.proof),
        ("trim", small.trim, large.trim),
    ] {
        assert!(small <= BUDGET, "{stage} holds {small} bytes at {SMALL} claims, above {BUDGET}");
        assert!(large <= BUDGET, "{stage} holds {large} bytes at {LARGE} claims, above {BUDGET}");
        // Anything proportional to history grows four times; the fixed set does not.
        assert!(
            large <= small.max(MIB) * 3 / 2,
            "{stage} grows with history: {small} bytes at {SMALL} claims, {large} at {LARGE}"
        );
    }
}
