//! A local cut's capture of a copy of a store, scoped and whole, with what each takes and plans,
//! then the citation cache verified against a full read; with `trim`, the cut is then made, as
//! the daemon makes it, and what it freed is measured:
//! `cargo run --release -p fractal-store --example local_capture_bench -- STORE SCRATCH ORIGIN [CUT_UNIX_MS [trim]]`.
//! Run it on a copy: it opens the store as the daemon would, and `trim` deletes from it.
use fractal_store::store::Store;
use smallclaims::store::{LOCAL_TRIM_KINDS, local_trim_capture_kinds};
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

/// The store's pages, free pages and page size.
fn pages(store: &Store) -> (i64, i64, i64) {
    let connection = store.readers.get();
    let pragma = |name: &str| connection.query_row(&format!("PRAGMA {name}"), [], |row| row.get::<_, i64>(0)).unwrap();
    (pragma("page_count"), pragma("freelist_count"), pragma("page_size"))
}

/// The bytes of the claim bodies and raw records of `claims`.
fn claim_bytes(store: &Store, claims: &[String]) -> (i64, i64) {
    let connection = store.readers.get();
    let mut body = connection.prepare("SELECT COALESCE(SUM(LENGTH(body)), 0) FROM claims WHERE id=?1").unwrap();
    let mut raw = connection.prepare("SELECT COALESCE(SUM(LENGTH(raw)), 0) FROM replica_records WHERE claim_id=?1").unwrap();
    claims.iter().fold((0, 0), |(bodies, raws), id| {
        (
            bodies + body.query_row([id], |row| row.get::<_, i64>(0)).unwrap(),
            raws + raw.query_row([id], |row| row.get::<_, i64>(0)).unwrap(),
        )
    })
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
    if let Ok(every) = std::env::var("LOCAL_BENCH_QUERIES") {
        // The slowest statements of the last five minutes, every `every` seconds.
        let every = std::time::Duration::from_secs(every.parse().unwrap());
        std::thread::spawn(move || loop {
            std::thread::sleep(every);
            for query in smallclaims::performance::snapshot()["queries"].as_array().into_iter().flatten().take(8) {
                println!("query: {}", query.to_string().chars().take(600).collect::<String>());
            }
        });
    }
    let trim_only = args.get(5).map(String::as_str) == Some("trim-only");
    let started = Instant::now();
    store.catch_up_local_trim_citations().unwrap();
    println!("citation cache caught up from nothing in {:?}", started.elapsed());
    let capture = local_trim_capture_kinds();
    for (label, scope) in [("scoped", Some(capture.as_slice())), ("whole", None)].into_iter().filter(|_| !trim_only) {
        let dir = scratch.join(label);
        let started = Instant::now();
        let staged = store.checkpoint_staged_set_scoped(cut, None, &dir, scope).unwrap();
        let captured = started.elapsed();
        let size = bytes(&staged.path);
        staged.protect_all_but(LOCAL_TRIM_KINDS).unwrap();
        staged.split_envelopes().unwrap();
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
        if label == "scoped" {
            let (bodies, raws) = claim_bytes(&store, &claims);
            println!("scoped: dropped claims hold {bodies} bytes of bodies and {raws} bytes of raw records");
        }
        println!("{label}: guard answers digest {}", hex::encode(sha2::Sha256::digest(staged.guard_answers(LOCAL_TRIM_KINDS).unwrap().join("\n").as_bytes())));
    }
    let started = Instant::now();
    let check = if trim_only { store.catch_up_local_trim_citations().map(|()| None).unwrap() } else { Some(store.verify_local_trim_citations().unwrap()) };
    if let Some(check) = check { println!(
        "verify: {:?}, {} claims read, {} citations between present claims, {} missing, {} extra",
        started.elapsed(),
        check.claims_read,
        check.citations,
        check.missing.len(),
        check.extra.len(),
    ); }
    if !matches!(args.get(5).map(String::as_str), Some("trim" | "trim-only")) {
        return;
    }
    let (before_pages, before_free, page_size) = pages(&store);
    store.trim_locally(true).unwrap();
    let context = smallclaims::store::checkpoint_agreement::CheckpointContext {
        now_unix_ms: cut + 2 * smallclaims::store::checkpoint::DAY_MS + 1,
        configured_peers: Vec::new(),
        scratch: scratch.join("trim"),
        reviewer: "person/operator".into(),
    };
    let started = Instant::now();
    let mut steps = 0;
    loop {
        steps += 1;
        let actions = store.checkpoint_step(&context).unwrap();
        let state: Option<String> = store
            .readers
            .get()
            .query_row("SELECT state FROM checkpoints WHERE cut_unix_ms=?1", [i64::try_from(cut).unwrap()], |row| row.get(0))
            .ok();
        if state.as_deref() != Some("trimming") || actions.is_empty() {
            println!("trim: {state:?} after {steps} step(s), {:?}; last actions {actions:?}", started.elapsed());
            break;
        }
    }
    let (after_pages, after_free, _) = pages(&store);
    println!(
        "trim: pages {before_pages} -> {after_pages}, free pages {before_free} -> {after_free} ({} bytes freed in the file)",
        (after_free - before_free) * page_size
    );
    let check = store.verify_local_trim_citations().unwrap();
    println!("after trim, verify: {} missing, {} extra", check.missing.len(), check.extra.len());
}
