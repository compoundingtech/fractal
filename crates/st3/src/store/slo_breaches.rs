//! Canonical breach obligations and their ordinary, owner-assigned missions.
//! No timer, measurement collector, profile, network read or production activation lives here.
use super::*;
pub use st3_schema::slo_breaches::{Check, Decision, Event, HealthyWitness, Observation, Record};
use st3_schema::slo_breaches::{DEFAULT_OWNER, KIND, MAX_GROUP_BYTES, MAX_GROUP_CLAIMS, MAX_OBJECTIVES, MAX_DAILY_OPENINGS};

pub(super) fn create_schema(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS slo_episode_events (
        episode TEXT NOT NULL,event TEXT NOT NULL,member TEXT NOT NULL,fleet TEXT NOT NULL,
        objective TEXT NOT NULL,ordinal INTEGER NOT NULL,claim_id TEXT NOT NULL,body TEXT NOT NULL,
        conflict INTEGER NOT NULL DEFAULT 0,dispatched_at_ms INTEGER,
        PRIMARY KEY(episode,event));
        CREATE INDEX IF NOT EXISTS slo_objective_events ON slo_episode_events(fleet,member,objective,ordinal DESC,episode);
        CREATE INDEX IF NOT EXISTS slo_dispatch_rate ON slo_episode_events(member,dispatched_at_ms)
            WHERE event='dispatched';
        CREATE TABLE IF NOT EXISTS slo_episode_heads (
        fleet TEXT NOT NULL,member TEXT NOT NULL,objective TEXT NOT NULL,episode TEXT NOT NULL,
        ordinal INTEGER NOT NULL,conflict INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY(fleet,member,objective));
        CREATE TABLE IF NOT EXISTS slo_pending (
        episode TEXT PRIMARY KEY,fleet TEXT NOT NULL,member TEXT NOT NULL,objective TEXT NOT NULL,
        ordinal INTEGER NOT NULL,onset_end_ms INTEGER NOT NULL,onset_claim TEXT,body TEXT NOT NULL,body_key TEXT NOT NULL,
        mission_run TEXT);
        CREATE INDEX IF NOT EXISTS slo_pending_dispatch ON slo_pending(member,onset_end_ms,episode)
            WHERE mission_run IS NULL;")?;
    Ok(())
}

/// Each event is once per episode, including the initial owner-block and first unblock.
/// Equal-event conflicts poison readiness rather than selecting by arrival order.
pub(super) fn project(tx: &Transaction<'_>, claim: &ClaimRecord) -> Result<()> {
    if claim.kind != KIND { return Ok(()); }
    let repaired: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM projection_digest_repaired_claims WHERE id=?1)",
        [&claim.id], |row| row.get(0))?;
    if repaired { return Ok(()); }
    st3_schema::slo_breaches::validate_claim(&claim.subject, &claim.kind, &claim.body,
        &claim.origin, claim.actor.as_deref()).map_err(anyhow::Error::msg)?;
    let record: Record = serde_json::from_value(claim.body["fields"]["record"].clone())?;
    let body = serde_json::to_string(&record)?;
    tx.execute("INSERT INTO slo_episode_events
        (episode,event,member,fleet,objective,ordinal,claim_id,body,dispatched_at_ms)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
        ON CONFLICT(episode,event) DO UPDATE SET
        conflict=MAX(conflict,body<>excluded.body),
        body=CASE WHEN excluded.claim_id<claim_id THEN excluded.body ELSE body END,
        ordinal=MAX(ordinal,excluded.ordinal),
        dispatched_at_ms=CASE WHEN excluded.claim_id<claim_id THEN excluded.dispatched_at_ms ELSE dispatched_at_ms END,
        claim_id=MIN(claim_id,excluded.claim_id)",
        params![record.episode,record.event.name(),record.member,record.fleet,record.objective,
            record.ordinal,claim.id,body,record.dispatched_at_ms])?;
    tx.execute("INSERT INTO slo_episode_heads(fleet,member,objective,episode,ordinal)
        VALUES(?1,?2,?3,?4,?5) ON CONFLICT(fleet,member,objective) DO UPDATE SET
        conflict=CASE WHEN excluded.ordinal=ordinal AND excluded.episode<>episode THEN 1
            WHEN excluded.ordinal>ordinal THEN 0 ELSE conflict END,
        episode=CASE WHEN excluded.ordinal>ordinal THEN excluded.episode
            WHEN excluded.ordinal=ordinal THEN MIN(episode,excluded.episode) ELSE episode END,
        ordinal=MAX(ordinal,excluded.ordinal)",
        params![record.fleet,record.member,record.objective,record.episode,record.ordinal])?;
    // Every event repeats immutable onset and owner facts. This makes missing-prefix
    // arrival bounded; readiness still requires the actual Pending record below.
    let onset_body = serde_json::to_string(&pending_form(&record))?;
    let differs: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM slo_pending WHERE episode=?1 AND body<>?2)",
        params![record.episode,onset_body], |row| row.get(0))?;
    tx.execute("INSERT INTO slo_pending
        (episode,fleet,member,objective,ordinal,onset_end_ms,onset_claim,body,mission_run,body_key)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
        ON CONFLICT(episode) DO UPDATE SET
        onset_claim=CASE WHEN excluded.onset_claim IS NULL THEN onset_claim
            WHEN onset_claim IS NULL THEN excluded.onset_claim ELSE MIN(onset_claim,excluded.onset_claim) END,
        body=CASE WHEN excluded.body_key<body_key THEN excluded.body ELSE body END,
        onset_end_ms=CASE WHEN excluded.body_key<body_key THEN excluded.onset_end_ms ELSE onset_end_ms END,
        ordinal=CASE WHEN excluded.body_key<body_key THEN excluded.ordinal ELSE ordinal END,
        body_key=MIN(body_key,excluded.body_key),
        mission_run=COALESCE(mission_run,excluded.mission_run)",
        params![record.episode,record.fleet,record.member,record.objective,record.ordinal,
            record.onset.end_ms,(record.event==Event::Pending).then_some(&claim.id),onset_body,record.mission_run,claim.id])?;
    // Different events must agree on the episode's immutable identity, owner and onset.
    let poisoned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM slo_episode_events WHERE episode=?1 AND conflict<>0)",
        [&record.episode], |row| row.get(0))?;
    if differs || poisoned {
        tx.execute("UPDATE slo_episode_events SET conflict=1 WHERE episode=?1",[&record.episode])?;
    }
    Ok(())
}

fn pending_form(record: &Record) -> Record {
    let mut record = record.clone();
    record.event = Event::Pending;
    record.healthy = None;
    record.witness = None;
    record.mission_run = None;
    record.dispatched_at_ms = None;
    record.execution_ready = None;
    record
}

pub(super) fn project_index(tx: &Transaction<'_>, index: u64) -> Result<()> {
    let claim = tx.query_row("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
        FROM claims WHERE store_index=?1",[index],claim_from_row)?;
    project(tx,&claim)
}

/// Only explicit replay/checkpoint uses this source scan. Opening a daemon never calls it.
pub(super) fn rebuild(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch("DELETE FROM slo_episode_events; DELETE FROM slo_episode_heads; DELETE FROM slo_pending;")?;
    let mut statement = tx.prepare_cached(&format!("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
        FROM claims WHERE kind='slo.breach-transition' AND {UNREPAIRED_CLAIM}"))?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? { project(tx,&claim_from_row(row)?)?; }
    Ok(())
}

/// Explicit canonical record repair affects one episode, not the member's history.
/// The supported writer emits at most five source receipts per episode. Head repair
/// seeks the next objective ordinal through the dedicated index.
pub(super) fn repair(tx: &Transaction<'_>, repaired: &str) -> Result<()> {
    let original: Option<String> = tx.query_row("SELECT body FROM claims WHERE id=?1 AND kind=?2",
        params![repaired,KIND], |row| row.get(0)).optional()?;
    let Some(original) = original else { return Ok(()); };
    let record: Record = serde_json::from_value(serde_json::from_str::<Value>(&original)?["fields"]["record"].clone())?;
    let records = tx.prepare_cached(&format!("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
        FROM claims WHERE subject=?1 AND kind=?2 AND {UNREPAIRED_CLAIM}"))?
        .query_map(params![record.subject(),KIND],claim_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    tx.execute("DELETE FROM slo_episode_events WHERE episode=?1", [&record.episode])?;
    tx.execute("DELETE FROM slo_pending WHERE episode=?1", [&record.episode])?;
    tx.execute("DELETE FROM slo_episode_heads WHERE fleet=?1 AND member=?2 AND objective=?3",
        params![record.fleet,record.member,record.objective])?;
    for claim in records { project(tx, &claim)?; }
    let ordinal: Option<u64> = tx.query_row("SELECT MAX(ordinal) FROM slo_episode_events
        WHERE fleet=?1 AND member=?2 AND objective=?3", params![record.fleet,record.member,record.objective], |row| row.get(0))?;
    if let Some(ordinal) = ordinal {
        let episodes = tx.prepare_cached("SELECT DISTINCT episode FROM slo_episode_events
            WHERE fleet=?1 AND member=?2 AND objective=?3 AND ordinal=?4 ORDER BY episode LIMIT 2")?
            .query_map(params![record.fleet,record.member,record.objective,ordinal], |row| row.get::<_,String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        tx.execute("INSERT INTO slo_episode_heads(fleet,member,objective,episode,ordinal,conflict) VALUES(?1,?2,?3,?4,?5,?6)
            ON CONFLICT(fleet,member,objective) DO UPDATE SET episode=excluded.episode,ordinal=excluded.ordinal,conflict=excluded.conflict",
            params![record.fleet,record.member,record.objective,episodes[0],ordinal,episodes.len()>1])?;
    }
    Ok(())
}

fn event(tx: &Connection, episode: &str, kind: Event) -> Result<Option<(String, Record)>> {
    let row: Option<(String,String)> = tx.query_row(
        "SELECT claim_id,body FROM slo_episode_events WHERE episode=?1 AND event=?2 AND conflict=0
            AND NOT EXISTS(SELECT 1 FROM projection_digest_repaired_claims repaired WHERE repaired.id=claim_id)",
        params![episode,kind.name()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    row.map(|(id,body)| Ok((id,serde_json::from_str(&body)?))).transpose()
}

fn head(tx: &Connection, fleet: &str, member: &str, objective: &str) -> Result<Option<(String,u64,bool)>> {
    Ok(tx.query_row("SELECT episode,ordinal,conflict FROM slo_episode_heads WHERE fleet=?1 AND member=?2 AND objective=?3",
        params![fleet,member,objective],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?)
}

/// The gate's fact comes ONLY from this node's canonical transition projection. It is
/// never an independently authored resource observation. Five exact event keys bound work.
pub(super) fn fact(connection: &Connection, subject: &str) -> Result<Option<Value>> {
    if connection.is_autocommit() {
        let snapshot = connection.unchecked_transaction()?;
        let result = fact_at(&snapshot, subject)?;
        snapshot.commit()?;
        return Ok(result);
    }
    fact_at(connection, subject)
}

fn fact_at(connection: &Connection, subject: &str) -> Result<Option<Value>> {
    let Some(episode) = subject.strip_prefix("resource/slo-breach/") else { return Ok(None); };
    let row: Option<(String,Option<String>)> = connection.query_row(
        "SELECT body,onset_claim FROM slo_pending WHERE episode=?1",[episode],|r| Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((body,onset_claim)) = row else { return Ok(None); };
    let record: Record = serde_json::from_str(&body)?;
    let selected = head(connection,&record.fleet,&record.member,&record.objective)?;
    let conflict: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM slo_episode_events WHERE episode=?1 AND conflict<>0)",
        [episode],|r|r.get(0))?;
    let pending = onset_claim.is_some() && event(connection,episode,Event::Pending)?.is_some();
    let recovered = event(connection,episode,Event::Recovered)?.is_some();
    let dispatched = event(connection,episode,Event::Dispatched)?
        .is_some_and(|(_, record)| record.execution_ready == Some(true));
    let blocked = dispatched && (!person_work::declaration_live(connection, &record.owner)?
        || (event(connection,episode,Event::OwnerBlocked)?.is_some()
            && event(connection,episode,Event::OwnerUnblocked)?.is_none()));
    let lineage = lineage_valid(connection, &record)?;
    let selected = selected.is_some_and(|(id,ordinal,conflict)| id==episode && ordinal==record.ordinal && !conflict);
    let state = if conflict || !pending || !lineage || !selected { "unknown" }
        else if recovered { "recovered" } else if blocked { "owner_unavailable" }
        else if dispatched { "ready" } else { "pending" };
    Ok(Some(json!({"facts":{"dispatch_state":state,"episode":episode,"owner":record.owner,
        "member":record.member,"objective":record.objective}})))
}

fn lineage_valid(connection: &Connection, record: &Record) -> Result<bool> {
    let Some(previous) = &record.previous_recovery else { return Ok(record.ordinal == 1); };
    // A claim-id primary-key seek, followed by one episode/event primary-key seek.
    // Never search retained episode history for a predecessor.
    let body: Option<String> = connection.query_row("SELECT body FROM claims WHERE id=?1 AND kind=?2",
        params![previous,KIND], |row| row.get(0)).optional()?;
    let Some(body) = body else { return Ok(false); };
    let prior: Record = serde_json::from_value(serde_json::from_str::<Value>(&body)?["fields"]["record"].clone())?;
    Ok(prior.event == Event::Recovered && prior.fleet == record.fleet && prior.member == record.member
        && prior.objective == record.objective && prior.ordinal.checked_add(1) == Some(record.ordinal)
        && prior.healthy.as_ref().is_some_and(|healthy| record.onset.start_ms >= healthy.end_ms
            && record.onset.predecessor_cut.as_deref() == Some(healthy.source_cut.as_str()))
        && event(connection, &prior.episode, Event::Recovered)?.is_some_and(|(id, _)| id == *previous))
}

/// This receipt remains true for an admitted fix after recovery or a newer episode.
/// A never-admitted recovered mission has false in the receipt at every prefix.
pub(super) fn execution_admitted(connection: &Connection, run: &str) -> Result<bool> {
    if !run.starts_with("mission-run/slo/") { return Ok(true); }
    if connection.is_autocommit() {
        let snapshot = connection.unchecked_transaction()?;
        let result = execution_admitted_at(&snapshot, run)?;
        snapshot.commit()?;
        return Ok(result);
    }
    execution_admitted_at(connection, run)
}

fn execution_admitted_at(connection: &Connection, run: &str) -> Result<bool> {
    let Some(episode) = run.strip_prefix("mission-run/slo/") else { return Ok(true); };
    let Some((_, pending)) = event(connection, episode, Event::Pending)? else { return Ok(false); };
    let Some((_, dispatched)) = event(connection, episode, Event::Dispatched)? else { return Ok(false); };
    let conflict: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM slo_episode_events WHERE episode=?1 AND conflict<>0)",
        [episode], |row| row.get(0))?;
    Ok(!conflict && dispatched.execution_ready == Some(true)
        && pending_form(&dispatched) == pending && lineage_valid(connection, &pending)?)
}

fn append(tx: &Transaction<'_>, origin: &str, record: &Record, batch: &str) -> Result<ClaimRecord> {
    record.validate().map_err(anyhow::Error::msg)?;
    let claim = append_claim_tx(tx,origin,&record.subject(),KIND,Some("daemon/runtime"),
        &json!({"fields":{"record":record}}),&[],Some(batch))?;
    anyhow::ensure!(encoded_upper_bound(serde_json::to_vec(&claim)?.len(), 1) <= 16*1024,
        "SLO receipt including signed/base64 framing exceeds 16 KiB");
    Ok(claim)
}

fn check_group(tx: &Connection, batch: &str) -> Result<()> {
    let records = tx.prepare_cached("SELECT id,store_index,batch_id,subject,kind,origin,actor,body,predecessors,accepted_at_unix_ms
        FROM claims WHERE batch_id=?1 LIMIT 17")?.query_map([batch],claim_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // System-runtime claims seal with the node key (no principal delegation chain).
    // Include a KiB per claim for its node signature/map entry, 2 KiB of batch
    // metadata, base64 expansion, and another KiB for the outer signed envelope.
    anyhow::ensure!(records.len()<=MAX_GROUP_CLAIMS
        && encoded_upper_bound(serde_json::to_vec(&records)?.len(), records.len())<=MAX_GROUP_BYTES,
        "SLO group exceeds 16 claims or signed/base64 32 KiB bound");
    Ok(())
}

fn encoded_upper_bound(claim_json_bytes: usize, count: usize) -> usize {
    (claim_json_bytes + count*1024 + 2048).saturating_mul(4).div_ceil(3) + 1024
}

/// Prepared off the writer, then revalidated against its exact pending source claim.
pub struct PreparedSloMission {
    record: Record,
    onset_claim: String,
    member_key: String,
    authority_generation: i64,
    workspace: String,
    mission: MissionSpec,
}

fn prepare_mission(record: &Record) -> Result<MissionSpec> {
    let id = format!("slo/{}",record.episode);
    let goal = format!("Repair {} on {}; onset {} at cut {}. Recovery is not evidence of a code fix.",
        record.objective,record.member,record.onset.observed_at_ms,record.onset.source_cut).replace("${","$${");
    let source = format!("version 2\nmission {id:?} state=\"ready\" {{
        completion {{ when \"all-steps-exhausted\" }}
        goal {goal:?}
        step \"dispatch-ready\" {{ agentless
            gate \"this exact episode was admitted for execution\" {{
                field \"dispatch_state\" {:?} is \"ready\"
            }}
        }}
        step \"fix\" {{ depends-on {{ step \"dispatch-ready\" completed }}
            assigned-to {:?}; goal {goal:?}
        }}
    }}",record.subject(),record.owner);
    crate::graph::parse_internal_intent(&source,&record.member)?.missions.remove(&id)
        .context("prepared SLO mission missing")
}

impl Store {
    fn slo_member(&self) -> Result<(String,String,i64)> {
        let fleet = self.bound_fleet()?.context("SLO observations need an authenticated fleet")?;
        let key = self.member_public_key().context("SLO observations need this member's signing key")?;
        if let Some(first) = self.first_sync()? {
            anyhow::ensure!(first.state=="verified", "SLO source lineage is UNKNOWN until first sync completes");
        }
        let generation: i64 = self.readers.get().query_row("SELECT value FROM fleet_generation WHERE id=1", [], |row| row.get(0))?;
        {
            let membership = self.membership_cache.lock().unwrap_or_else(PoisonError::into_inner);
            let (at, membership) = membership.as_ref().context("SLO authority is UNKNOWN until the existing membership reader publishes it")?;
            anyhow::ensure!(*at == generation, "SLO membership certificate is stale");
            anyhow::ensure!(matches!(membership.state(&self.origin), crate::fleet::MemberState::Current(member)
                if member.member_key == key), "SLO origin has no unambiguous current member credential");
        }
        anyhow::ensure!(!self.replication_projection_needs_recovery()?, "SLO source projection is UNKNOWN");
        Ok((fleet,key,generation))
    }

    fn slo_authority_at(&self, tx: &Connection, fleet: &str, key: &str, generation: i64) -> Result<()> {
        let current: i64 = tx.query_row("SELECT value FROM fleet_generation WHERE id=1", [], |row| row.get(0))?;
        anyhow::ensure!(current == generation && self.member_public_key().as_deref() == Some(key)
            && smallclaims::store::fleet_meta(tx, "fleet_id")?.as_deref() == Some(fleet),
            "SLO prepared member authority changed");
        anyhow::ensure!(smallclaims::store::first_sync::first_sync_tx(tx)?.is_none_or(|first| first.state == "verified"),
            "SLO first sync is incomplete");
        let healthy: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM projection_health WHERE aggregate='graph' AND status='healthy')", [], |row| row.get(0))?;
        anyhow::ensure!(healthy, "SLO source projection is UNKNOWN");
        Ok(())
    }

    /// Private source seam only; no daemon task calls this in this change. UNKNOWN/partial
    /// windows create no false breach or recovery. Accepted onsets survive restart/replay.
    pub fn record_slo_miss(&self, objective: &str, owner: Option<&str>, observation: Observation, witness: Option<HealthyWitness>) -> Result<Option<String>> {
        observation.validate().map_err(anyhow::Error::msg)?;
        anyhow::ensure!(observation.member == self.origin, "relabelled remote SLO reading");
        if observation.decision()!=Decision::Missed { return Ok(None); }
        let (fleet,key,generation) = self.slo_member()?;
        let batch = Uuid::now_v7().simple().to_string();
        self.connection.batched(|tx| {
            self.slo_authority_at(tx, &fleet, &key, generation)?;
            let current = head(tx,&fleet,&self.origin,objective)?;
            let (ordinal,previous) = if let Some((episode,ordinal,conflict))=current {
                anyhow::ensure!(!conflict,"conflicting SLO lineage is UNKNOWN");
                if let Some((id,recovery))=event(tx,&episode,Event::Recovered)? {
                    // A recovered obligation must still get its one mission before a later
                    // episode can replace it. Late observations cannot start a new episode.
                    if event(tx,&episode,Event::Dispatched)?.is_none() { return Ok(Some(episode)); }
                    let healthy = recovery.healthy.as_ref().context("recovery window missing")?;
                    anyhow::ensure!(observation.start_ms >= healthy.end_ms
                        && observation.predecessor_cut.as_deref() == Some(healthy.source_cut.as_str()),
                        "stale or unlinked miss predates recovery");
                    (ordinal.checked_add(1).context("SLO ordinal overflow")?,Some(id))
                }
                else { return Ok(Some(episode)); }
            } else {
                let count: usize = tx.query_row("SELECT COUNT(*) FROM slo_episode_heads WHERE fleet=?1 AND member=?2",
                    params![fleet,self.origin],|r|r.get(0))?;
                anyhow::ensure!(count<MAX_OBJECTIVES,"SLO member already has 64 objective slots");
                (1,None)
            };
            let record=Record{episode:Record::episode_id(&fleet,&self.origin,objective,previous.as_deref()),
                fleet:fleet.clone(),member:self.origin.clone(),objective:objective.into(),ordinal,
                previous_recovery:previous,owner:owner.unwrap_or(DEFAULT_OWNER).into(),event:Event::Pending,
                onset:observation.clone(),onset_witness:witness.clone(),healthy:None,witness:None,mission_run:None,dispatched_at_ms:None,execution_ready:None};
            append(tx,&self.origin,&record,&batch)?;
            check_group(tx,&batch)?;
            Ok(Some(record.episode))
        }).map_err(anyhow::Error::msg)?
    }

    /// Oldest undispatched durable obligations; callers prepare at most two, outside writer.
    pub fn prepare_slo_missions(&self, workspace: &str) -> Result<Vec<PreparedSloMission>> {
        anyhow::ensure!(workspace.len()<=1024 && Path::new(workspace).is_absolute(),
            "SLO mission preparation needs a bounded absolute workspace");
        let (_,key,generation)=self.slo_member()?;
        let connection=self.readers.get();
        let rows=connection.prepare_cached("SELECT pending.body,pending.onset_claim FROM slo_pending pending
            JOIN slo_episode_heads heads ON heads.fleet=pending.fleet AND heads.member=pending.member
                AND heads.objective=pending.objective AND heads.episode=pending.episode AND heads.conflict=0
            WHERE pending.member=?1 AND pending.mission_run IS NULL AND pending.onset_claim IS NOT NULL
                AND NOT EXISTS(SELECT 1 FROM slo_episode_events events WHERE events.episode=pending.episode AND events.conflict<>0)
            ORDER BY pending.onset_end_ms,pending.episode LIMIT 2")?.query_map([&self.origin],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(connection);
        rows.into_iter().map(|(body,onset_claim)| {
            let record:Record=serde_json::from_str(&body)?;
            let mission=prepare_mission(&record)?;
            Ok(PreparedSloMission{record,onset_claim,member_key:key.clone(),authority_generation:generation,workspace:workspace.into(),mission})
        }).collect()
    }

    pub fn dispatch_slo_mission(&self, prepared: PreparedSloMission) -> Result<String> {
        let (_,key,generation)=self.slo_member()?;
        anyhow::ensure!(key==prepared.member_key && generation==prepared.authority_generation,
            "SLO member authority changed during preparation");
        let batch=Uuid::now_v7().simple().to_string();
        self.connection.batched(|tx| {
            self.slo_authority_at(tx, &prepared.record.fleet, &prepared.member_key, prepared.authority_generation)?;
            let episode=&prepared.record.episode;
            if let Some((_,existing))=event(tx,episode,Event::Dispatched)? {
                return existing.mission_run.context("dispatch has no mission");
            }
            let (id,source)=event(tx,episode,Event::Pending)?.context("SLO pending onset missing")?;
            anyhow::ensure!(id==prepared.onset_claim && source==prepared.record,"SLO prepared onset changed");
            anyhow::ensure!(lineage_valid(tx, &source)?, "SLO predecessor lineage is UNKNOWN");
            let selected=head(tx,&source.fleet,&source.member,&source.objective)?;
            anyhow::ensure!(selected==Some((episode.clone(),source.ordinal,false)),"SLO lineage changed during preparation");
            let now=u64::try_from(now_ms())?;
            let recent:usize=tx.query_row("SELECT COUNT(*) FROM (SELECT 1 FROM slo_episode_events
                WHERE member=?1 AND event='dispatched' AND dispatched_at_ms>=?2 LIMIT 2)",
                params![self.origin,now.saturating_sub(60_000)],|r|r.get(0))?;
            anyhow::ensure!(recent<2,"SLO dispatch rate: at most two mission openings per minute");
            let daily: usize = tx.query_row("SELECT COUNT(*) FROM (SELECT 1 FROM slo_episode_events
                WHERE member=?1 AND event='dispatched' AND dispatched_at_ms>=?2 LIMIT 700)",
                params![self.origin,now.saturating_sub(86_400_000)],|r|r.get(0))?;
            anyhow::ensure!(daily<MAX_DAILY_OPENINGS,"SLO daily storage budget exhausted; durable obligation remains pending");
            let recovered_before_dispatch=event(tx,episode,Event::Recovered)?.is_some();
            let mission=&prepared.mission;
            let published=append_claim_tx(tx,&self.origin,&mission.subject,"mission.published",Some("daemon/runtime"),
                &serde_json::to_value(mission)?,&[],Some(&batch))?;
            select_replicated_mission(tx,&published,published.store_index)?;
            let run=format!("mission-run/slo/{episode}");
            let declaration=MissionRunDeclaration{subject:run.clone(),creation:Some(MissionRunCreation{
                mission:mission.id.clone(),revision:mission.revision.clone(),workspace:prepared.workspace.clone(),
                requester:"daemon/runtime".into(),inputs:BTreeMap::new(),mode:"run".into(),after:None,
                report_to:None,report_completed:false}),..Default::default()};
            create_declared_mission_run_tx(tx,&self.origin,&declaration,&batch)?;
            let mut dispatched=source.clone();
            dispatched.event=Event::Dispatched;
            dispatched.mission_run=Some(run.clone());
            dispatched.dispatched_at_ms=Some(now);
            dispatched.execution_ready=Some(!recovered_before_dispatch);
            append(tx,&self.origin,&dispatched,&batch)?;
            if recovered_before_dispatch {
                // This mission never had a positive dispatch fact at any delivery prefix.
                let cancelled=append_claim_tx(tx,&self.origin,&run,"mission-run.state",Some("daemon/runtime"),
                    &json!({"fields":{"status":"cancelled","phase":"terminal",
                        "reason":"breach recovered before execution admission; no repair performed"}}),&[],Some(&batch))?;
                project_mission_run_update(tx,&cancelled)?;
            } else if !person_work::declaration_live(tx,&source.owner)? {
                let mut blocked=source;
                blocked.event=Event::OwnerBlocked;
                append(tx,&self.origin,&blocked,&batch)?;
            }
            check_group(tx,&batch)?;
            Ok(run)
        }).map_err(anyhow::Error::msg)?
    }

    pub fn recover_slo_breach(&self, episode: &str, healthy: Observation, witness: Option<HealthyWitness>) -> Result<()> {
        let (fleet,key,generation) = self.slo_member()?;
        let batch=Uuid::now_v7().simple().to_string();
        self.connection.batched(|tx| {
            self.slo_authority_at(tx, &fleet, &key, generation)?;
            if event(tx,episode,Event::Recovered)?.is_some() { return Ok(()); }
            let (_,mut record)=event(tx,episode,Event::Pending)?.context("SLO onset missing")?;
            anyhow::ensure!(record.member==self.origin,"only the observing member recovers its episode");
            anyhow::ensure!(head(tx,&record.fleet,&record.member,&record.objective)?==Some((episode.into(),record.ordinal,false)),
                "stale health cannot recover an obsolete episode");
            record.event=Event::Recovered; record.healthy=Some(healthy); record.witness=witness;
            append(tx,&self.origin,&record,&batch)?;
            check_group(tx,&batch)
        }).map_err(anyhow::Error::msg)?
    }

    /// One initial block/unblock receipt per episode; subsequent availability is read from
    /// ordinary owner facts, not an SLO write loop. No actor reassignment or person fallback.
    pub fn unblock_slo_owner(&self, episode: &str) -> Result<()> {
        let (fleet,key,generation) = self.slo_member()?;
        let batch=Uuid::now_v7().simple().to_string();
        self.connection.batched(|tx| {
            self.slo_authority_at(tx, &fleet, &key, generation)?;
            if event(tx,episode,Event::OwnerUnblocked)?.is_some()
                || event(tx,episode,Event::Recovered)?.is_some() { return Ok(()); }
            let (_,mut record)=event(tx,episode,Event::OwnerBlocked)?.context("SLO owner block missing")?;
            anyhow::ensure!(record.member==self.origin && person_work::declaration_live(tx,&record.owner)?,
                "exact SLO owner remains unavailable");
            record.event=Event::OwnerUnblocked;
            append(tx,&self.origin,&record,&batch)?;
            check_group(tx,&batch)
        }).map_err(anyhow::Error::msg)?
    }
}

#[cfg(test)]
mod tests;
