//! The part of `store::limits` that moved up from fractal-store.
#![allow(unused_imports)]  // the imports serve the modules below through `use super::*`
use crate::AccountsStore;
use super::accounts::limit_binding_is_current;
use super::*;
use crate::model::DoctorCheck;
use serde::Deserialize;
pub use fractal_missions::store::limits::*;
pub use fractal_missions::store::limits::{ACCOUNT_READING_WINDOW_MS, AccountLimit, LIMITS_CATCH_UP_PAGE, account_limits_at, create_limits_schema, flush_limits, flush_limits_page, open_limits, reading, refold_limits_tx};

/// The actor st records for what its limits policy does.
pub const LIMITS_ACTOR: &str = "daemon/limits";

/// Bound accounts remain distinct even when an older producer reports a generic provider label.
pub fn episode_account(limit: &AccountLimit) -> String {
    limit
        .account_ref
        .as_ref()
        .map_or_else(|| limit.account.clone(), |name| format!("account/{name}"))
}

/// One node's `[limits]` policy.
#[derive(Clone, Debug)]
pub struct LimitsPolicy {
    pub stop_at_weekly_percent: u32,
    pub keep: BTreeSet<String>,
    /// Accounts never stopped: a declared account name or a provider label.
    pub exempt_accounts: BTreeSet<String>,
    /// Harnesses never stopped, whatever their account.
    pub exempt_harnesses: BTreeSet<String>,
    pub notify: String,
    pub fresh_ms: u64,
}

impl LimitsPolicy {
    /// The policy a `[limits]` section describes, or `None` while it is disabled.
    pub fn from_config(config: &crate::config::LimitsConfig) -> Option<Self> {
        if !config.enabled {
            return None;
        }
        Some(Self {
            stop_at_weekly_percent: config.stop_at_weekly_percent,
            keep: config.keep.iter().cloned().collect(),
            exempt_accounts: config.exempt_accounts.iter().cloned().collect(),
            exempt_harnesses: config.exempt_harnesses.iter().cloned().collect(),
            notify: config.notify.clone()?,
            fresh_ms: config.fresh_ms().ok()?,
        })
    }

    /// Whether this account's seats are never stopped, whenever they started: it is exempt by
    /// the declared name, the provider's label, or its harness.
    pub fn exempts(&self, limit: &AccountLimit) -> bool {
        self.exempt_harnesses.contains(&limit.driver)
            || self.exempt_accounts.contains(&limit.account)
            || limit
                .account_ref
                .as_ref()
                .is_some_and(|name| self.exempt_accounts.contains(name))
            || self.exempt_accounts.contains(&episode_account(limit))
            || limit
                .identity
                .as_ref()
                .is_some_and(|label| self.exempt_accounts.contains(label))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct LimitsOutcome {
    /// Seats this node stopped in this pass.
    pub stopped: Vec<String>,
    /// Pooled seats this node restarted on another account in their pool, with that account.
    pub switched: Vec<(String, String)>,
    /// Operations messages this pass made or found.
    pub notified: Vec<String>,
}

/// The host a seat is declared on, when its current declaration is an agent and not a stop.
fn seat_host(connection: &Connection, seat: &str) -> Result<Option<String>> {
    let row = connection
        .query_row(
            "SELECT kind, json_extract(member, '$.host') FROM desired WHERE subject=?1",
            [seat],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()?;
    Ok(row.and_then(|(kind, host)| (kind == "agent").then_some(host).flatten()))
}

fn utc(unix_ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(unix_ms as i64).map_or_else(
        || unix_ms.to_string(),
        |at| at.format("%Y-%m-%d %H:%M UTC").to_string(),
    )
}

pub trait LimitsStore {
    /// Configured omp-usage evidence, associated with seats by their current login binding.
    /// Returns nothing until the projection has caught up after an upgrade.
    fn account_limits(&self) -> Result<Vec<AccountLimit>>;
    /// Whether the observed quota projection has caught up at least once.
    fn account_limits_ready(&self) -> Result<bool>;
    /// Fold one page of `usage.limits.observed` claims into the projection, in its own short writer
    /// transaction, and say whether more remain. The daemon calls this until it returns false;
    /// the cursor is stored, so a restart resumes where it stopped.
    fn catch_up_account_limits(&self, page: usize) -> Result<bool>;
    /// Missing quota evidence is unknown, never evidence that an account is below its limit.
    /// Keep the last reading visible and report unavailable weekly evidence for active seats.
    fn account_limits_check(&self, now: u128, fresh_ms: u64) -> Result<DoctorCheck>;
    /// Stop the seats this node hosts on every account whose fresh weekly reading reached the
    /// policy's percentage, and notify operations once per weekly window. Each seat is
    /// stopped at most once per window, so a person can start it again.
    fn enforce_account_limits(
            &self,
            policy: &LimitsPolicy,
            now: u128,
        ) -> Result<LimitsOutcome, FractalError>;
    /// The usage of a period against the declared accounts: every row and limit, and each
    /// account's credits used so far in the calendar month of `until_ms`.
    fn attributed_usage(&self, since_ms: u64, until_ms: u64) -> Result<super::usage_accounts::AttributedUsage>;
    /// Which of `seats` are declared agents now. `[limits] keep` lists seats by name and nothing
    /// removes a retired seat from it, so this lets a report say which entries keep nothing.
    fn declared_seats(&self, seats: &[String]) -> Result<BTreeSet<String>>;
}

impl LimitsStore for Store {
    fn declared_seats(&self, seats: &[String]) -> Result<BTreeSet<String>> {
        let connection = self.readers.get();
        let mut statement =
            connection.prepare_cached("SELECT 1 FROM desired WHERE subject=?1 AND kind='agent'")?;
        let mut declared = BTreeSet::new();
        for seat in seats {
            if statement.exists([seat])? {
                declared.insert(seat.clone());
            }
        }
        Ok(declared)
    }

    fn attributed_usage(&self, since_ms: u64, until_ms: u64) -> Result<super::usage_accounts::AttributedUsage> {
        super::usage_accounts::attributed_usage(self, since_ms, until_ms)
    }

    fn account_limits(&self) -> Result<Vec<AccountLimit>> {
        if !self.account_limits_ready()? {
            return Ok(Vec::new());
        }
        let connection = self.readers.get();
        let mut limits = account_limits_for_writers(&connection, &self.usage_limits_writers())?;
        let mut statement = connection.prepare_cached("SELECT subject FROM desired WHERE kind='agent' ORDER BY subject")?;
        let seats = statement.query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for limit in &mut limits {
            for seat in &seats {
                if limit_binding_is_current(&connection, &self.origin, seat, limit)? {
                    limit.seats.push(seat.clone());
                }
            }
        }
        Ok(limits)
    }

    fn account_limits_ready(&self) -> Result<bool> {
        Ok(self.readers.get().query_row(
            "SELECT EXISTS(SELECT 1 FROM meta WHERE key=?1)",
            [LIMITS_READY],
            |row| row.get(0),
        )?)
    }

    fn catch_up_account_limits(&self, page: usize) -> Result<bool> {
        let page = page.max(1);
        let mut connection = self.connection.write();
        let transaction = connection.transaction()?;
        let folded = flush_limits_page(&transaction, page)?;
        transaction.commit()?;
        Ok(folded == page)
    }

    fn account_limits_check(&self, now: u128, fresh_ms: u64) -> Result<DoctorCheck> {
        if !self.account_limits_ready()? {
            return Ok(DoctorCheck {
                name: "account-limits".into(),
                status: "pass".into(),
                message: "the account limits projection is catching up after an upgrade; no limits decision is made until it has".into(),
            });
        }
        let limits = self.account_limits()?;
        let connection = self.readers.get();
        let mut statement = connection.prepare_cached(
            "SELECT subject, body FROM desired WHERE kind='agent' ORDER BY subject",
        )?;
        let mut issues = BTreeSet::new();
        let mut active_accounts = BTreeSet::new();
        for row in statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })? {
            let (seat, body) = row?;
            let desired: Value = serde_json::from_str(&body)?;
            let binding = crate::accounts::harness_binding(&desired);
            let mut reading = None;
            for limit in &limits {
                if let Some(binding) = &binding
                    && let crate::accounts::Binding::Account(name) = &binding.binding
                {
                    if limit.driver == binding.driver
                        && limit.account_ref.as_ref() == Some(name)
                    {
                        reading = Some(limit);
                        break;
                    }
                    continue;
                }
                if limit.seats.contains(&seat)
                    && limit_binding_is_current(&connection, &self.origin, &seat, limit)?
                {
                    reading = Some(limit);
                    break;
                }
            }
            if let Some(reading) = reading {
                active_accounts.insert(episode_account(reading));
            } else {
                let driver = desired["children"]
                    .as_array()
                    .and_then(|children| children.iter().find(|child| child["name"] == "harness"))
                    .and_then(|harness| harness["arguments"][0].as_str());
                if matches!(driver, Some("claude" | "codex")) {
                    let account = match binding.map(|binding| binding.binding) {
                        Some(crate::accounts::Binding::Account(name)) => format!("account/{name}"),
                        _ => seat,
                    };
                    issues.insert(format!("{account}: no weekly reading"));
                }
            }
        }
        for limit in limits {
            let account = episode_account(&limit);
            if !active_accounts.contains(&account)
                || !matches!(limit.driver.as_str(), "claude" | "codex")
            {
                continue;
            }
            let reason = if limit.weekly_window(now, fresh_ms).is_none() {
                Some("weekly evidence is unavailable (missing, unreported, expired or stale)")
            } else {
                None
            };
            if let Some(reason) = reason {
                issues.insert(format!("{account}: {reason}"));
            }
        }
        Ok(DoctorCheck {
            name: "account-limits".into(),
            status: if issues.is_empty() { "pass" } else { "warn" }.into(),
            message: if issues.is_empty() {
                "active accounts have current weekly evidence, or use a harness without weekly limits".into()
            } else {
                format!(
                    "{}. Unavailable evidence is not below the limit. Automatic stops wait for fresh weekly evidence; inspect the provider login and quota reporting. The last source reading remains visible in `st usage --json`.",
                    issues.into_iter().collect::<Vec<_>>().join("; ")
                )
            },
        })
    }

    fn enforce_account_limits(
            &self,
            policy: &LimitsPolicy,
            now: u128,
        ) -> Result<LimitsOutcome, FractalError> {
        if !policy.notify.starts_with("agent/")
            || policy.notify.split('/').skip(1).any(str::is_empty)
            || policy.notify.chars().any(char::is_whitespace)
        {
            return Err(FractalError::new(
                "invalid-limits-recipient",
                "limits notifications require an operations agent",
            ));
        }
        let mut outcome = LimitsOutcome::default();
        // Not yet known is not "below the limit" nor "unused": decide nothing until it is.
        if !self.account_limits_ready().map_err(internal)? {
            return Ok(outcome);
        }
        for limit in self.account_limits().map_err(internal)? {
            let Some(window) = limit.weekly_window(now, policy.fresh_ms) else {
                continue;
            };
            let weekly = window.used_percent.expect("available quota has a percentage");
            if policy.exempts(&limit) || weekly < f64::from(policy.stop_at_weekly_percent) {
                continue;
            }
            let episode_account = episode_account(&limit);
            let episode = window.resets_at_unix_ms.expect("available quota has a reset").to_string();
            // A pooled seat moves to another account in its pool, so the stop becomes a switch;
            // every other seat on the account stops.
            let mut targets = Vec::new();
            let mut local = Vec::new();
            let mut local_switch = Vec::new();
            {
                let connection = self.readers.get();
                for seat in &limit.seats {
                    if policy.keep.contains(seat) {
                        continue;
                    }
                    let Some(host) = seat_host(&connection, seat).map_err(internal)? else {
                        continue;
                    };
                    if crate::suspension::current(self, seat)
                        .map_err(internal)?
                        .is_some_and(|suspension| suspension.holds_seat())
                    {
                        continue;
                    }
                    if !super::accounts::limit_binding_is_current(
                        &connection,
                        &self.origin,
                        seat,
                        &limit,
                    )
                    .map_err(internal)?
                    {
                        continue;
                    }
                    let binding = self.seat_binding(seat).map_err(internal)?;
                    let mut alternative = None;
                    if let Some(binding) = &binding {
                        // A pooled seat's reading names the account it ran on. A reading without
                        // one predates the binding, so it says nothing about this seat.
                        let Some(leaving) = limit.account_ref.as_deref() else {
                            continue;
                        };
                        if matches!(&binding.binding, crate::accounts::Binding::Account(name) if name != leaving)
                        {
                            continue;
                        }
                        alternative = self
                            .pool_alternatives(
                                binding,
                                &host,
                                leaving,
                                f64::from(policy.stop_at_weekly_percent),
                                now,
                            )
                            .map_err(internal)?
                            .into_iter()
                            .next();
                    }
                    if alternative.is_none() {
                        targets.push(seat.clone());
                    }
                    let handled_before: bool = connection
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM local_limit_stops
                             WHERE account=?1 AND episode=?2 AND seat=?3)",
                            params![episode_account, episode, seat],
                            |row| row.get(0),
                        )
                        .map_err(internal)?;
                    if host == self.origin && !handled_before {
                        match alternative {
                            Some(account) => local_switch.push((seat.clone(), account)),
                            None => local.push(seat.clone()),
                        }
                    }
                }
            }
            for (seat, account) in local_switch {
                if self.switch_seat_account(&seat, &limit, &account, &episode, now)? {
                    outcome.switched.push((seat, account));
                }
            }
            let mut stopped_here = Vec::new();
            for seat in local {
                let kdl = format!("version 2\nstop {seat:?}\n");
                let intent = crate::graph::parse_internal_intent(&kdl, &self.origin)?;
                let planned = self.mission(
                    &intent,
                    IntentInput {
                        kdl: kdl.clone(),
                        source_name: Some("st limits".into()),
                    },
                )?;
                self.apply_as(
                    &intent,
                    &planned.subject_tokens,
                    &format!("limits-stop:{episode_account}:{episode}:{seat}"),
                    Some(LIMITS_ACTOR),
                )?;
                self.connection
                    .batched(|tx| {
                        tx.execute(
                            /* sqlite-large-value: allow-for-now lv-h604d12b1d064 owner=fractal_seats::store::limits TODO=bound-or-reference-values-and-reclamation */ "INSERT OR IGNORE INTO local_limit_stops(account, episode, seat, stopped_at_unix_ms)
                             VALUES (?1, ?2, ?3, ?4)",
                            params![episode_account, episode, seat, now as i64],
                        )
                    })
                    .map_err(internal)?
                    .map_err(internal)?;
                stopped_here.push(seat.clone());
                outcome.stopped.push(seat);
            }
            // The node that measured the reading notifies operations, so one node writes.
            if targets.is_empty() || window.host != self.origin {
                continue;
            }
            let provider = limit.account.split('/').next().unwrap_or("").to_owned();
            let provider = provider
                .chars()
                .next()
                .map(|first| {
                    first.to_uppercase().collect::<String>() + &provider[first.len_utf8()..]
                })
                .unwrap_or_default();
            let plural = if targets.len() == 1 { "" } else { "s" };
            let kept = if policy.keep.is_empty() {
                String::new()
            } else {
                format!(
                    " except {}",
                    policy.keep.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            };
            let reset = format!("It resets {}.", utc(window.resets_at_unix_ms.expect("available quota has a reset")));
            let title = format!(
                "{provider} account {} is at {weekly:.0}% of its weekly limit: {} affected seat{plural}",
                limit.account,
                targets.len()
            );
            let reason = format!(
                "{} reached {weekly:.0}% of its weekly limit ({} measured it at {}). The policy stops \
                 seats that use it{kept}. Affected seats across the fleet: {}. This pass stopped \
                 {} on {}. Other hosts enforce their own stops. {reset} A seat restarted by a \
                 person stays running until the reset. Review the account and stopped seats, group related alerts, and act \
                 on standing instructions. Ask a person only if a decision is needed, using a \
                 structured request with your recommendation, reasons and proposed action.",
                limit.account,
                limit.measured_by,
                utc(window.measured_at_unix_ms),
                targets.join(", "),
                if stopped_here.is_empty() {
                    "none".into()
                } else {
                    stopped_here.join(", ")
                },
                self.origin,
            );
            // The payload is pinned by account/window. A later reading must not turn a retry
            // into an idempotency conflict or recreate an already handled notification.
            let key = format!("limits-notify:{episode_account}:{episode}");
            let subject = format!(
                "message/limits-{}",
                &canonical_hash(&key).map_err(internal)?[..20]
            );
            let notice = ClaimInput {
                subject: subject.clone(),
                kind: "message.sent".into(),
                actor: Some(LIMITS_ACTOR.into()),
                fields: serde_json::from_value(json!({
                    "from": LIMITS_ACTOR, "to": policy.notify,
                    "title": title, "content": reason, "status": "sent",
                    "tags": ["account-limits", format!("account:{}", limit.account), format!("window:{episode}")]
                })).map_err(internal)?,
                evidence: Vec::new(),
                expected_subject: None,
                idempotency_key: None,
            };
            // Where its recipient's host reads it, decided before the transaction: a notice
            // routed to the stream is queued in the outbox, which drains to the shard.
            let route = fractal_store::store::message_routing::legacy_reason(self, &notice).map_err(internal)?;
            self.connection.batched(|tx| {
                // Sent as a graph claim, a mirrored stream item or a queued outbox row.
                let recorded = fractal_store::store::message_sent::message_sent_tx(tx, &subject).map_err(internal)?;
                if !recorded {
                    fractal_store::store::message_routing::send_message_tx(tx, &self.origin, &notice, route)?;
                }
                Ok::<_, FractalError>(())
            }).map_err(internal)??;
            outcome.notified.push(subject);
        }
        Ok(outcome)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    const HOUR: u128 = 3_600_000;
    const NOW: u128 = 1_800_000_000_000;
    const WRITER: &str = "agent/alder.usage-producer";
    const SINGLE: &str = "agent/alder.single";
    const POOLED: &str = "agent/alder.pooled";

    fn policy() -> LimitsPolicy {
        LimitsPolicy {
            stop_at_weekly_percent: 95,
            keep: BTreeSet::new(),
            exempt_accounts: BTreeSet::new(),
            exempt_harnesses: BTreeSet::new(),
            notify: "agent/alder.operations".into(),
            fresh_ms: HOUR as u64,
        }
    }

    fn declare_accounts(store: &Store) {
        let intent = crate::graph::parse_internal_intent(
            r#"version 2
account "ada/one" { provider "openai"; owner "person/ada"; login "/srv/logins/one"; }
account "ada/two" { provider "openai"; owner "person/ada"; login "/srv/logins/two"; }
"#,
            "alder",
        )
        .unwrap();
        store.apply_internal(&intent, "declare-accounts").unwrap();
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
    }

    fn declare_seat(store: &Store, name: &str, binding: &str, host: &str) {
        let intent = crate::graph::parse_internal_intent(
            &format!(
                "version 2\nagent {name:?} {{ workspace \"/tmp\"; harness \"codex\" {{ {binding}; }}; }}"
            ),
            "alder",
        )
        .unwrap();
        store
            .apply_internal(&intent, &format!("declare-{name}-{binding}"))
            .unwrap();
        store
            .connection
            .write()
            .execute(
                "UPDATE desired SET member=json_object('host', ?2) WHERE subject=?1",
                params![format!("agent/alder.{name}"), host],
            )
            .unwrap();
    }

    fn live(store: &Store, seat: &str) -> bool {
        seat_host(&store.readers.get(), seat).unwrap().is_some()
    }

    fn timestamp(at: u128) -> String {
        chrono::DateTime::from_timestamp_millis(at as i64)
            .unwrap()
            .to_rfc3339()
    }

    fn quota(account: &str, window: &str, used: Option<f64>, measured: u128) -> Value {
        json!({
            "account": fractal_drivers::account::account_label(
                "codex", &format!("declared:{account}")
            ),
            "account_ref": account,
            "provider": "openai-codex",
            "window": window,
            "used_percent": used,
            "resets_at": timestamp(NOW + 7 * 24 * HOUR),
            "measured_at": timestamp(measured),
            "coverage": "reported",
            "source": "omp-usage",
            "host": "alder"
        })
    }

    fn observe_as(store: &Store, writer: &str, readings: Vec<Value>) {
        store
            .append_claim(&ClaimInput {
                subject: writer.into(),
                actor: Some(writer.into()),
                kind: "usage.limits.observed".into(),
                fields: BTreeMap::from([
                    ("readings".into(), json!(readings)),
                    ("credit_readings".into(), json!([])),
                ]),
                evidence: vec![],
                expected_subject: None,
                idempotency_key: None,
            })
            .unwrap();
    }

    fn observe(store: &Store, readings: Vec<Value>) {
        observe_as(store, WRITER, readings);
    }

    fn single_store() -> Store {
        let store = Store::open_memory("alder").unwrap();
        declare_accounts(&store);
        declare_seat(&store, "single", "account \"ada/one\"", "alder");
        store
    }

    fn pooled_store() -> Store {
        let store = single_store();
        declare_seat(&store, "pooled", "account-pool \"person/ada\"", "alder");
        store.choose_seat_account(POOLED, "ada/one", NOW).unwrap();
        store
    }

    #[test]
    fn only_configured_omp_usage_writers_supply_enforcement_evidence() {
        let store = single_store();
        let excluded_writer = "agent/alder.unconfigured";
        let excluded = ClaimInput {
            subject: excluded_writer.into(),
            actor: Some(excluded_writer.into()),
            kind: "usage.limits.observed".into(),
            fields: BTreeMap::from([
                ("readings".into(), json!([quota("ada/one", "weekly", Some(100.0), NOW)])),
                ("credit_readings".into(), json!([])),
            ]),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        };
        assert!(store.append_claim(&excluded).is_err(), "unconfigured writers cannot append");
        // Admit the evidence while configured, then exclude it without deleting the claim.
        store.set_usage_limits_writers(vec![WRITER.into(), excluded_writer.into()]).unwrap();
        let retained = store.append_claim(&excluded).unwrap();
        assert_eq!(store.account_limits().unwrap()[0].measured_by, excluded_writer);
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        assert_eq!(
            store.claims_for(excluded_writer, Some("usage.limits.observed")).unwrap()[0].id,
            retained.id
        );
        assert!(store.account_limits().unwrap().is_empty());
        assert_eq!(
            store.enforce_account_limits(&policy(), NOW).unwrap(),
            LimitsOutcome::default()
        );
        observe(&store, vec![quota("ada/one", "weekly", Some(95.0), NOW)]);
        let limits = store.account_limits().unwrap();
        assert_eq!(limits[0].measured_by, WRITER);
        assert_eq!(limits[0].seats, [SINGLE]);
        assert_eq!(limits[0].windows[0].source, "omp-usage");
        assert_eq!(
            store.enforce_account_limits(&policy(), NOW).unwrap().stopped,
            [SINGLE]
        );
    }

    #[test]
    fn latest_weekly_value_wins_in_both_directions_not_the_recent_maximum() {
        for (older, newer, stops) in [(99.0, 20.0, false), (20.0, 99.0, true)] {
            let store = single_store();
            observe(&store, vec![quota("ada/one", "weekly", Some(older), NOW - 2)]);
            observe(&store, vec![quota("ada/one", "weekly", Some(newer), NOW)]);
            // Arrival order cannot replace newer source evidence with an older maximum.
            observe(&store, vec![quota("ada/one", "weekly", Some(100.0), NOW - 1)]);
            let limits = store.account_limits().unwrap();
            let weekly = limits[0].weekly_window(NOW, HOUR as u64).unwrap();
            assert_eq!(weekly.used_percent, Some(newer));
            assert_eq!(weekly.measured_at_unix_ms, NOW as u64);
            let outcome = store.enforce_account_limits(&policy(), NOW).unwrap();
            assert_eq!(outcome.stopped, if stops { vec![SINGLE] } else { vec![] });
            assert_eq!(live(&store, SINGLE), !stops);
        }
    }

    #[test]
    fn one_hour_is_inclusive_but_one_millisecond_later_never_enforces() {
        for (age, horizon, stops) in [
            (HOUR, HOUR, true),
            (HOUR + 1, HOUR, false),
            (HOUR + 1, 24 * HOUR, false),
            (60_000, 60_000, true),
            (60_001, 60_000, false),
        ] {
            let store = single_store();
            observe(&store, vec![quota("ada/one", "weekly", Some(99.0), NOW - age)]);
            let configured = LimitsPolicy { fresh_ms: horizon as u64, ..policy() };
            let outcome = store.enforce_account_limits(&configured, NOW).unwrap();
            assert_eq!(outcome.stopped, if stops { vec![SINGLE] } else { vec![] });
            assert_eq!(live(&store, SINGLE), !stops);
        }
    }

    #[test]
    fn newer_unreported_gap_missing_reset_and_expired_evidence_do_not_enforce() {
        for case in ["unreported", "gap", "missing-reset", "expired", "future"] {
            let store = single_store();
            observe(&store, vec![quota("ada/one", "weekly", Some(99.0), NOW - 1)]);
            let mut unavailable = quota("ada/one", "weekly", Some(100.0), NOW);
            match case {
                "unreported" => {
                    unavailable["coverage"] = json!("unreported");
                    unavailable["used_percent"] = Value::Null;
                }
                "gap" => {
                    unavailable["coverage"] = json!("unsupported");
                    unavailable["used_percent"] = Value::Null;
                }
                "missing-reset" => unavailable["resets_at"] = Value::Null,
                "expired" => unavailable["resets_at"] = json!(timestamp(NOW)),
                "future" => unavailable["measured_at"] = json!(timestamp(NOW + 60_001)),
                _ => unreachable!(),
            }
            observe(&store, vec![unavailable]);
            let limits = store.account_limits().unwrap();
            assert!(limits[0].weekly_window(NOW, HOUR as u64).is_none(), "{case}");
            assert_eq!(
                store.enforce_account_limits(&policy(), NOW).unwrap(),
                LimitsOutcome::default(),
                "{case}"
            );
            assert!(live(&store, SINGLE));
            let check = store.account_limits_check(NOW, HOUR as u64).unwrap();
            assert_eq!(check.status, "warn", "{case}");
            assert!(check.message.contains("weekly evidence is unavailable"), "{case}");
            assert!(check.message.contains("not below the limit"));
        }
    }

    #[test]
    fn another_window_neither_enforces_nor_refreshes_stale_weekly_evidence() {
        let store = single_store();
        observe(&store, vec![quota("ada/one", "weekly", Some(99.0), NOW - HOUR - 1)]);
        observe(&store, vec![quota("ada/one", "5h", Some(100.0), NOW)]);
        let limits = store.account_limits().unwrap();
        assert_eq!(limits[0].measured_at_unix_ms, NOW as u64);
        assert!(limits[0].weekly_window(NOW, HOUR as u64).is_none());
        assert_eq!(
            store.enforce_account_limits(&policy(), NOW).unwrap(),
            LimitsOutcome::default()
        );
        assert_eq!(store.account_limits_check(NOW, HOUR as u64).unwrap().status, "warn");
        observe(&store, vec![quota("ada/one", "weekly", Some(94.9), NOW)]);
        assert_eq!(
            store.enforce_account_limits(&policy(), NOW).unwrap(),
            LimitsOutcome::default(),
            "a full five-hour window does not substitute for weekly quota"
        );
        assert!(live(&store, SINGLE));
    }

    #[test]
    fn seven_day_window_is_weekly_evidence_and_future_tolerance_is_bounded() {
        for (offset, stops) in [(60_000, true), (60_001, false)] {
            let store = single_store();
            observe(&store, vec![quota("ada/one", "7d", Some(95.0), NOW + offset)]);
            let outcome = store.enforce_account_limits(&policy(), NOW).unwrap();
            assert_eq!(outcome.stopped, if stops { vec![SINGLE] } else { vec![] });
        }
    }

    #[test]
    fn notification_ownership_uses_the_weekly_host_not_the_newest_other_window() {
        for (weekly_host, other_host, notifies) in [
            ("alder", "cedar", true),
            ("cedar", "alder", false),
        ] {
            let store = single_store();
            let mut weekly = quota("ada/one", "weekly", Some(99.0), NOW - 1);
            weekly["host"] = json!(weekly_host);
            let mut other = quota("ada/one", "5h", Some(20.0), NOW);
            other["host"] = json!(other_host);
            observe(&store, vec![weekly, other]);
            let outcome = store.enforce_account_limits(&policy(), NOW).unwrap();
            assert_eq!(outcome.stopped, [SINGLE]);
            assert_eq!(outcome.notified.len(), usize::from(notifies));
        }
    }

    #[test]
    fn missing_evidence_warns_without_enforcing_and_fresh_evidence_clears_warning() {
        let store = single_store();
        let missing = store.account_limits_check(NOW, HOUR as u64).unwrap();
        assert_eq!(missing.status, "warn");
        assert!(missing.message.contains("account/ada/one: no weekly reading"));
        assert_eq!(
            store.enforce_account_limits(&policy(), NOW).unwrap(),
            LimitsOutcome::default()
        );
        observe(&store, vec![quota("ada/one", "weekly", Some(40.0), NOW)]);
        assert_eq!(store.account_limits_check(NOW, HOUR as u64).unwrap().status, "pass");
        assert_eq!(
            store.account_limits_check(NOW + HOUR + 1, HOUR as u64).unwrap().status,
            "warn"
        );
    }

    #[test]
    fn stops_are_local_keep_is_respected_and_operations_is_notified_once_per_reset() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        let store = Store::open(&path, "alder").unwrap();
        declare_accounts(&store);
        for (name, host) in [("single", "alder"), ("idle", "alder"), ("kept", "alder"), ("remote", "birch")] {
            declare_seat(&store, name, "account \"ada/one\"", host);
        }
        observe(&store, vec![quota("ada/one", "weekly", Some(96.0), NOW)]);
        let configured = LimitsPolicy {
            keep: BTreeSet::from(["agent/alder.kept".into()]),
            ..policy()
        };
        let first = store.enforce_account_limits(&configured, NOW).unwrap();
        assert_eq!(first.stopped, ["agent/alder.idle", SINGLE]);
        assert_eq!(first.notified.len(), 1);
        assert!(live(&store, "agent/alder.kept"));
        assert!(live(&store, "agent/alder.remote"));
        let message = store.message(&first.notified[0]).unwrap().unwrap();
        assert_eq!(message.to, configured.notify);
        assert!(message.title.as_deref().unwrap().contains("96%"));
        assert!(message.content.contains(SINGLE));
        assert!(message.content.contains("It resets"));
        assert!(store.claims_for_kind_at("work.person-asked", None, false, 1).unwrap().claims.is_empty());
        drop(store);

        let store = Store::open(&path, "alder").unwrap();
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        declare_seat(&store, "single", "account \"ada/one\"", "alder");
        observe(&store, vec![quota("ada/one", "weekly", Some(97.0), NOW + 1)]);
        let again = store.enforce_account_limits(&configured, NOW + 1).unwrap();
        assert!(again.stopped.is_empty());
        assert!(live(&store, SINGLE));
        assert_eq!(again.notified, first.notified);
        assert_eq!(store.claims_for(&first.notified[0], Some("message.sent")).unwrap().len(), 1);
        assert_eq!(store.message(&first.notified[0]).unwrap().unwrap().content, message.content);

        let mut next = quota("ada/one", "weekly", Some(99.0), NOW + 2);
        next["resets_at"] = json!(timestamp(NOW + 14 * 24 * HOUR));
        observe(&store, vec![next]);
        let next_window = store.enforce_account_limits(&configured, NOW + 2).unwrap();
        assert_eq!(next_window.stopped, [SINGLE]);
        assert_eq!(next_window.notified.len(), 1);
        assert_ne!(next_window.notified, first.notified);
    }

    #[test]
    fn a_restarted_bound_seat_uses_account_evidence_without_its_own_quota() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        {
            let store = Store::open(&path, "alder").unwrap();
            declare_accounts(&store);
            declare_seat(&store, "single", "account \"ada/one\"", "alder");
            observe(&store, vec![quota("ada/one", "weekly", Some(95.0), NOW)]);
        }
        let store = Store::open(&path, "alder").unwrap();
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        store.append_claim(&ClaimInput {
            subject: SINGLE.into(),
            kind: "runtime.observed".into(),
            actor: Some("daemon/alder".into()),
            fields: serde_json::from_value(json!({
                "status": "running", "runtime_id": "alder.single", "incarnation_id": "inc-2"
            })).unwrap(),
            evidence: vec![],
            expected_subject: None,
            idempotency_key: None,
        }).unwrap();
        assert_eq!(store.enforce_account_limits(&policy(), NOW).unwrap().stopped, [SINGLE]);
    }

    #[test]
    fn a_pooled_seat_switches_while_a_single_account_seat_stops() {
        let store = pooled_store();
        observe(&store, vec![
            quota("ada/one", "weekly", Some(96.0), NOW),
            quota("ada/two", "weekly", Some(40.0), NOW),
        ]);
        let outcome = store.enforce_account_limits(&policy(), NOW).unwrap();
        assert_eq!(outcome.stopped, [SINGLE]);
        assert_eq!(outcome.switched, [(POOLED.to_owned(), "ada/two".to_owned())]);
        assert!(live(&store, POOLED));
        assert_eq!(store.seat_account_choice(POOLED).unwrap().as_deref(), Some("ada/two"));
        let restart = store.claims_for(POOLED, Some("runtime.action.requested")).unwrap().pop().unwrap();
        assert_eq!(restart.actor.as_deref(), Some(LIMITS_ACTOR));
        assert_eq!(restart.body["fields"]["action"], "restart");
        let again = store.enforce_account_limits(&policy(), NOW + 1).unwrap();
        assert!(again.stopped.is_empty() && again.switched.is_empty());
        // The old account remains exhausted, but it no longer owns the switched seat.
        let limits = store.account_limits().unwrap();
        let exhausted = limits.iter().find(|limit| limit.account_ref.as_deref() == Some("ada/one")).unwrap();
        assert_eq!(exhausted.weekly_window(NOW, HOUR as u64).unwrap().used_percent, Some(96.0));
        assert!(!exhausted.seats.contains(&POOLED.to_owned()));
        observe(&store, vec![quota("ada/two", "weekly", Some(99.0), NOW + 2)]);
        let exhausted_pool = store.enforce_account_limits(&policy(), NOW + 2).unwrap();
        assert!(exhausted_pool.switched.is_empty());
        assert_eq!(exhausted_pool.stopped, [POOLED]);
    }

    #[test]
    fn rebinding_does_not_enforce_the_previous_accounts_reading() {
        let store = single_store();
        observe(&store, vec![quota("ada/one", "weekly", Some(99.0), NOW)]);
        declare_seat(&store, "single", "account \"ada/two\"", "alder");
        assert_eq!(
            store.enforce_account_limits(&policy(), NOW).unwrap(),
            LimitsOutcome::default()
        );
        assert!(live(&store, SINGLE));
    }

    #[test]
    fn an_interrupted_switch_rolls_back_choice_restart_and_episode_and_can_retry() {
        let store = pooled_store();
        observe(&store, vec![
            quota("ada/one", "weekly", Some(97.0), NOW),
            quota("ada/two", "weekly", Some(20.0), NOW),
        ]);
        store.connection.write().execute_batch(
            "CREATE TRIGGER fail_account_switch BEFORE INSERT ON local_limit_stops
             BEGIN SELECT RAISE(ABORT, 'injected account switch interruption'); END;",
        ).unwrap();
        assert!(store.enforce_account_limits(&policy(), NOW).is_err());
        assert_eq!(store.seat_account_choice(POOLED).unwrap().as_deref(), Some("ada/one"));
        assert!(store.claims_for(POOLED, Some("runtime.action.requested")).unwrap().is_empty());
        let markers: u64 = store.readers.get()
            .query_row("SELECT COUNT(*) FROM local_limit_stops", [], |row| row.get(0)).unwrap();
        assert_eq!(markers, 0);
        store.connection.write().execute_batch("DROP TRIGGER fail_account_switch").unwrap();
        let retried = store.enforce_account_limits(&policy(), NOW + 1).unwrap();
        assert_eq!(retried.switched, [(POOLED.to_owned(), "ada/two".to_owned())]);
        assert_eq!(store.claims_for(POOLED, Some("runtime.action.requested")).unwrap().len(), 1);
    }

    #[test]
    fn kept_seats_never_stop_or_switch_and_exemptions_apply_to_later_seats() {
        let store = pooled_store();
        observe(&store, vec![
            quota("ada/one", "weekly", Some(100.0), NOW),
            quota("ada/two", "weekly", Some(10.0), NOW),
        ]);
        let kept = LimitsPolicy {
            keep: BTreeSet::from([SINGLE.into(), POOLED.into()]),
            ..policy()
        };
        let outcome = store.enforce_account_limits(&kept, NOW).unwrap();
        assert!(outcome.stopped.is_empty() && outcome.switched.is_empty());
        assert!(live(&store, SINGLE) && live(&store, POOLED));
        assert_eq!(store.seat_account_choice(POOLED).unwrap().as_deref(), Some("ada/one"));
        assert!(store.claims_for(POOLED, Some("runtime.action.requested")).unwrap().is_empty());
        let exempt = LimitsPolicy {
            exempt_accounts: BTreeSet::from(["ada/one".into()]),
            ..policy()
        };
        assert_eq!(store.enforce_account_limits(&exempt, NOW).unwrap(), LimitsOutcome::default());
        declare_seat(&store, "later", "account \"ada/one\"", "alder");
        assert_eq!(store.enforce_account_limits(&exempt, NOW + 1).unwrap(), LimitsOutcome::default());
        assert!(live(&store, "agent/alder.later"));
        let outcome = store.enforce_account_limits(&policy(), NOW + 2).unwrap();
        assert_eq!(outcome.stopped, ["agent/alder.later", SINGLE]);
        assert_eq!(outcome.switched, [(POOLED.to_owned(), "ada/two".to_owned())]);
    }

    #[test]
    fn exemptions_match_a_label_declared_name_or_driver_and_nothing_else() {
        let store = single_store();
        observe(&store, vec![quota("ada/one", "weekly", Some(99.0), NOW)]);
        let limit = store.account_limits().unwrap().remove(0);
        let with = |accounts: &[&str], harnesses: &[&str]| LimitsPolicy {
            exempt_accounts: accounts.iter().map(|name| (*name).to_owned()).collect(),
            exempt_harnesses: harnesses.iter().map(|name| (*name).to_owned()).collect(),
            ..policy()
        };
        assert!(!with(&[], &[]).exempts(&limit));
        assert!(with(&["ada/one"], &[]).exempts(&limit));
        assert!(with(&["account/ada/one"], &[]).exempts(&limit));
        assert!(with(&[limit.account.as_str()], &[]).exempts(&limit));
        assert!(with(&[], &["codex"]).exempts(&limit));
        assert!(!with(&["ada/two"], &["claude"]).exempts(&limit));
        assert_eq!(
            store.declared_seats(&[SINGLE.into(), "agent/alder.retired".into()]).unwrap(),
            BTreeSet::from([SINGLE.to_owned()])
        );
    }

    #[test]
    fn catch_up_is_resumable_and_no_policy_decides_from_a_partial_projection() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("claims.sqlite3");
        {
            let store = Store::open(&path, "alder").unwrap();
            declare_accounts(&store);
            declare_seat(&store, "single", "account \"ada/one\"", "alder");
            declare_seat(&store, "pooled", "account-pool \"person/ada\"", "alder");
            store.choose_seat_account(POOLED, "ada/one", NOW).unwrap();
            observe(&store, vec![quota("ada/one", "weekly", Some(99.0), NOW)]);
            observe(&store, vec![quota("ada/two", "weekly", Some(20.0), NOW)]);
        }
        {
            let connection = rusqlite::Connection::open(&path).unwrap();
            connection.execute("DELETE FROM observed_account_limits_v4", []).unwrap();
            connection.execute(
                "DELETE FROM meta WHERE key IN (?1,?2)",
                params![LIMITS_READY, "observed_account_limits_v4_through_index"],
            ).unwrap();
        }
        let store = Store::open(&path, "alder").unwrap();
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        assert!(!store.account_limits_ready().unwrap());
        assert!(store.account_limits().unwrap().is_empty());
        assert_eq!(store.enforce_account_limits(&policy(), NOW).unwrap(), LimitsOutcome::default());
        assert!(store.account_limits_check(NOW, HOUR as u64).unwrap().message.contains("catching up"));
        let binding = store.seat_binding(POOLED).unwrap().unwrap();
        assert!(store.account_for_start(POOLED, &binding, "alder", NOW, true).unwrap_err().contains("catching up"));
        assert!(store.pool_alternatives(&binding, "alder", "ada/one", 95.0, NOW).unwrap().is_empty());
        assert!(store.catch_up_account_limits(1).unwrap());
        assert!(!store.account_limits_ready().unwrap());
        assert_eq!(store.enforce_account_limits(&policy(), NOW).unwrap(), LimitsOutcome::default());
        drop(store);
        let store = Store::open(&path, "alder").unwrap();
        store.set_usage_limits_writers(vec![WRITER.into()]).unwrap();
        assert!(!store.account_limits_ready().unwrap());
        while store.catch_up_account_limits(1).unwrap() {}
        assert!(store.account_limits_ready().unwrap());
        assert_eq!(store.account_limits().unwrap().len(), 2);
        let outcome = store.enforce_account_limits(&policy(), NOW).unwrap();
        assert_eq!(outcome.stopped, [SINGLE]);
        assert_eq!(outcome.switched, [(POOLED.to_owned(), "ada/two".to_owned())]);
    }
}
