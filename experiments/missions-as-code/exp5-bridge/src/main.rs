//! Experiment 5: run a mission program inside an isolated JS engine embedded in Rust, as a pure
//! function from the run's history to its next actions.
//!
//! The only surface is `__decide(program, historyJson) -> decisionJson`. The host registers no
//! functions: the script has no clock, timers, randomness of its own, file system or network.
//!
//! Usage: mission-bridge <quickjs|boa|v8|v8-snapshot|v8-make-snapshot> <fixtures-dir>

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

mod engines;
use engines::Engine;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let engine = args.get(1).map(String::as_str).unwrap_or("quickjs");
    let dir = PathBuf::from(args.get(2).map(String::as_str).unwrap_or("fixtures"));
    #[cfg(feature = "v8")]
    if engine == "v8-make-snapshot" {
        let bundle = fs::read_to_string(dir.join("bundle.js")).unwrap();
        engines::v8::make_snapshot(&bundle, &dir.join("bundle.snapshot"));
        return;
    }
    let report = match engine {
        #[cfg(feature = "quickjs")]
        "quickjs" => run::<engines::quickjs::QuickJs>(&dir),
        #[cfg(feature = "boa")]
        "boa" => run::<engines::boa::Boa>(&dir),
        #[cfg(feature = "v8")]
        "v8" => run::<engines::v8::V8>(&dir),
        #[cfg(feature = "v8")]
        "v8-snapshot" => run::<engines::v8::V8Snapshot>(&dir),
        other => panic!("engine {other} is not built; enable its cargo feature"),
    };
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

struct Fixture {
    name: String,
    program: String,
    history: String,
    prefix: String,
    decision: String,
    prefix_decision: String,
    events: usize,
}

fn fixtures(dir: &Path) -> Vec<Fixture> {
    let mut out = Vec::new();
    let mut names: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    names.sort();
    for path in names {
        let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let history = v["history"].as_array().unwrap();
        out.push(Fixture {
            name: path.file_stem().unwrap().to_string_lossy().into_owned(),
            program: v["program"].as_str().unwrap().to_owned(),
            history: serde_json::to_string(history).unwrap(),
            prefix: serde_json::to_string(&history[..history.len() - 1]).unwrap(),
            decision: serde_json::to_string(&v["decision"]).unwrap(),
            prefix_decision: serde_json::to_string(&v["prefixDecision"]).unwrap(),
            events: history.len(),
        });
    }
    out
}

fn rss_bytes() -> usize {
    let statm = fs::read_to_string("/proc/self/statm").unwrap_or_default();
    let pages: usize = statm.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    pages * 4096
}

fn percentile(samples: &mut [Duration], p: f64) -> f64 {
    samples.sort();
    let i = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[i].as_secs_f64() * 1e3
}

fn run<E: Engine>(dir: &Path) -> serde_json::Value {
    let bundle = fs::read_to_string(dir.join("bundle.js")).unwrap();
    let fixtures = fixtures(dir);

    // Start: a fresh engine with the bundle loaded, ready to decide.
    let rss_before = rss_bytes();
    let mut starts = Vec::new();
    for _ in 0..10 {
        let t = Instant::now();
        let e = E::load(&bundle);
        starts.push(t.elapsed());
        drop(e);
    }
    let start_ms = percentile(&mut starts, 0.5);

    // Memory: hold 20 loaded engines at once and divide.
    let base = rss_bytes();
    let held: Vec<E> = (0..20).map(|_| E::load(&bundle)).collect();
    let per_engine_rss = (rss_bytes().saturating_sub(base)) / 20;
    let engine_heap = held[0].heap_bytes();
    drop(held);

    // Per-decision cost and determinism, on one warm engine.
    let mut engine = E::load(&bundle);
    let mut cases = Vec::new();
    let mut all_match = true;
    for f in &fixtures {
        let first = engine.decide(&f.program, &f.history).expect("decide");
        let prefix = engine.decide(&f.program, &f.prefix).expect("decide");
        let matches = first == f.decision && prefix == f.prefix_decision;
        all_match &= matches;
        let iterations = if f.events > 1000 { 20 } else if f.events > 100 { 100 } else { 300 };
        let mut samples = Vec::with_capacity(iterations);
        let mut repeat_identical = true;
        for _ in 0..iterations {
            let t = Instant::now();
            let out = engine.decide(&f.program, &f.prefix).expect("decide");
            samples.push(t.elapsed());
            repeat_identical &= out == f.prefix_decision;
        }
        // A fresh engine decides the same way as a warm one.
        let fresh = E::load(&bundle).decide(&f.program, &f.prefix).expect("decide");
        cases.push(serde_json::json!({
            "fixture": f.name,
            "events": f.events,
            "matches_node": matches,
            "repeatable": repeat_identical,
            "fresh_equals_warm": fresh == f.prefix_decision,
            "p50_ms": percentile(&mut samples, 0.5),
            "p99_ms": percentile(&mut samples, 0.99),
            "per_event_us": percentile(&mut samples, 0.5) * 1e3 / f.events as f64,
        }));
    }

    // Incremental: open a live run once, then feed one event at a time, as a driver or the daemon
    // would after each result. The final decision must equal Node's full replay.
    let mut incremental = Vec::new();
    for f in fixtures.iter().filter(|f| ["ship-messy", "long-100", "long-1000", "wide-1000"].contains(&f.name.as_str())) {
        let events: Vec<serde_json::Value> = serde_json::from_str(&f.history).unwrap();
        let first = serde_json::to_string(&events[..1]).unwrap();
        let t = Instant::now();
        let id = engine.call("__open", &[&f.program, &first]).expect("open");
        let open_ms = t.elapsed().as_secs_f64() * 1e3;
        let mut samples = Vec::with_capacity(events.len());
        let mut last = String::new();
        for event in &events[1..] {
            let one = serde_json::to_string(&[event]).unwrap();
            let t = Instant::now();
            last = engine.call("__apply", &[&id, &one]).expect("apply");
            samples.push(t.elapsed());
        }
        engine.call("__close", &[&id]).unwrap();
        let mut expected: serde_json::Value = serde_json::from_str(&f.decision).unwrap();
        expected.as_object_mut().unwrap().remove("stats");
        let got: serde_json::Value = serde_json::from_str(&last).unwrap();
        let total: f64 = samples.iter().map(|d| d.as_secs_f64() * 1e3).sum();
        incremental.push(serde_json::json!({
            "fixture": f.name,
            "events": f.events,
            "matches_full_replay": got == expected,
            "open_ms": open_ms,
            "apply_p50_us": percentile(&mut samples, 0.5) * 1e3,
            "apply_p99_us": percentile(&mut samples, 0.99) * 1e3,
            "apply_max_us": percentile(&mut samples, 1.0) * 1e3,
            "whole_run_ms": total,
        }));
    }

    // Live-run memory: hold 50 open sessions of the 1000-step run in one engine.
    let long = fixtures.iter().find(|f| f.name == "long-1000").unwrap();
    let mut holder = E::load(&bundle);
    let heap0 = holder.heap_bytes();
    let rss0 = rss_bytes();
    let ids: Vec<String> = (0..50).map(|_| holder.call("__open", &[&long.program, &long.prefix]).unwrap()).collect();
    let session_heap_kb = holder.heap_bytes().zip(heap0).map(|(a, b)| a.saturating_sub(b) / 50 / 1024);
    let session_rss_kb = rss_bytes().saturating_sub(rss0) / 50 / 1024;
    for id in ids {
        holder.call("__close", &[&id]).unwrap();
    }
    drop(holder);

    // Isolation: a program that never yields must be stopped by the bridge, not hang the caller.
    let spin = r#"[{"type":"started","at":1,"program":"orchard/spin","version":"dev","input":null},{"type":"scheduled","at":1,"op":"first","kind":"step","hash":"x"},{"type":"completed","at":2,"op":"first","value":{"summary":"ok","evidence":[],"attempt":1}}]"#;
    let mut fresh = E::load(&bundle);
    let t = Instant::now();
    let spin_result = fresh.decide_with_budget("orchard/spin", spin, Duration::from_millis(50));
    let spin_stopped_ms = t.elapsed().as_secs_f64() * 1e3;
    let clock = r#"[{"type":"started","at":1,"program":"orchard/reads-clock","version":"dev","input":null}]"#;
    let clock_result = E::load(&bundle).decide("orchard/reads-clock", clock);

    serde_json::json!({
        "engine": E::NAME,
        "bundle_kb": bundle.len() / 1024,
        "start_ms_p50": start_ms,
        "rss_per_engine_kb": per_engine_rss / 1024,
        "engine_heap_kb": engine_heap.map(|b| b / 1024),
        "process_rss_mb_at_start": rss_before / (1024 * 1024),
        "all_match_node": all_match,
        "cases": cases,
        "incremental": incremental,
        "session_of_2000_events": { "heap_kb": session_heap_kb, "rss_kb": session_rss_kb },
        "spin": { "stopped": spin_result.is_err(), "after_ms": spin_stopped_ms, "error": spin_result.err() },
        "reads_clock": clock_result.map_err(|e| e).unwrap_or_else(|e| e),
    })
}
