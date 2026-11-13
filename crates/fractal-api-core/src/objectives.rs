//! Objectives and budgets as one list.
//!
//! An objective is a target with windows and an error budget: the daemon's latency targets, its
//! store and CPU targets, the database's size and the paired phone's reported targets, all from
//! `slo/objectives.kdl` and the same report as `GET /v1/client/request-latency`. A budget is an
//! allowance that resets: each account's weekly and 5-hour limits, from the readings `st usage`
//! shows. Nothing here measures anything. It reads those sources and says, for each row, its
//! target, its current value, its window, what is left of its budget and what that means.
//!
//! Every row has the same fields, so a terminal, the phone and the web render one shape:
//! `id`, `kind` (`objective` or `budget`), `about`, `target`, `current`, `window`,
//! `remaining_percent` (null when nothing was measured), `status` (`met`, `missed`, `no-data` or
//! `stale`), `resets_at_unix_ms` (budgets only) and `explain`, the row in plain words.

use serde_json::{Value, json};

use crate::store::AccountLimit;

const WINDOWS: [&str; 3] = ["1m", "5m", "1h"];
/// A 99th-percentile target allows one sample in a hundred to run over.
const P99_ALLOWANCE: f64 = 0.01;
const GB: f64 = 1e9;
const MB: f64 = 1e6;

/// Every objective in `report` (the request-latency read's `targets`) and every budget in
/// `limits`, objectives first. `stale_after_ms` is how old a limits reading may be before it is
/// the last known one rather than the current one.
pub fn rows(report: &Value, limits: &[AccountLimit], now_unix_ms: u64, stale_after_ms: u64) -> Vec<Value> {
    let mut rows = Vec::new();
    for target in report["targets"].as_array().into_iter().flatten() {
        let Some(name) = target["name"].as_str() else { continue };
        let about = target["about"].as_str().unwrap_or_default();
        match name {
            "sql-statement" => rows.push(p99(name, about, "SQL statements", target["p99_ms"].as_f64(), &target["windows"])),
            "transaction" => {
                let max = target["max_ms"].as_f64();
                for (kind, label) in [
                    ("read", "read transactions"),
                    ("write", "write transactions"),
                    ("writer_hold", "holds of the store's writer"),
                ] {
                    rows.push(max_latency(&format!("{name}/{kind}"), about, label, max, &target[kind]));
                }
            }
            "cpu" => rows.extend(cpu(about, target)),
            "database" => rows.push(database(about, target)),
            _ if target["min_percent"].is_number() => rows.push(share(name, about, target)),
            _ => {
                rows.push(p99(name, about, "requests", target["p99_ms"].as_f64(), &target["windows"]));
                if target["remote_windows"].is_object() {
                    rows.push(p99(
                        &format!("{name}/remote"),
                        about,
                        "requests served from another machine",
                        target["remote_p99_ms"].as_f64(),
                        &target["remote_windows"],
                    ));
                }
            }
        }
    }
    for limit in limits {
        let name = limit.account_ref.as_deref().unwrap_or(&limit.account);
        if !limit.windows.is_empty() {
            for window in &limit.windows {
                let label = if window.window == "5h" { "5-hour" } else { &window.window };
                let stale = now_unix_ms.saturating_sub(window.measured_at_unix_ms) > stale_after_ms;
                let mut row = budget(name, &limit.account, &window.window, label, window.used_percent,
                    window.resets_at_unix_ms, window.measured_at_unix_ms, stale, now_unix_ms);
                row["source"] = json!(window.source);
                row["host"] = json!(window.host);
                row["coverage"] = json!(window.coverage);
                if window.coverage == "unsupported" {
                    row["status"] = json!("unsupported");
                    row["current"] = json!("unsupported");
                    row["remaining_percent"] = Value::Null;
                    row["explain"] = json!(format!("{} does not support {} quota evidence for {name}.", window.source, window.window));
                } else if window.coverage != "reported" {
                    row["status"] = json!(if stale { "stale" } else { "no-data" });
                    row["current"] = json!("not reported");
                    row["remaining_percent"] = Value::Null;
                }
                rows.push(row);
            }
            continue;
        }
        let stale = now_unix_ms.saturating_sub(limit.measured_at_unix_ms) > stale_after_ms;
        for (period, label, percent, resets) in [
            ("weekly", "weekly", limit.weekly_percent, limit.weekly_resets_at_unix_ms),
            ("5h", "5-hour", limit.five_hour_percent, limit.five_hour_resets_at_unix_ms),
        ] {
            rows.push(budget(name, &limit.account, period, label, percent, resets, limit.measured_at_unix_ms, stale, now_unix_ms));
        }
    }
    rows
}

pub fn count(window: &Value) -> u64 {
    window["count"].as_u64().unwrap_or_default()
}

fn share_of(window: &Value) -> f64 {
    window["over_target_share"].as_f64().unwrap_or_default()
}

pub fn ms(value: f64) -> String {
    if value >= 10.0 { format!("{} ms", value.round()) } else { format!("{:.1} ms", value) }
}

pub(crate) fn clamp_percent(value: f64) -> f64 {
    (value.clamp(0.0, 100.0) * 10.0).round() / 10.0
}

pub(crate) fn plural(n: u64, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

/// A 99th-percentile target: no more than 1% of samples over, in each window. The budget is that
/// 1%, measured over the hour, the longest window.
fn p99(id: &str, about: &str, what: &str, target_ms: Option<f64>, windows: &Value) -> Value {
    let target = target_ms.unwrap_or_default();
    let hour = &windows["1h"];
    let samples = count(hour);
    let met = WINDOWS.iter().all(|w| share_of(&windows[*w]) <= P99_ALLOWANCE);
    if samples == 0 && WINDOWS.iter().all(|w| count(&windows[*w]) == 0) {
        return json!({
            "id": id, "kind": "objective", "about": about,
            "target": format!("p99 ≤ {}", ms(target)), "current": "no samples", "window": "1h",
            "remaining_percent": null, "status": "no-data",
            "explain": format!(
                "{about}. Goal: 99 of every 100 {what} finish within {}. Nothing has been measured in the last hour, so there is no budget to spend or save yet.",
                ms(target)
            ),
        });
    }
    let remaining = clamp_percent(100.0 * (1.0 - share_of(hour) / P99_ALLOWANCE));
    let p99_now = hour["p99_ms"].as_f64().unwrap_or_default();
    let over = hour["over_target"].as_u64().unwrap_or_default();
    let verdict = if met {
        "Every window is within its allowance.".to_owned()
    } else {
        let missed = WINDOWS
            .iter()
            .filter(|w| share_of(&windows[**w]) > P99_ALLOWANCE)
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        format!("The goal is missed in the {missed} window.")
    };
    json!({
        "id": id, "kind": "objective", "about": about,
        "target": format!("p99 ≤ {}", ms(target)),
        "current": format!("p99 {} over {}", ms(p99_now), plural(samples, "sample")),
        "window": "1h", "remaining_percent": remaining,
        "status": if met { "met" } else { "missed" },
        "explain": format!(
            "{about}. Goal: 99 of every 100 {what} finish within {}. In the last hour {} of {} were slower ({:.1}%), against an allowance of 1%, so {remaining:.0}% of the error budget is left. The slowest 1% start at {}. {verdict}",
            ms(target), over, plural(samples, "sample"), share_of(hour) * 100.0, ms(p99_now),
        ),
    })
}

/// A maximum target: not one sample over. The budget is all or nothing.
fn max_latency(id: &str, about: &str, what: &str, target_ms: Option<f64>, windows: &Value) -> Value {
    let target = target_ms.unwrap_or_default();
    let hour = &windows["1h"];
    let samples = count(hour);
    if WINDOWS.iter().all(|w| count(&windows[*w]) == 0) {
        return json!({
            "id": id, "kind": "objective", "about": about,
            "target": format!("max ≤ {}", ms(target)), "current": "no samples", "window": "1h",
            "remaining_percent": null, "status": "no-data",
            "explain": format!("{about}. Goal: no {what} take longer than {}. Nothing has been measured in the last hour.", ms(target)),
        });
    }
    let met = WINDOWS.iter().all(|w| windows[*w]["over_target"].as_u64().unwrap_or_default() == 0);
    let over = hour["over_target"].as_u64().unwrap_or_default();
    json!({
        "id": id, "kind": "objective", "about": about,
        "target": format!("max ≤ {}", ms(target)),
        "current": format!("max {} over {}", ms(hour["max_ms"].as_f64().unwrap_or_default()), plural(samples, "sample")),
        "window": "1h", "remaining_percent": if over == 0 { 100.0 } else { 0.0 },
        "status": if met { "met" } else { "missed" },
        "explain": format!(
            "{about}. Goal: no {what} take longer than {}; one slow sample misses it. In the last hour {} of {} ran over. {}",
            ms(target), over, plural(samples, "sample"),
            if met { "No window has a slow sample." } else { "A slow sample is still inside a window." },
        ),
    })
}

/// The member's CPU, its daemon and replication workers, as two rows: `cpu` over the hour, and
/// `cpu/idle` over the hour's idle minutes. Either is no-data while it is unknown: the hour not
/// whole, a minute's CPU not known, or too few idle minutes to judge.
fn cpu(about: &str, target: &Value) -> [Value; 2] {
    use crate::slo::cores;
    let verdict = crate::slo::cpu_verdict(target);
    let max = target["max_cores"].as_f64().unwrap_or_default();
    let idle_max = target["idle_max_cores"].as_f64().unwrap_or_default();
    let needed = target["idle_min_minutes"].as_u64().unwrap_or_default();
    let window = |w: &str| target["windows"][w]["cores"].as_f64();
    let status = |met: Option<bool>| match met {
        Some(true) => "met",
        Some(false) => "missed",
        None => "no-data",
    };
    let remaining = |value: Option<f64>, of: f64| value.map(|value| clamp_percent(100.0 * (1.0 - value / of)));
    let not_idle = target["idle"]["not_idle"]
        .as_object()
        .map(|reasons| {
            reasons
                .iter()
                .filter(|(_, n)| n.as_u64().unwrap_or_default() > 0)
                .map(|(reason, n)| format!("{reason} {n}"))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|reasons| !reasons.is_empty())
        .map_or(String::new(), |reasons| format!(" Minutes not idle, by why (one may have several reasons): {reasons}."));
    let hour = match verdict.hour {
        Some(hour) => json!({
            "current": format!("{hour:.2} cores"),
            "explain": format!(
                "{about}. Goal: the member uses at most {} over the hour. Over the last hour it used {hour:.2} on average, so {:.0}% of that allowance is left. Last minute {}, last five minutes {}.",
                cores(max),
                remaining(Some(hour), max).unwrap_or_default(),
                window("1m").map_or("unknown".to_owned(), |c| format!("{c:.2}")),
                window("5m").map_or("unknown".to_owned(), |c| format!("{c:.2}")),
            ),
        }),
        None => json!({
            "current": format!("unknown: {} of 60 minutes known", target["windows"]["1h"]["minutes"].as_u64().unwrap_or_default()),
            "explain": format!(
                "{about}. Goal: the member uses at most {} over the hour. The hour is not known: a minute is known only once the daemon has been up for all of it and the CPU of every replication worker it runs is known for it.",
                cores(max),
            ),
        }),
    };
    let idle = match verdict.idle {
        Some(idle) => json!({
            "current": format!("{idle:.2} cores over {}", plural(verdict.idle_minutes, "idle minute")),
            "explain": format!(
                "{about}. Goal: at most {} on average over the hour's idle minutes. {} of the last 60 were idle and used {idle:.2} cores, so {:.0}% of that allowance is left.{not_idle}",
                cores(idle_max), verdict.idle_minutes, remaining(Some(idle), idle_max).unwrap_or_default(),
            ),
        }),
        None => json!({
            "current": if verdict.minutes < 60 {
                format!("unknown: {} of 60 minutes held", verdict.minutes)
            } else {
                format!("unknown: {}, needs {needed}", plural(verdict.idle_minutes, "idle minute"))
            },
            "explain": format!(
                "{about}. Goal: at most {} on average over the hour's idle minutes. It is judged once the daemon has been up for the whole hour and at least {needed} of its minutes are known to be idle; the hour holds {} minutes, {} of them idle.{not_idle}",
                cores(idle_max), verdict.minutes, verdict.idle_minutes,
            ),
        }),
    };
    [
        json!({
            "id": "cpu", "kind": "objective", "about": about,
            "target": format!("≤ {} over the hour", cores(max)), "current": hour["current"], "window": "1h",
            "remaining_percent": remaining(verdict.hour, max), "status": status(verdict.hour_met),
            "explain": hour["explain"],
        }),
        json!({
            "id": "cpu/idle", "kind": "objective", "about": about,
            "target": format!("≤ {} over idle minutes", cores(idle_max)), "current": idle["current"], "window": "1h",
            "remaining_percent": remaining(verdict.idle, idle_max), "status": status(verdict.idle_met),
            "explain": idle["explain"],
        }),
    ]
}

fn database(about: &str, target: &Value) -> Value {
    let max_live = target["max_live_gb"].as_f64().unwrap_or_default();
    let max_growth = target["max_growth_mb_per_day"].as_f64().unwrap_or_default();
    let size = &target["size"];
    let Some(live) = size["live_bytes"].as_f64() else {
        return json!({
            "id": "database", "kind": "objective", "about": about,
            "target": format!("live ≤ {max_live} GB"), "current": "not measured yet", "window": "1d",
            "remaining_percent": null, "status": "no-data",
            "explain": format!("{about}. The store's size is measured hourly and has not been measured yet."),
        });
    };
    let growth = size["growth_bytes_per_day"].as_f64();
    let met = live / GB <= max_live && growth.is_none_or(|growth| growth / MB <= max_growth);
    let remaining = clamp_percent(100.0 * (1.0 - live / GB / max_live));
    let growth_text = growth.map_or("growth is not known yet".to_owned(), |growth| format!("it is growing {:+.0} MB a day", growth / MB));
    json!({
        "id": "database", "kind": "objective", "about": about,
        "target": format!("live ≤ {max_live} GB, growth ≤ {max_growth} MB a day"),
        "current": format!("live {:.2} GB", live / GB), "window": "1d",
        "remaining_percent": remaining, "status": if met { "met" } else { "missed" },
        "explain": format!(
            "{about}. Goal: no more than {max_live} GB of pages in use, growing no faster than {max_growth} MB a day. It holds {:.2} GB, so {remaining:.0}% of the size allowance is left, and {growth_text}.",
            live / GB,
        ),
    })
}

/// Open projection divergences, counted fleet-wide from their faults. Each is a P1 bug, so the
/// target is none, and one open divergence spends the whole allowance.
pub fn divergence(open: u64) -> Value {
    let targets = crate::slo::targets();
    let about = targets.divergence.about.as_str();
    let max_open = targets.divergence.max_open;
    let met = open <= max_open;
    json!({
        "id": "projection-divergence", "kind": "objective", "about": about,
        "target": format!("≤ {max_open} open"), "current": format!("{open} open"), "window": "now",
        "remaining_percent": if met { 100.0 } else { 0.0 }, "status": if met { "met" } else { "missed" },
        "explain": format!(
            "{about}. Goal: at most {max_open} open. {}",
            if open == 0 {
                "None is open.".to_owned()
            } else {
                format!(
                    "{} open; `st subject daemon/NODE` for the member that found one shows it with the projection, the claim range, both digests and both builds.",
                    plural(open, "divergence")
                )
            }
        ),
    })
}

/// A duration in seconds under a minute and in minutes from a minute on: a declared interval's
/// targets may be seconds or minutes.
pub(crate) fn duration(target_ms: u64) -> String {
    if target_ms < 1_000 {
        return ms(target_ms as f64);
    }
    if target_ms < 60_000 {
        let seconds = target_ms as f64 / 1_000.0;
        return if seconds.fract() == 0.0 { format!("{seconds} s") } else { format!("{seconds:.1} s") };
    }
    let minutes = target_ms as f64 / 60_000.0;
    if minutes.fract() == 0.0 { format!("{minutes} min") } else { format!("{minutes:.1} min") }
}

/// The value at `percentile` of `samples`, nearest rank: the smallest sample that at least
/// `percentile` in 100 are at or under.
pub(crate) fn nearest_rank(samples: &[u64], percentile: u64) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = (percentile as usize * sorted.len()).div_ceil(100).max(1);
    sorted[rank - 1]
}

/// A minimum share (a phone's live time over its foreground time). The budget is the allowed
/// remainder: with a 99% minimum, 1% of foreground time may be disconnected.
fn share(id: &str, about: &str, target: &Value) -> Value {
    let min = target["min_percent"].as_f64().unwrap_or_default();
    let hour = &target["windows"]["1h"];
    let met = WINDOWS.iter().all(|w| {
        let window = &target["windows"][*w];
        window["over_target"].as_f64().unwrap_or_default() * 100.0
            <= window["foreground_ms"].as_f64().unwrap_or_default() * (100.0 - min)
    });
    let Some(live) = hour["live_share"].as_f64() else {
        return json!({
            "id": id, "kind": "objective", "about": about, "target": format!("live ≥ {min}%"),
            "current": "no reported time", "window": "1h", "remaining_percent": null, "status": "no-data",
            "explain": format!("{about}. No paired phone has reported foreground time in the last hour."),
        });
    };
    let allowed = (100.0 - min) / 100.0;
    let remaining = clamp_percent(100.0 * (1.0 - (1.0 - live) / allowed));
    json!({
        "id": id, "kind": "objective", "about": about,
        "target": format!("live ≥ {min}%"), "current": format!("live {:.1}%", live * 100.0), "window": "1h",
        "remaining_percent": remaining, "status": if met { "met" } else { "missed" },
        "explain": format!(
            "{about}. Goal: connected for at least {min}% of the time the app is open. Over the last hour it was {:.1}%, so {remaining:.0}% of the {:.0}% that may be disconnected is left. This counts only phones that report.",
            live * 100.0, 100.0 - min,
        ),
    })
}

#[allow(clippy::too_many_arguments)]
fn budget(
    name: &str,
    label: &str,
    period: &str,
    period_label: &str,
    percent: Option<f64>,
    resets_at: Option<u64>,
    measured_at: u64,
    stale: bool,
    now: u64,
) -> Value {
    let id = format!("{name}/{period}");
    let reset = resets_at.map_or("no reset time reported".to_owned(), |at| {
        let minutes = (at.saturating_sub(now) / 60_000) as i64;
        if at <= now {
            "it has reset since".to_owned()
        } else if minutes < 120 {
            format!("it resets in {minutes} minutes")
        } else if minutes < 48 * 60 {
            format!("it resets in {} hours", minutes / 60)
        } else {
            format!("it resets in {} days", minutes / (24 * 60))
        }
    });
    let Some(used) = percent else {
        return json!({
            "id": id, "kind": "budget", "account": label, "about": format!("{name}'s {period_label} limit"),
            "target": "100% allowance", "current": "not reported", "window": period_label,
            "remaining_percent": null, "status": "no-data", "resets_at_unix_ms": resets_at,
            "measured_at_unix_ms": measured_at,
            "explain": format!("No seat on {name} has reported its {period_label} limit."),
        });
    };
    let remaining = clamp_percent(100.0 - used);
    let status = if stale { "stale" } else if used >= 100.0 { "missed" } else { "met" };
    json!({
        "id": id, "kind": "budget", "account": label, "about": format!("{name}'s {period_label} limit"),
        "target": "100% allowance", "current": format!("{used:.0}% used"), "window": period_label,
        "remaining_percent": remaining, "status": status, "resets_at_unix_ms": resets_at,
        "measured_at_unix_ms": measured_at,
        "explain": format!(
            "An allowance that resets: {name}'s {period_label} limit. {used:.0}% is used, so {remaining:.0}% is left, and {reset}.{}",
            if stale { " This is the last reading, and it is not recent." } else { "" },
        ),
    })
}

const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 60 * MINUTE_MS;
/// A reading that rises by more than this between two minutes is a reset (or a new account
/// reading), not spending: the rate is read only from what follows it.
const RESET_JUMP_PERCENT: f64 = 1.0;
/// Fewest readings a rate is worth fitting to; the log has one a minute.
const MIN_FORECAST_READINGS: usize = 5;

/// How far back a budget's recent rate is read, or `None` for a row that has no forecast: a
/// budget whose reset is known and whose newest reading is current. A 5-hour limit follows the
/// last hour; a weekly one, the last day.
pub fn forecast_basis_ms(row: &Value, now_unix_ms: u64) -> Option<u64> {
    if row["kind"] != "budget" || !matches!(row["status"].as_str(), Some("met" | "missed")) {
        return None;
    }
    row["remaining_percent"].as_f64()?;
    if row["resets_at_unix_ms"].as_u64()? <= now_unix_ms {
        return None;
    }
    Some(if row["window"] == "5-hour" { HOUR_MS } else { 24 * HOUR_MS })
}

/// Say, on a budget row, how much of the allowance will be used when it resets at the rate of its
/// recent past. `readings` are `(unix ms, remaining percent)` as the observation log holds them,
/// oldest first, over the span `forecast_basis_ms` named. Adds `forecast_used_percent` (at most
/// 100), `forecast_basis_minutes` and, when the allowance would run out first, `runs_out_at_unix_ms`,
/// and says so in `explain`. A row with too few readings, or none since its last reset, is left
/// as it was: no forecast is better than a guess from a minute of data.
pub fn add_forecast(row: &mut Value, readings: &[(u64, f64)], now_unix_ms: u64) {
    let Some(basis) = forecast_basis_ms(row, now_unix_ms) else { return };
    let (Some(remaining), Some(resets_at)) = (row["remaining_percent"].as_f64(), row["resets_at_unix_ms"].as_u64()) else {
        return;
    };
    // The log has one reading a minute, the row is live: a reset since the newest reading leaves
    // only the last period's spending behind, which says nothing about the new one.
    if readings.last().is_some_and(|(_, left)| remaining > left + RESET_JUMP_PERCENT) {
        return;
    }
    // Only what follows the last reset says anything about the next stretch.
    let from = readings
        .windows(2)
        .rposition(|pair| pair[1].1 > pair[0].1 + RESET_JUMP_PERCENT)
        .map_or(0, |at| at + 1);
    let used: Vec<(f64, f64)> = readings[from..]
        .iter()
        .filter(|(at, _)| *at <= now_unix_ms)
        .map(|(at, left)| (*at as f64, 100.0 - left))
        .collect();
    let (Some(first), Some(last)) = (used.first(), used.last()) else { return };
    let span_ms = (last.0 - first.0) as u64;
    if used.len() < MIN_FORECAST_READINGS || span_ms < basis / 4 {
        return;
    }
    // Least squares, so one noisy minute does not set the rate.
    let n = used.len() as f64;
    let (mean_t, mean_u) = (used.iter().map(|p| p.0).sum::<f64>() / n, used.iter().map(|p| p.1).sum::<f64>() / n);
    let spread: f64 = used.iter().map(|p| (p.0 - mean_t).powi(2)).sum();
    if spread == 0.0 {
        return;
    }
    let per_ms = (used.iter().map(|p| (p.0 - mean_t) * (p.1 - mean_u)).sum::<f64>() / spread).max(0.0);
    let used_now = 100.0 - remaining;
    let until_reset = (resets_at - now_unix_ms) as f64;
    let projected = clamp_percent(used_now + per_ms * until_reset);
    let basis_minutes = span_ms / MINUTE_MS;
    let runs_out_in = (per_ms > 0.0 && remaining > 0.0).then(|| remaining / per_ms).filter(|ms| *ms < until_reset);
    let said = match runs_out_in {
        Some(ms) => format!(
            " At the rate of the last {}, it runs out in about {} and will not last until it resets.",
            span_words(span_ms),
            span_words(ms as u64),
        ),
        None => format!(
            " At the rate of the last {}, {projected:.0}% will be used when it resets.",
            span_words(span_ms),
        ),
    };
    row["forecast_used_percent"] = json!(projected);
    row["forecast_basis_minutes"] = json!(basis_minutes);
    if let Some(ms) = runs_out_in {
        row["runs_out_at_unix_ms"] = json!(now_unix_ms + ms as u64);
    }
    if let Some(explain) = row["explain"].as_str() {
        row["explain"] = json!(format!("{explain}{said}"));
    }
}

/// A span in the unit a person reads: minutes under two hours, hours under two days, else days.
fn span_words(ms: u64) -> String {
    let minutes = ((ms + MINUTE_MS / 2) / MINUTE_MS).max(1);
    if minutes < 120 {
        plural(minutes, "minute")
    } else if minutes < 48 * 60 {
        plural(minutes / 60, "hour")
    } else {
        plural(minutes / (24 * 60), "day")
    }
}

/// The row named `id`, or the single row `id` is an unambiguous prefix of.
pub fn find<'a>(rows: &'a [Value], id: &str) -> Result<&'a Value, String> {
    if let Some(row) = rows.iter().find(|row| row["id"] == id) {
        return Ok(row);
    }
    let matches = rows
        .iter()
        .filter(|row| row["id"].as_str().is_some_and(|candidate| candidate.starts_with(id)))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("no objective or budget is named {id}; `st slo ls` lists them")),
        many => Err(format!(
            "{id} matches {}; name one",
            many.iter().filter_map(|row| row["id"].as_str()).collect::<Vec<_>>().join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(count: u64, p99: f64, over: u64) -> Value {
        json!({"count": count, "p50_ms": 1.0, "p99_ms": p99, "max_ms": p99 + 1.0,
            "over_target": over, "over_target_share": over as f64 / count.max(1) as f64})
    }

    fn windows(count: u64, p99: f64, over: u64) -> Value {
        json!({"1m": window(count, p99, over), "5m": window(count, p99, over), "1h": window(count, p99, over)})
    }

    fn limit(account: &str, account_ref: Option<&str>, weekly: Option<f64>, five: Option<f64>) -> AccountLimit {
        AccountLimit {
            account: account.into(),
            account_ref: account_ref.map(Into::into),
            identified: true,
            driver: "claude".into(),
            plan: None,
            five_hour_percent: five,
            five_hour_resets_at_unix_ms: Some(10_000_000 + 3 * 3_600_000),
            weekly_percent: weekly,
            weekly_resets_at_unix_ms: Some(10_000_000 + 3 * 86_400_000),
            measured_at_unix_ms: 10_000_000,
            measured_by: "agent/a".into(),
            host: "host".into(),
            seats: vec![],
            identity: None,
            windows: vec![],
        }
    }

    #[test]
    fn five_hour_only_harness_refresh_preserves_weekly_budget_and_its_clock() {
        use crate::LimitsStore;
        let store = crate::store::Store::open_memory("alder").unwrap();
        for fields in [
            json!({"driver":"claude", "account":"claude/example",
                "weekly_percent":40.0, "measured_at_unix_ms":1000}),
            json!({"driver":"claude", "account":"claude/example",
                "five_hour_percent":20.0, "measured_at_unix_ms":2000}),
        ] {
            store.append_claim(&crate::model::ClaimInput {
                subject:"agent/sample/harness".into(), actor:Some("agent/sample/harness".into()),
                kind:"harness.limits".into(), fields:serde_json::from_value(fields).unwrap(),
                evidence:vec![], expected_subject:None, idempotency_key:None,
            }).unwrap();
        }
        let limits = store.account_limits().unwrap();
        let report = rows(&json!({"targets":[]}), &limits, 2500, 1000);
        let weekly = by_id(&report, "claude/example/weekly");
        assert_eq!(weekly["remaining_percent"], 60.0);
        assert_eq!(weekly["status"], "stale");
        assert_eq!(weekly["measured_at_unix_ms"], 1000);
        let five = by_id(&report, "claude/example/5h");
        assert_eq!(five["remaining_percent"], 80.0);
        assert_eq!(five["status"], "met");
        assert_eq!(five["measured_at_unix_ms"], 2000);
    }

    #[test]
    fn quota_coverage_and_per_window_clocks_are_visible() {
        let mut account = limit("claude/example", None, None, None);
        account.windows = serde_json::from_value(json!([
            {"window":"weekly","used_percent":20.0,"resets_at_unix_ms":null,
             "measured_at_unix_ms":1000,"coverage":"reported","source":"omp-usage","host":"cedar"},
            {"window":"5h","used_percent":null,"resets_at_unix_ms":null,
             "measured_at_unix_ms":2000,"coverage":"unsupported","source":"omp-usage","host":"cedar"},
            {"window":"daily","used_percent":null,"resets_at_unix_ms":null,
             "measured_at_unix_ms":2000,"coverage":"unreported","source":"omp-usage","host":"cedar"}
        ])).unwrap();
        let report = rows(&json!({"targets":[]}), &[account], 2500, 1000);
        let weekly = by_id(&report, "claude/example/weekly");
        assert_eq!(weekly["status"], "stale");
        assert_eq!(weekly["source"], "omp-usage");
        assert_eq!(weekly["host"], "cedar");
        let five = by_id(&report, "claude/example/5h");
        assert_eq!(five["status"], "unsupported");
        assert_eq!(five["current"], "unsupported");
        assert!(five["remaining_percent"].is_null());
        assert_eq!(by_id(&report, "claude/example/daily")["status"], "no-data");
    }

    fn by_id<'a>(rows: &'a [Value], id: &str) -> &'a Value {
        rows.iter().find(|row| row["id"] == id).unwrap_or_else(|| panic!("no row {id}"))
    }

    #[test]
    fn a_p99_target_spends_a_one_percent_budget_over_the_hour() {
        let report = json!({"targets": [
            {"name": "person-read", "about": "Reads a person waits on", "p99_ms": 100,
             "windows": windows(200, 80.0, 1), "remote_p99_ms": 300, "remote_windows": windows(0, 0.0, 0)},
        ]});
        let rows = rows(&report, &[], 0, 0);
        let read = by_id(&rows, "person-read");
        assert_eq!(read["kind"], "objective");
        assert_eq!(read["target"], "p99 ≤ 100 ms");
        assert_eq!(read["window"], "1h");
        // One in 200 over is 0.5%: half the 1% allowance is gone.
        assert_eq!(read["remaining_percent"], 50.0);
        assert_eq!(read["status"], "met");
        assert!(read["explain"].as_str().unwrap().contains("50% of the error budget is left"));
        assert_eq!(by_id(&rows, "person-read/remote")["status"], "no-data");
    }

    #[test]
    fn a_target_missed_in_any_window_is_missed_and_its_budget_is_empty() {
        let mut slow = windows(100, 80.0, 0);
        slow["5m"] = window(100, 400.0, 5);
        let report = json!({"targets": [{"name": "write-ack", "about": "Writes", "p99_ms": 100, "windows": slow}]});
        let rows = rows(&report, &[], 0, 0);
        let row = by_id(&rows, "write-ack");
        assert_eq!(row["status"], "missed");
        assert!(row["explain"].as_str().unwrap().contains("missed in the 5m window"));
        // The hour itself is clean, so the budget over the hour is whole.
        assert_eq!(row["remaining_percent"], 100.0);
    }

    #[test]
    fn a_max_target_has_all_or_nothing_budgets_for_each_store_work() {
        let report = json!({"targets": [{"name": "transaction", "about": "Any transaction", "max_ms": 100,
            "read": windows(10, 4.0, 0), "write": windows(10, 9.0, 2), "writer_hold": windows(0, 0.0, 0)}]});
        let rows = rows(&report, &[], 0, 0);
        assert_eq!(by_id(&rows, "transaction/read")["remaining_percent"], 100.0);
        assert_eq!(by_id(&rows, "transaction/write")["remaining_percent"], 0.0);
        assert_eq!(by_id(&rows, "transaction/write")["status"], "missed");
        assert_eq!(by_id(&rows, "transaction/writer_hold")["status"], "no-data");
    }

    #[test]
    fn cpu_database_and_a_phone_share_read_against_their_own_allowances() {
        let report = json!({"targets": [
            {"name": "cpu", "about": "Member CPU", "max_cores": 1.0, "idle_max_cores": 0.5, "idle_min_minutes": 10,
             "windows": {"1m": {"cores": 0.5, "minutes": 1}, "5m": {"cores": 0.5, "minutes": 5}, "1h": {"cores": 0.25, "minutes": 60}},
             "minutes": 60, "idle": {"minutes": 20, "cores": 0.1, "not_idle": {"seat": 40}}},
            {"name": "database", "about": "The store", "max_live_gb": 8.0, "max_growth_mb_per_day": 300,
             "size": {"live_bytes": 2_000_000_000u64, "growth_bytes_per_day": 60_000_000i64}},
            {"name": "ios-live-share", "about": "Phone live share", "min_percent": 99,
             "population": "client-observed-foreground-ms", "windows": {
                "1m": {"over_target": 0, "foreground_ms": 0, "live_share": null},
                "5m": {"over_target": 0, "foreground_ms": 0, "live_share": null},
                "1h": {"over_target": 6_000, "foreground_ms": 1_000_000, "live_share": 0.994}}},
        ]});
        let rows = rows(&report, &[], 0, 0);
        assert_eq!(by_id(&rows, "cpu")["remaining_percent"], 75.0);
        assert_eq!(by_id(&rows, "cpu/idle")["remaining_percent"], 80.0);
        assert_eq!(by_id(&rows, "database")["remaining_percent"], 75.0);
        assert_eq!(by_id(&rows, "database")["status"], "met");
        // 0.6% disconnected of the 1% allowed leaves 40%.
        assert_eq!(by_id(&rows, "ios-live-share")["remaining_percent"], 40.0);
    }

    #[test]
    fn the_cpu_rows_are_met_missed_or_no_data_never_met_without_the_facts() {
        let target = |hour: Option<f64>, minutes: u64, idle_minutes: u64, idle: Option<f64>| {
            json!({"targets": [{"name": "cpu", "about": "Member CPU", "max_cores": 1.0, "idle_max_cores": 0.5,
                "idle_min_minutes": 10, "windows": {"1m": {"cores": 0.2, "minutes": 1},
                    "5m": {"cores": 0.2, "minutes": 5}, "1h": {"cores": hour, "minutes": minutes}},
                "minutes": minutes, "idle": {"minutes": idle_minutes, "cores": idle,
                    "not_idle": {"seat": 30, "start": 0, "deploy": 0, "catch-up": 2, "unknown": 1}}}]})
        };
        let both = |report: Value| {
            let rows = rows(&report, &[], 0, 0);
            (by_id(&rows, "cpu").clone(), by_id(&rows, "cpu/idle").clone())
        };
        let (hour, idle) = both(target(Some(0.4), 60, 27, Some(0.1)));
        assert_eq!((hour["status"].as_str(), hour["target"].as_str()), (Some("met"), Some("≤ 1 core over the hour")));
        assert_eq!((idle["status"].as_str(), idle["target"].as_str()), (Some("met"), Some("≤ 0.5 core over idle minutes")));
        assert_eq!(idle["current"], "0.10 cores over 27 idle minutes");
        let explain = idle["explain"].as_str().unwrap();
        for reason in ["seat 30", "catch-up 2", "unknown 1"] {
            assert!(explain.contains(reason), "{reason}: {explain}");
        }
        assert!(!explain.contains("deploy"), "{explain}");
        let (hour, idle) = both(target(Some(1.2), 60, 27, Some(0.6)));
        assert_eq!((hour["status"].as_str(), idle["status"].as_str()), (Some("missed"), Some("missed")));
        assert_eq!((hour["remaining_percent"].as_f64(), idle["remaining_percent"].as_f64()), (Some(0.0), Some(0.0)));
        // Too few idle minutes: unknown, however low.
        let (_, idle) = both(target(Some(0.4), 60, 9, Some(0.01)));
        assert_eq!((idle["status"].as_str(), idle["current"].as_str()), (Some("no-data"), Some("unknown: 9 idle minutes, needs 10")));
        assert_eq!(idle["remaining_percent"], Value::Null);
        // A daemon up less than the hour: both unknown.
        let (hour, idle) = both(target(None, 42, 30, Some(0.01)));
        assert_eq!((hour["status"].as_str(), idle["status"].as_str()), (Some("no-data"), Some("no-data")));
        assert_eq!(hour["current"], "unknown: 42 of 60 minutes known");
        assert_eq!(idle["current"], "unknown: 42 of 60 minutes held");
    }

    #[test]
    fn limits_become_weekly_and_five_hour_budgets_named_by_the_declared_account() {
        let now = 10_000_000 + 60_000;
        let limits = [
            limit("claude/abc", Some("ada/claude"), Some(40.0), Some(100.0)),
            limit("claude/def", None, None, None),
        ];
        let rows = rows(&json!({"targets": []}), &limits, now, 3_600_000);
        let weekly = by_id(&rows, "ada/claude/weekly");
        assert_eq!(weekly["kind"], "budget");
        assert_eq!(weekly["remaining_percent"], 60.0);
        assert_eq!(weekly["status"], "met");
        assert!(weekly["explain"].as_str().unwrap().contains("resets in 2 days"));
        let five = by_id(&rows, "ada/claude/5h");
        assert_eq!(five["remaining_percent"], 0.0);
        assert_eq!(five["status"], "missed");
        assert!(five["explain"].as_str().unwrap().contains("resets in 2 hours"));
        assert_eq!(by_id(&rows, "claude/def/weekly")["status"], "no-data");
    }

    /// One reading a minute for the `minutes` before `now`, oldest first, each the remaining
    /// percent `left(minutes_ago)`.
    fn minutely(now: u64, minutes: u64, left: impl Fn(u64) -> f64) -> Vec<(u64, f64)> {
        (0..=minutes).rev().map(|ago| (now - ago * 60_000, left(ago))).collect()
    }

    fn five_hour_budget(used: f64, now: u64) -> Value {
        let rows = rows(&json!({"targets": []}), &[limit("claude/abc", None, Some(used), Some(used))], now, 3_600_000);
        by_id(&rows, "claude/abc/5h").clone()
    }

    #[test]
    fn a_budget_spent_fast_says_when_it_runs_out_before_its_reset() {
        let now = 10_000_000 + 60_000;
        let mut row = five_hour_budget(40.0, now);
        // 1% a minute for twenty minutes: 60% is left, so it lasts about an hour, not three.
        add_forecast(&mut row, &minutely(now, 20, |ago| 60.0 + ago as f64), now);
        assert_eq!(row["forecast_used_percent"], 100.0);
        assert_eq!(row["forecast_basis_minutes"], 20);
        let out = row["runs_out_at_unix_ms"].as_u64().unwrap();
        assert!(out.abs_diff(now + 3_600_000) < 60_000, "{out}");
        let explain = row["explain"].as_str().unwrap();
        assert!(explain.contains("it runs out in about 60 minutes"), "{explain}");
        assert!(explain.contains("the last 20 minutes"), "{explain}");
    }

    #[test]
    fn a_budget_spent_slowly_forecasts_its_use_at_the_reset() {
        let now = 10_000_000 + 60_000;
        let mut row = five_hour_budget(40.0, now);
        // 0.1% a minute; the reset is 179 minutes away: 40% + about 18%.
        add_forecast(&mut row, &minutely(now, 20, |ago| 60.0 + ago as f64 * 0.1), now);
        assert_eq!(row["forecast_used_percent"], 57.9);
        assert!(row.get("runs_out_at_unix_ms").is_none(), "{row}");
        assert!(row["explain"].as_str().unwrap().contains("58% will be used when it resets"), "{row}");
    }

    #[test]
    fn a_budget_not_spent_at_all_forecasts_what_is_used_now() {
        let now = 10_000_000 + 60_000;
        let mut row = five_hour_budget(40.0, now);
        add_forecast(&mut row, &minutely(now, 20, |_| 60.0), now);
        assert_eq!(row["forecast_used_percent"], 40.0);
        assert!(row.get("runs_out_at_unix_ms").is_none(), "{row}");
    }

    #[test]
    fn the_rate_is_read_only_from_after_the_last_reset() {
        let now = 10_000_000 + 60_000;
        let mut row = five_hour_budget(1.0, now);
        // Ten minutes at 5% a minute down to nothing, a reset, then 0.04% a minute: only the
        // second stretch counts, so about 1% + 7% is used at the next reset.
        let mut readings = minutely(now - 17 * 60_000, 9, |ago| 5.0 + ago as f64 * 5.0);
        readings.extend(minutely(now, 16, |ago| 99.0 + ago as f64 * 0.04));
        add_forecast(&mut row, &readings, now);
        assert_eq!(row["forecast_used_percent"], 8.2);
        assert_eq!(row["forecast_basis_minutes"], 16);
        assert!(row.get("runs_out_at_unix_ms").is_none(), "{row}");
    }

    #[test]
    fn a_reset_since_the_newest_reading_leaves_no_forecast_until_there_is_history_after_it() {
        let now = 10_000_000 + 60_000;
        // Twenty minutes at 1% a minute down to 1% left, then the live row reads a fresh allowance.
        let mut row = five_hour_budget(0.5, now);
        add_forecast(&mut row, &minutely(now - 60_000, 20, |ago| 1.0 + ago as f64 * 4.0), now);
        for field in ["forecast_used_percent", "forecast_basis_minutes", "runs_out_at_unix_ms"] {
            assert!(row.get(field).is_none(), "{field}: {row}");
        }
        assert!(!row["explain"].as_str().unwrap().contains("At the rate"), "{row}");
    }

    #[test]
    fn too_little_history_or_a_row_without_a_current_reading_has_no_forecast() {
        let now = 10_000_000 + 60_000;
        let mut few = five_hour_budget(40.0, now);
        add_forecast(&mut few, &minutely(now, 3, |ago| 60.0 + ago as f64), now);
        assert!(few.get("forecast_used_percent").is_none(), "{few}");
        // Five readings a minute apart cover too short a span to set a rate.
        let mut short = five_hour_budget(40.0, now);
        add_forecast(&mut short, &minutely(now, 4, |ago| 60.0 + ago as f64), now);
        assert!(short.get("forecast_used_percent").is_none(), "{short}");
        let mut stale = five_hour_budget(40.0, now + 7_200_000);
        assert_eq!(stale["status"], "stale");
        add_forecast(&mut stale, &minutely(now, 20, |ago| 60.0 + ago as f64), now + 7_200_000);
        assert!(stale.get("forecast_used_percent").is_none(), "{stale}");
        let mut none = five_hour_budget(40.0, now);
        add_forecast(&mut none, &[], now);
        assert!(none.get("forecast_used_percent").is_none(), "{none}");
        let mut objective = p99("sql-statement", "x", "requests", Some(10.0), &windows(100, 5.0, 0));
        assert_eq!(forecast_basis_ms(&objective, now), None);
        add_forecast(&mut objective, &minutely(now, 20, |ago| 60.0 + ago as f64), now);
        assert!(objective.get("forecast_used_percent").is_none(), "{objective}");
    }

    #[test]
    fn a_weekly_budget_reads_its_rate_from_the_last_day() {
        let now = 10_000_000 + 60_000;
        let rows = rows(&json!({"targets": []}), &[limit("claude/abc", None, Some(10.0), Some(10.0))], now, 3_600_000);
        let weekly = by_id(&rows, "claude/abc/weekly");
        assert_eq!(forecast_basis_ms(weekly, now), Some(24 * 3_600_000));
        assert_eq!(forecast_basis_ms(by_id(&rows, "claude/abc/5h"), now), Some(3_600_000));
    }

    #[test]
    fn an_old_reading_is_stale_and_says_so() {
        let limits = [limit("claude/abc", None, Some(10.0), Some(10.0))];
        let rows = rows(&json!({"targets": []}), &limits, 10_000_000 + 7_200_000, 3_600_000);
        let row = by_id(&rows, "claude/abc/weekly");
        assert_eq!(row["status"], "stale");
        assert!(row["explain"].as_str().unwrap().contains("not recent"));
    }

    #[test]
    fn find_takes_an_exact_id_or_one_unambiguous_prefix() {
        let rows = vec![json!({"id": "write-ack"}), json!({"id": "transaction/read"}), json!({"id": "transaction/write"})];
        assert_eq!(find(&rows, "write-ack").unwrap()["id"], "write-ack");
        assert_eq!(find(&rows, "write").unwrap()["id"], "write-ack");
        assert!(find(&rows, "transaction").unwrap_err().contains("transaction/read, transaction/write"));
        assert!(find(&rows, "nope").unwrap_err().contains("st slo ls"));
    }

    #[test]
    fn the_live_report_yields_a_row_for_every_target_in_the_targets_file() {
        let report = crate::api::request_latency_target_windows();
        let rows = rows(&report, &[], 0, 0);
        for latency in &crate::slo::targets().latency {
            assert!(rows.iter().any(|row| row["id"] == latency.name.as_str()), "{} is missing", latency.name);
        }
        for id in ["sql-statement", "transaction/read", "cpu", "cpu/idle", "database"] {
            assert!(rows.iter().any(|row| row["id"] == id), "{id}");
        }
        let mut ids = rows.iter().map(|row| row["id"].as_str().unwrap()).collect::<Vec<_>>();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), rows.len(), "ids are unique");
    }

    #[test]
    fn objectives_come_before_budgets() {
        let report = crate::api::request_latency_target_windows();
        let rows = rows(&report, &[limit("claude/abc", None, Some(1.0), Some(1.0))], 10_000_000, 3_600_000);
        let first_budget = rows.iter().position(|row| row["kind"] == "budget").unwrap();
        assert!(first_budget > 0);
        assert!(rows[..first_budget].iter().all(|row| row["kind"] == "objective"));
        assert!(rows[first_budget..].iter().all(|row| row["kind"] == "budget"));
    }

    #[test]
    fn a_percentile_is_the_nearest_rank() {
        let samples = (1..=20).collect::<Vec<u64>>();
        assert_eq!(nearest_rank(&samples, 95), 19);
        assert_eq!(nearest_rank(&samples, 50), 10);
        assert_eq!(nearest_rank(&samples, 99), 20);
        assert_eq!(nearest_rank(&[7], 95), 7);
        assert_eq!(nearest_rank(&[3, 1, 2], 50), 2);
    }

    #[test]
    fn a_duration_reads_in_minutes_from_a_minute_and_seconds_below() {
        assert_eq!(duration(600_000), "10 min");
        assert_eq!(duration(780_000), "13 min");
        assert_eq!(duration(90_000), "1.5 min");
        assert_eq!(duration(55_000), "55 s");
        assert_eq!(duration(1_500), "1.5 s");
        assert_eq!(duration(400), "400 ms");
    }
}
