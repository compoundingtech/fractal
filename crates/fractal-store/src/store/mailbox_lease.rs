//! A channel binding is a connection capability, not authority to replace a live session.
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::mailbox_lease_shared_seats::*;
#[cfg(any(test, feature = "test-support"))]
#[allow(unused_imports)]
pub use crate::store::mailbox_lease_shared_seats::{AFTER_FAULT_ACQUISITION, AFTER_PROMOTION_PRECHECK, BEFORE_FAULT_READER, CARD_FAULT_DEADLINES, DURING_FAULT_READ};
use super::*;
use crate::mailbox::{Authority, Fence};
use crate::model::{LaunchSpec, MemberKind, MemberSpec};

#[cfg(any(test, feature = "test-support"))]
pub type FaultReadHook = Box<dyn FnOnce(&Connection) -> Result<()>>;

#[cfg(any(test, feature = "test-support"))]
impl Store {
    pub fn expire_next_card_fault_read_for_test() {
        Self::expire_card_fault_reads_for_test(1);
    }

    pub fn expire_card_fault_reads_for_test(count: usize) {
        CARD_FAULT_DEADLINES.with(|remaining| remaining.set(count));
    }
}

pub fn lease(
    connection: &Connection,
    fence: &Fence,
) -> Result<Option<(Fence, Authority, bool)>, St3Error> {
    connection
        .prepare_cached(
            "SELECT incarnation,token,epoch,provider,session,sequence,pid,process_token,revoked
         FROM local_mailbox_leases WHERE subject=?1 AND component=?2",
        )
        .map_err(internal)?
        .query_row(/* sqlite-large-value: allow-for-now lv-h4c5d5462bedf owner=st3_store::store::mailbox_lease TODO=trace-statement-and-value-representation */ params![fence.subject, fence.component], |row| {
            Ok((
                Fence {
                    subject: fence.subject.clone(),
                    component: fence.component.clone(),
                    incarnation: row.get(0)?,
                    token: row.get(1)?,
                    epoch: row.get(2)?,
                },
                Authority {
                    provider: row.get(3)?,
                    session: row.get(4)?,
                    sequence: row.get(5)?,
                    pid: row.get(6)?,
                    process_token: row.get::<_, String>(7)?.parse().map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            7,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                },
                row.get(8)?,
            ))
        })
        .optional()
        .map_err(internal)
}


pub fn refused(reason: &str) -> St3Error {
    St3Error::new("stale-mailbox-session", reason)
}

/// Durable designation of a physically admitted argv capability. It is not a
/// native provider lease and cannot be inferred from the current declaration.
fn argv_binding(connection: &Connection, fence: &Fence) -> Result<Option<MemberSpec>, St3Error> {
    let held: Option<(Fence, String)> = connection.prepare_cached(
        "SELECT subject,component,incarnation,epoch,member FROM local_mailbox_argv_bindings WHERE token=?1",
    ).map_err(internal)?.query_row(/* sqlite-large-value: allow-for-now lv-heb6759882bc5 owner=st3_store::store::mailbox_lease TODO=trace-statement-and-value-representation */ [&fence.token], |row| Ok((Fence {
        token: fence.token.clone(), subject: row.get(0)?, component: row.get(1)?,
        incarnation: row.get(2)?, epoch: row.get(3)?,
    }, row.get(4)?))).optional().map_err(internal)?;
    let Some((held, member)) = held else { return Ok(None); };
    if held.subject != fence.subject || held.component != fence.component
        || held.incarnation != fence.incarnation || held.epoch != fence.epoch
    {
        return Err(refused("the argv capability belongs to another binding"));
    }
    serde_json::from_str(&member).map(Some).map_err(internal)
}

pub fn check_lease_fence(
    connection: &Connection,
    fence: &Fence,
    host: &str,
) -> Result<(), St3Error> {
    if let Some(member) = argv_binding(connection, fence)? {
        check_argv_declaration(connection, fence, &member, host)?;
    }
    let Some((held, owner, revoked)) = lease(connection, fence)? else {
        return Ok(());
    };
    if revoked
        || held.token != fence.token
        || held.epoch != fence.epoch
        || held.incarnation != fence.incarnation
    {
        return Err(refused("the authenticated lease was superseded or revoked"));
    }
    let desired = current_desired_row(connection, &fence.subject)
        .map_err(internal)?
        .ok_or_else(|| refused("the seat is no longer declared"))?;
    let member: Option<MemberSpec> = desired
        .member
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(internal)?;
    if desired.kind == "stop"
        || member.as_ref().is_none_or(|member| {
            member.driver.as_deref() != Some(&owner.provider)
                || member.host != host
                || member
                    .terminal_binding
                    .as_ref()
                    .is_some_and(|binding| binding.agent_incarnation() != fence.incarnation)
        })
    {
        return Err(refused(
            "the lease's declared provider was stopped or replaced",
        ));
    }
    Ok(())
}

pub fn check_argv_declaration(
    connection: &Connection,
    fence: &Fence,
    captured: &MemberSpec,
    host: &str,
) -> Result<(), St3Error> {
    if lease(connection, fence)?.is_some() {
        return Err(refused("qualified mailbox history cannot become an argv-only transport"));
    }
    let desired = current_desired_row(connection, &fence.subject)
        .map_err(internal)?
        .ok_or_else(|| refused("the argv seat is no longer declared"))?;
    let current: MemberSpec = desired.member.as_deref()
        .map(serde_json::from_str).transpose().map_err(internal)?
        .ok_or_else(|| refused("the argv seat is intentionally stopped"))?;
    if desired.kind == "stop"
        || current.kind != MemberKind::Agent
        || current.host != host
        || current.driver.is_some()
        || current.terminal_binding.is_some()
        || !matches!(&current.launch, LaunchSpec::Argv(argv) if !argv.is_empty())
        || current.kind != captured.kind
        || current.terminal_binding != captured.terminal_binding
        || current.runtime_id != captured.runtime_id
        || !current.launch_changes(captured).is_empty()
        // This is an explicit argv transport, not a typed harness wrapper:
        // every argument remains authoritative, including wrapper-like flags.
        || current.launch != captured.launch
    {
        return Err(refused("the explicit argv seat declaration was stopped or replaced"));
    }
    Ok(())
}

pub fn admit(
    connection: &Connection,
    request: &Fence,
    authority: &Authority,
) -> Result<(), St3Error> {
    admit_with_probe(connection, request, authority, |owner| {
        channel_liveness(owner, st_runtime::process_start_token(owner.pid), || {
            (unsafe { libc::kill(owner.pid as i32, 0) }) != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        })
    })
}

pub fn channel_liveness(
    owner: &Authority,
    birth: anyhow::Result<u64>,
    absent: impl FnOnce() -> bool,
) -> ChannelLiveness {
    match birth {
        Ok(token) if token == owner.process_token => ChannelLiveness::Alive,
        Ok(_) => ChannelLiveness::Dead,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound)
                && absent() =>
        {
            ChannelLiveness::Dead
        }
        Err(_) => ChannelLiveness::Indeterminate,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelLiveness {
    Alive,
    Dead,
    Indeterminate,
}

pub fn admit_with_probe(
    connection: &Connection,
    request: &Fence,
    authority: &Authority,
    liveness: impl FnOnce(&Authority) -> ChannelLiveness,
) -> Result<(), St3Error> {
    if authority.session.is_empty()
        || (authority.sequence == 0
            && (authority.provider != "codex"
                || request.component != "delivery"
                || authority.session != format!("runtime:{}", request.incarnation)))
        || authority.sequence > i64::MAX as u64
    {
        return Err(refused("missing authenticated provider ownership"));
    }
    let Some((owner, prior, revoked)) = lease(connection, request)? else {
        // A restarted registry is not takeover permission. A pre-lease current binding
        // may be authenticated in place, but opaque same-runtime history cannot grant a
        // fresh token seniority over its current owner.
        if request.epoch == 0 {
            let unqualified_owner: Option<String> = connection.query_row(
                "SELECT incarnation FROM local_mailbox_owners WHERE subject=?1 AND component=?2",
                params![request.subject, request.component], |row| row.get(0),
            ).optional().map_err(internal)?;
            if unqualified_owner.as_deref() == Some(&request.incarnation) {
                let current_token: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM local_mailbox_bindings binding JOIN local_mailbox_owners owner
                     ON owner.subject=binding.subject AND owner.component=binding.component AND owner.incarnation=binding.incarnation AND owner.epoch=binding.epoch
                     WHERE binding.token=?1 AND binding.subject=?2 AND binding.component=?3 AND binding.incarnation=?4)",
                    params![request.token,request.subject,request.component,request.incarnation], |row| row.get(0),
                ).map_err(internal)?;
                if !current_token {
                    return Err(refused(
                        "unqualified ownership history does not authorize a fresh takeover",
                    ));
                }
            }
        }
        return Ok(());
    };
    if owner.incarnation != request.incarnation {
        return Ok(());
    }
    if revoked {
        return Err(refused("the mailbox lease was revoked"));
    }
    let bootstrap_promotion = prior.sequence == 0
        && authority.sequence > 0
        && prior.provider == authority.provider
        && prior.pid == authority.pid
        && prior.process_token == authority.process_token;
    if request.token == owner.token && (*authority == prior || bootstrap_promotion) {
        let bound: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM local_mailbox_bindings WHERE token=?1)",
                [&request.token],
                |row| row.get(0),
            )
            .map_err(internal)?;
        return if bound {
            Ok(())
        } else {
            Err(refused("the binding token was revoked"))
        };
    }
    // Only a captured newer provider record, or a provably dead channel process, replaces
    // custody. A duplicate attachment in the same session cannot silence the live channel.
    if authority.provider != prior.provider
        || authority.sequence < prior.sequence
        || (authority.sequence == prior.sequence
            && (authority.session != prior.session || liveness(&prior) != ChannelLiveness::Dead))
    {
        return Err(refused(
            "another live channel holds this provider session's lease",
        ));
    }
    // Old capabilities remain terminal even after the old process disappears.
    if request.epoch != 0 {
        return Err(refused(
            "a retired binding cannot acquire a successor lease",
        ));
    }
    Ok(())
}

pub fn record(
    connection: &Connection,
    fence: &Fence,
    authority: &Authority,
) -> Result<(), St3Error> {
    connection
        .execute(
            /* sqlite-large-value: allow-for-now lv-h17864914d6eb owner=st3_store::store::mailbox_lease TODO=bound-or-reference-values-and-reclamation */ "INSERT INTO local_mailbox_leases VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0)
         ON CONFLICT(subject,component) DO UPDATE SET incarnation=excluded.incarnation,
         provider=excluded.provider,session=excluded.session,sequence=excluded.sequence,
         pid=excluded.pid,process_token=excluded.process_token,token=excluded.token,
         epoch=excluded.epoch,revoked=0",
            params![
                fence.subject,
                fence.component,
                fence.incarnation,
                authority.provider,
                authority.session,
                authority.sequence,
                authority.pid,
                authority.process_token.to_string(),
                fence.token,
                fence.epoch
            ],
        )
        .map_err(internal)?;
    Ok(())
}
