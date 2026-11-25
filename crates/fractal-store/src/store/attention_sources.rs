//! Subject-keyed source folds for the published person-attention collection.
//! Entries retain eventual rows, so clock admissions do not rerun source reducers.
use super::*;

type DiscussionContexts = BTreeMap<(String, String, String), Vec<serde_json::Map<String, Value>>>;

#[derive(Clone)]
struct SourceEntry {
    rows: Vec<(AttentionItemView, u128, serde_json::Map<String, Value>)>,
    reads: BTreeSet<String>,
    refresh_at: Option<u128>,
}

/// Volatile source state at a publication cut. A reset discards it; ordinary commits replace
/// only changed subjects and the shared source groups whose inputs changed.
#[derive(Clone, Default)]
pub struct AttentionSourceSnapshot {
    pub items: Vec<AttentionItemView>,
    /// Discussion context captured before projecting the responsible conversation agent.
    pub contexts: DiscussionContexts,
    pub valid_until: Option<u128>,
    /// Actual subject reducer calls in this refresh, including empty results.
    pub refolded_subjects: usize,
    /// Shared login, drift, review, hold and broken-gate reducers run in this refresh.
    pub refolded_groups: usize,
    entries: BTreeMap<String, Arc<SourceEntry>>,
}

impl AttentionSourceSnapshot {
    /// Runtime-derived candidates include rows whose admission time is still in the future.
    pub fn candidate_seats(&self) -> impl Iterator<Item = &str> {
        self.entries.values().flat_map(|entry| entry.rows.iter())
            .filter(|(item, _)| matches!(item.kind.as_str(), "harness-login" | "harness-prompt" | "model-drift"))
            .flat_map(|(item, _)| std::iter::once(item.subject.as_str()).chain(item.targets.iter().map(String::as_str)))
    }

    /// Number of subject candidates, excluding the fixed shared reducer groups.
    pub fn retained_subjects(&self) -> usize {
        self.entries.keys().filter(|key| key.starts_with("subject:")).count()
    }
}

impl Store {
    /// `changed=None` is a full reset (initial publication, checkpoint, or projection reset).
    /// Otherwise `changed` contains claim subjects and activity actors and `kinds` their kinds.
    /// An empty change set only advances admission clocks, with no source reduction.
    pub fn attention_source_snapshot(
        &self,
        previous: Option<&AttentionSourceSnapshot>,
        changed: Option<&BTreeSet<String>>,
        kinds: &BTreeSet<String>,
        now: u128,
    ) -> Result<AttentionSourceSnapshot> {
        let full = previous.is_none() || changed.is_none();
        let empty = BTreeSet::new();
        let changed = changed.unwrap_or(&empty);
        let mut snapshot = AttentionSourceSnapshot {
            entries: previous.map(|snapshot| snapshot.entries.clone()).unwrap_or_default(),
            ..Default::default()
        };
        let connection = self.readers.get();
        let mut changed_keys = changed.clone();
        for subject in changed {
            changed_keys.insert(format!("actor:{subject}"));
            changed_keys.insert(format!("generations:{subject}"));
            changed_keys.insert(format!("claim-subject:{subject}"));
            for kind in kinds {
                changed_keys.insert(smallclaims::touched::claim_key(subject, kind));
            }
        }
        changed_keys.extend(kinds.iter().map(|kind| format!("kind:{kind}")));
        let mut subjects = if full {
            snapshot.entries.clear();
            connection.prepare_cached(
                "SELECT subject FROM step_runs WHERE assignee>='person/' AND assignee<'person0'
                 UNION SELECT subject FROM desired WHERE kind='agent'
                 UNION SELECT subject FROM custom_sources",
            )?.query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<BTreeSet<_>>>()?
        } else {
            changed.clone()
        };
        // A run, declaration or deadline-clock change can invalidate another source's ask.
        // Keep the actual reducer reads, including negative reads, rather than every item of
        // the same kind. Subject discovery above is needed only for a full reset.
        for (key, entry) in &snapshot.entries {
            if let Some(subject) = key.strip_prefix("subject:")
                && (!entry.reads.is_disjoint(&changed_keys)
                    || entry.refresh_at.is_some_and(|due| now >= due))
            {
                subjects.insert(subject.to_owned());
            }
        }
        // Registration changes can introduce sources that previously projected no card.
        if kinds.iter().any(|kind| kind == "custom.st3-kinds.registered" || kind == "doc.bound") {
            subjects.extend(connection.prepare_cached("SELECT subject FROM custom_sources")?
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?);
        }
        if !changed.is_empty() {
            subjects.extend(connection.prepare_cached(
                "SELECT source FROM custom_dependencies
                 WHERE dependency IN (SELECT value FROM json_each(?1))
                    OR dependency IN (SELECT id FROM claims
                        WHERE subject IN (SELECT value FROM json_each(?1)))",
            )?.query_map([serde_json::to_string(changed)?], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?);
        }
        for subject in subjects {
            let is_agent = subject.starts_with("agent/");
            let is_step = subject.starts_with("step-run/");
            let custom: Option<(String, String)> = connection.prepare_cached(
                "SELECT body, requested_at FROM custom_sources WHERE subject=?1 AND active=1",
            )?.query_row(/* sqlite-large-value: allow-for-now lv-h2acf43c6c302 owner=fractal_store::store::attention_sources TODO=trace-statement-and-value-representation */ [&subject], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
            let key = format!("subject:{subject}");
            // Keep empty live candidates: their negative reads discover newly eligible rows.
            // Historical/undeclared subjects cannot contribute until their own state changes.
            let step_candidate = if is_step {
                person_work::step(&connection, &subject)?.is_some_and(|view| {
                    matches!(view.status.as_str(), "pending" | "ready")
                        && view.assigned_to.as_deref().is_some_and(|assignee| {
                            assignee.get(..7).is_some_and(|prefix| prefix.eq_ignore_ascii_case("person/"))
                        })
                })
            } else { false };
            let agent_candidate = is_agent && person_work::requester_declared(&connection, &subject)?;
            if !step_candidate && !agent_candidate && custom.is_none() {
                snapshot.entries.remove(&key);
                continue;
            }
            let mut contexts = DiscussionContexts::new();
            let mut context_reads = BTreeSet::new();
            let mut refresh_at = None;
            let (rows, mut reads) = smallclaims::touched::record(|| -> Result<Vec<AttentionItemView>> {
                let mut rows = Vec::new();
                if step_candidate {
                    rows.extend(person_attention_item(&connection, &subject, i64::MAX as u128)?);
                }
                if agent_candidate {
                    let (prompts, due) = self.harness_prompt_attention_items_with_deadline(
                        None, now, Some(&BTreeSet::from([subject.clone()])),
                    )?;
                    rows.extend(prompts);
                    refresh_at = due;
                }
                if let Some((body, at)) = custom
                    && !fractal_schema::notice::is_reserved_subject(&subject)
                {
                    rows.push(custom::attention_item(&serde_json::from_str(&body)?, at.parse().unwrap_or(0))?);
                }
                self.current_attention_with_context(rows, i64::MAX as u128, |item| {
                    retain_discussion_context(&connection, item, &mut contexts, &mut context_reads)
                })
            });
            reads.insert(subject.clone());
            reads.extend(context_reads);
            let rows = rows?;
            let mut entry = source_entry(&connection, rows, contexts, reads)?;
            entry.refresh_at = refresh_at;
            snapshot.entries.insert(key, Arc::new(entry));
            snapshot.refolded_subjects += 1;
        }
        let any = |prefix: &str| kinds.iter().any(|kind| kind.starts_with(prefix));
        let run_change = any("mission-run.") || any("mission.") || any("run-generation.");
        let declaration = any("intent.") || any("member.") || any("rollout.");
        let failure = any("operational.");
        let mut groups = Vec::new();
        if full || run_change || any("review.") || any("human-review.") || any("gate.")
            || any("revision-proposal.") || any("planning-session.") || any("step-run.") || any("loop-run.") {
            groups.push("reviews");
        }
        if full || any("host-hold.") { groups.push("holds"); }
        let login_activity = any("harness.") || any("runtime.")
            || any("transport.future-") || any("work.progress");
        let mut login_changed = full || run_change || declaration;
        if !login_changed && login_activity {
            login_changed = snapshot.entries.get("group:logins").is_some_and(|entry| {
                entry.rows.iter().any(|(item, _, _)| changed.contains(&item.subject)
                    || item.targets.iter().any(|seat| changed.contains(seat)))
            });
            if !login_changed {
                for subject in changed.iter().filter(|subject| subject.starts_with("agent/")) {
                    if self.current_harness_for_login(subject)?
                        .is_some_and(|harness| harness.state == "needs-login")
                    {
                        login_changed = true;
                        break;
                    }
                }
            }
        }
        if login_changed { groups.push("logins"); }
        if full || run_change || declaration || failure { groups.push("drift"); }
        if full || run_change || declaration || failure || any("step-run.")
            || any("gate.") || any("effect.") { groups.push("gates"); }
        for (key, entry) in &snapshot.entries {
            if let Some(group) = key.strip_prefix("group:")
                && !entry.reads.is_disjoint(&changed_keys)
                && !groups.contains(&group)
            {
                groups.push(match group {
                    "reviews" => "reviews",
                    "holds" => "holds",
                    "logins" => "logins",
                    "drift" => "drift",
                    "gates" => "gates",
                    _ => unreachable!(),
                });
            }
        }
        for group in groups {
            let mut contexts = DiscussionContexts::new();
            let mut context_reads = BTreeSet::new();
            let (rows, reads) = smallclaims::touched::record(|| -> Result<Vec<AttentionItemView>> {
                let rows = match group {
                    "reviews" => self.mission_run_attention_items(None)?,
                    "holds" => self.host_hold_attention_items(None)?,
                    "logins" => self.harness_login_attention_items(None)?,
                    "drift" => self.model_drift_attention_items(None, i64::MAX as u128)?,
                    "gates" => self.broken_gate_items(None, i64::MAX as u128)?.into_iter()
                        .filter(|item| item.person.starts_with("person/")).collect(),
                    _ => unreachable!(),
                };
                self.current_attention_with_context(rows, i64::MAX as u128, |item| {
                    retain_discussion_context(&connection, item, &mut contexts, &mut context_reads)
                })
            });
            // Login candidate absence is invalidated by the changed seat's current login
            // state above. Retaining every candidate read would make an unrelated prompt
            // refold all login groups even when no login card exists.
            let mut reads = if group == "logins" { BTreeSet::new() } else { reads };
            reads.extend(context_reads);
            snapshot.entries.insert(format!("group:{group}"), Arc::new(source_entry(&connection, rows?, contexts, reads)?));
            snapshot.refolded_groups += 1;
        }
        snapshot.items.clear();
        snapshot.contexts.clear();
        snapshot.valid_until = None;
        for entry in snapshot.entries.values() {
            if let Some(due) = entry.refresh_at {
                snapshot.valid_until = Some(snapshot.valid_until.map_or(due, |kept| kept.min(due)));
            }
            for (item, starts_at, context) in &entry.rows {
                if *starts_at <= now {
                    snapshot.items.push(item.clone());
                    let key = (item.subject.clone(), item.person.clone(), item.episode.clone());
                    snapshot.contexts.entry(key).or_default().push(context.clone());
                } else {
                    snapshot.valid_until = Some(snapshot.valid_until.map_or(*starts_at, |due| due.min(*starts_at)));
                }
            }
        }
        super::super::attention::sort_attention_items(&mut snapshot.items);
        Ok(snapshot)
    }
}

fn source_entry(
    connection: &Connection,
    rows: Vec<AttentionItemView>,
    mut contexts: DiscussionContexts,
    mut reads: BTreeSet<String>,
) -> Result<SourceEntry> {
    let rows = rows.into_iter().map(|item| {
        reads.insert(item.subject.clone());
        reads.insert(item.person.clone());
        reads.extend(item.targets.iter().cloned());
        if let Some(run) = &item.mission_run {
            reads.insert(if run.starts_with("mission-run/") { run.clone() } else { format!("mission-run/{run}") });
        }
        if let Some(conversation) = &item.conversation { reads.insert(conversation.clone()); }
        let accepted = if item.kind == "person-step" {
            person_work::request(connection, &item.subject)?.map_or(0, |ask| ask.accepted_at_unix_ms)
        } else { 0 };
        let starts_at = item.requested_at_unix_ms.max(accepted);
        let key = (item.subject.clone(), item.person.clone(), item.episode.clone());
        let context = contexts.get_mut(&key).and_then(Vec::pop)
            .context("current attention discussion context is missing")?;
        Ok((item, starts_at, context))
    }).collect::<Result<Vec<_>>>()?;
    Ok(SourceEntry { rows, reads, refresh_at: None })
}

fn retain_discussion_context(
    connection: &Connection,
    item: &AttentionItemView,
    contexts: &mut DiscussionContexts,
    reads: &mut BTreeSet<String>,
) -> Result<()> {
    // Preserve the original association: projection can replace it with a fallback.
    if let Some(agent) = &item.conversation { reads.insert(agent.clone()); }
    let context = super::super::attention::discussion_context_at(connection, item)?;
    if let Some(agent) = context.get("start_recipient").and_then(|recipient| recipient["agent_id"].as_str()) {
        reads.insert(agent.to_owned());
    }
    contexts.entry((item.subject.clone(), item.person.clone(), item.episode.clone()))
        .or_default().push(context);
    Ok(())
}
