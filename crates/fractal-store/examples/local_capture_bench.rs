//! A local cut's capture of a copy of a store, scoped and whole, with what each takes and plans,
//! then the citation cache verified against a full read:
//! `cargo run --release -p fractal-store --example local_capture_bench -- STORE SCRATCH ORIGIN [CUT_UNIX_MS]`.
//! Run it on a copy: it opens the store as the daemon would.
use fractal_store::store::Store;
use smallclaims::store::LOCAL_TRIM_KINDS;
use sha2::Digest;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

fn bytes(path: &Path) -> u64 {
    ["", "-wal"]
        .iter()
        .filter_map(|suffix| std::fs::metadata(format!("{}{suffix}", path.display())).ok())
        .map(|metadata| metadata.len())
        .sum()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (store, scratch, origin) = (PathBuf::from(&args[1]), PathBuf::from(&args[2]), args[3].clone());
    let opened = Instant::now();
    let store = Store::open(&store, origin).unwrap();
    println!("opened in {:?}", opened.elapsed());
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis();
    let cut = args.get(4).map_or_else(|| smallclaims::store::checkpoint::newest_due_cut(now), |cut| cut.parse().unwrap());
    println!("cut {cut} ({})", smallclaims::store::checkpoint::checkpoint_name(cut));
    let started = Instant::now();
    store.catch_up_local_trim_citations().unwrap();
    println!("citation cache caught up from nothing in {:?}", started.elapsed());
    for (label, scope) in [("scoped", Some(LOCAL_TRIM_KINDS)), ("whole", None)] {
        let dir = scratch.join(label);
        let started = Instant::now();
        let staged = store.checkpoint_staged_set_scoped(cut, None, &dir, scope).unwrap();
        let captured = started.elapsed();
        let size = bytes(&staged.path);
        staged.protect_all_but(LOCAL_TRIM_KINDS).unwrap();
        let started = Instant::now();
        // The reference planner knows no scope, so a local cut never compares with it.
        let plan = smallclaims::store::checkpoint_staging::without_reference_plan(|| store.plan_staged_checkpoint(&staged)).unwrap();
        println!(
            "{label}: capture {captured:?}, staging {size} bytes, plan {:?}: {} of {} envelopes and {} of {} claims dropped",
            started.elapsed(),
            plan.dropped_envelopes,
            plan.sealed_envelopes,
            plan.dropped_claims,
            plan.sealed_claims,
        );
        let claims: Vec<String> = staged.dropped_claims(0, usize::MAX >> 1).unwrap().into_iter().map(|claim| claim.id).collect();
        let digest = sha2::Sha256::digest(claims.join("\n").as_bytes());
        println!("{label}: dropped claims digest {}", hex::encode(digest));
        println!("{label}: guard answers digest {}", hex::encode(sha2::Sha256::digest(staged.guard_answers(LOCAL_TRIM_KINDS).unwrap().join("\n").as_bytes())));
    }
    let started = Instant::now();
    let check = store.verify_local_trim_citations().unwrap();
    println!(
        "verify: {:?}, {} claims read, {} citations between present claims, {} missing, {} extra",
        started.elapsed(),
        check.claims_read,
        check.citations,
        check.missing.len(),
        check.extra.len(),
    );
}
