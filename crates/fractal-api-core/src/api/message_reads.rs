//! How far a message got on its recipient's host, read through the seat-freshness relay (db
//! break-up step 4b).
//!
//! A host that advertises `message_offsets` keeps its seats' delivery, read and close of graph
//! mail as local marks and writes no lifecycle claims for them, and every host keeps a stream
//! item's lifecycle in its own shard. Another member that needs that state, such as
//! `conversations status` or doctor's mail checks, asks the host and never guesses. A host that
//! does not answer within the deadline reads as unknown, never as sent.
//!
//! Answers are kept a few seconds, so repeated reads of one message ask the host once, and so is
//! a host's silence, so doctor's checks, one after another, wait for a silent host once. One read
//! asks at most [`MAX_HOSTS`] hosts, all at once, and at most [`MAX_MESSAGES_PER_HOST`] messages
//! of each, so a slow host costs one deadline and a large backlog a bounded number of requests.
//!
//! A message view or list asks the same way ([`read_views`]), but a UI never waits long: it waits
//! [`VIEW_DEADLINE`], and a host that did not answer is not asked again by views for
//! [`VIEW_SILENT_FOR`]. Its messages then read as this member's own records, with
//! `status_from` saying the host's answer is unknown.

use super::*;
use crate::peer::{MAX_READ_MESSAGES, MessageMarksAnswer, SEAT_FRESHNESS_READS_FEATURE};
use crate::model::StatusFrom;
use crate::store::message_marks;
use std::collections::HashMap;

/// How long `conversations status` waits for the recipient's host.
pub const STATUS_DEADLINE: Duration = Duration::from_secs(3);
/// How long a doctor check, cleanup or backlog read waits for every host it asks.
pub const CHECK_DEADLINE: Duration = Duration::from_secs(10);
/// How long a host's answer about a message, or its silence, is reused.
const KEEP_FOR: Duration = Duration::from_secs(5);
/// How long a message view or list waits for every host it asks.
pub const VIEW_DEADLINE: Duration = Duration::from_secs(1);
/// How long views do not ask a host that did not answer.
pub const VIEW_SILENT_FOR: Duration = Duration::from_secs(30);
/// The most answers kept; past it the oldest half goes.
const KEPT_ANSWERS: usize = 8_192;
/// The most hosts one read asks.
pub const MAX_HOSTS: usize = 16;
/// The most messages one read asks of one host: four requests.
pub const MAX_MESSAGES_PER_HOST: usize = 4 * MAX_READ_MESSAGES;
/// The `daemon.started` feature of a host that keeps its seats' lifecycle as local marks.
pub const MESSAGE_OFFSETS_FEATURE: &str = "message_offsets";

/// What a host answered about one message, or why there is no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Relayed {
    /// The host's seats' marks of the message, those the host is declared to hold. `held` is
    /// false when the host does not hold the message at all. `latest` is the newest of those
    /// marks: its receipt and when the host recorded it.
    Answered {
        host: String,
        held: bool,
        status: Option<&'static str>,
        latest: Option<(String, u64)>,
    },
    /// The host did not answer: the reason, naming it.
    Unknown { host: String, reason: String },
}

impl Relayed {
    /// The status the host's seats took the message to, if it answered.
    pub fn status(&self) -> Option<&'static str> {
        match self {
            Self::Answered { status, .. } => *status,
            Self::Unknown { .. } => None,
        }
    }
}

/// Answers this member read from hosts, by message, and the hosts that did not answer, with why,
/// each for [`KEEP_FOR`].
#[derive(Default)]
pub struct KeptAnswers {
    answers: std::sync::Mutex<HashMap<String, (Instant, MessageMarksAnswer)>>,
    silent: std::sync::Mutex<HashMap<String, (Instant, String)>>,
}

impl KeptAnswers {
    /// Why `host` did not answer, if it did not within the last `within`.
    fn silent(&self, host: &str, within: Duration) -> Option<String> {
        let silent = self.silent.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        silent.get(host).filter(|(at, _)| at.elapsed() < within).map(|(_, reason)| reason.clone())
    }

    fn fell_silent(&self, host: &str, reason: &str) {
        let mut silent = self.silent.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        silent.retain(|_, (at, _)| at.elapsed() < VIEW_SILENT_FOR.max(KEEP_FOR));
        silent.insert(host.to_owned(), (Instant::now(), reason.to_owned()));
    }

    fn get(&self, subject: &str) -> Option<MessageMarksAnswer> {
        let kept = self
            .answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        kept.get(subject)
            .filter(|(at, _)| at.elapsed() < KEEP_FOR)
            .map(|(_, answer)| answer.clone())
    }

    fn keep(&self, answers: impl IntoIterator<Item = MessageMarksAnswer>) {
        let mut kept = self
            .answers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        kept.retain(|_, (at, _)| at.elapsed() < KEEP_FOR);
        if kept.len() >= KEPT_ANSWERS {
            let mut ages = kept.values().map(|(at, _)| *at).collect::<Vec<_>>();
            ages.sort_unstable();
            let cutoff = ages[ages.len() / 2];
            kept.retain(|_, (at, _)| *at > cutoff);
        }
        for answer in answers {
            kept.insert(answer.subject.clone(), (now, answer));
        }
    }
}

/// Whether `member`'s own newest `daemon.started` advertises `feature` at `version`.
fn advertises(store: &Store, member: &str, feature: &str, version: u64) -> anyhow::Result<bool> {
    Ok(store
        .latest_claim(&format!("daemon/{member}"), Some("daemon.started"))?
        .is_some_and(|claim| {
            claim.origin == member
                && claim
                    .body
                    .pointer(&format!("/fields/features/{feature}"))
                    .and_then(Value::as_u64)
                    == Some(version)
        }))
}

/// The host to ask about `message`, when another member keeps how far its recipient got and
/// answers reads: a stream item's recipient on another host that answers them, or graph mail
/// whose recipient's host also keeps its lifecycle as marks. `None` reads this member's own
/// state, as before.
pub fn relay_host(store: &Store, message: &MessageView) -> anyhow::Result<Option<String>> {
    let owners = store.desired_subjects_named(std::slice::from_ref(&message.to))?;
    let Some(host) = owners
        .first()
        .and_then(|owner| owner.member.as_ref())
        .map(|member| member.host.clone())
        .filter(|host| host != store.origin())
    else {
        return Ok(None);
    };
    if !advertises(store, &host, SEAT_FRESHNESS_READS_FEATURE, 1)? {
        return Ok(None);
    }
    let stream_item = crate::store::messages_shard::mailbox_reads::is_stream_item(message);
    Ok((stream_item || advertises(store, &host, MESSAGE_OFFSETS_FEATURE, 1)?).then_some(host))
}

/// The other members that keep their seats' graph mail receipts as marks and answer reads, each
/// with the seats this member's declarations place on it. Empty until a host advertises
/// `message_offsets`, so a reader that finds none asks nobody and reads no declaration.
pub fn relay_hosts(store: &Store) -> anyhow::Result<BTreeMap<String, BTreeSet<String>>> {
    let any_offsets: bool = store
        .readers
        .get()
        .prepare_cached(/* sqlite-large-value: allow-for-now lv-he830e5bf3588 owner=fractal_api_core::api::message_reads TODO=trace-statement-and-value-representation */ &format!(
            "SELECT EXISTS(SELECT 1 FROM claims WHERE kind='daemon.started'
               AND json_extract(body,'$.fields.features.{MESSAGE_OFFSETS_FEATURE}')=1)"
        ))?
        .query_row(/* sqlite-large-value: allow-for-now lv-h17ed23f52569 owner=fractal_api_core::api::message_reads TODO=trace-statement-and-value-representation */ [], |row| row.get(0))?;
    if !any_offsets {
        return Ok(BTreeMap::new());
    }
    let mut hosts = BTreeMap::<String, BTreeSet<String>>::new();
    for desired in store.desired_subjects()? {
        if let Some(member) = desired
            .member
            .as_ref()
            .filter(|member| member.host != store.origin())
        {
            hosts
                .entry(member.host.clone())
                .or_default()
                .insert(desired.subject);
        }
    }
    let mut relaying = BTreeMap::new();
    for (host, seats) in hosts {
        if advertises(store, &host, SEAT_FRESHNESS_READS_FEATURE, 1)?
            && advertises(store, &host, MESSAGE_OFFSETS_FEATURE, 1)?
        {
            relaying.insert(host, seats);
        }
    }
    Ok(relaying)
}

/// The seats this member's declarations place on `host`.
fn seats_on(store: &Store, host: &str) -> anyhow::Result<BTreeSet<String>> {
    Ok(store
        .desired_subjects()?
        .into_iter()
        .filter(|desired| {
            desired
                .member
                .as_ref()
                .is_some_and(|member| member.host == host)
        })
        .map(|desired| desired.subject)
        .collect())
}

/// The newest of `answer`'s marks of seats this member places on the host: its receipt and when
/// it was recorded.
fn latest_of(answer: &MessageMarksAnswer, seats: &BTreeSet<String>) -> Option<(String, u64)> {
    answer
        .marks
        .iter()
        .filter(|mark| seats.contains(&mark.consumer))
        .max_by_key(|mark| mark.recorded_at_unix_ms)
        .map(|mark| (mark.receipt.clone(), mark.recorded_at_unix_ms))
}

/// How far `answer`'s marks took the message, keeping only those of seats this member places on
/// `host`: a host speaks for its own seats.
fn status_of(answer: &MessageMarksAnswer, seats: &BTreeSet<String>) -> Option<&'static str> {
    answer
        .marks
        .iter()
        .filter(|mark| seats.contains(&mark.consumer))
        .filter_map(|mark| message_marks::MARKS.iter().find(|kind| **kind == mark.kind))
        .max_by_key(|kind| message_marks::rank(kind.trim_start_matches("message.")))
        .map(|kind| kind.trim_start_matches("message."))
}

/// Ask each host about its messages, all hosts at once, within `deadline`. Every message asked
/// has an answer: what its host said, or why it is unknown.
pub async fn read(
    state: &AppState,
    asks: BTreeMap<String, Vec<String>>,
    deadline: Duration,
) -> BTreeMap<String, Relayed> {
    read_paced(state, asks, deadline, KEEP_FOR).await
}

/// [`read`], not asking a host that did not answer within the last `silent_for`.
async fn read_paced(
    state: &AppState,
    asks: BTreeMap<String, Vec<String>>,
    deadline: Duration,
    silent_for: Duration,
) -> BTreeMap<String, Relayed> {
    let kept = state.store.extension::<KeptAnswers>();
    // Each message's host, and what it answered or why it did not.
    let mut heard = Vec::<Heard>::new();
    let mut requests = Vec::new();
    for (position, (host, subjects)) in asks.into_iter().enumerate() {
        if position >= MAX_HOSTS {
            let reason = format!("one read asks at most {MAX_HOSTS} hosts");
            heard.extend(
                subjects
                    .into_iter()
                    .map(|subject| (host.clone(), Err((subject, reason.clone())))),
            );
            continue;
        }
        let mut asked = Vec::new();
        let silent = kept.silent(&host, silent_for);
        for subject in subjects {
            match (kept.get(&subject), &silent) {
                (Some(answer), _) => heard.push((host.clone(), Ok(answer))),
                (None, Some(reason)) => heard.push((host.clone(), Err((subject, reason.clone())))),
                (None, None) if asked.len() < MAX_MESSAGES_PER_HOST => asked.push(subject),
                (None, None) => {
                    let reason =
                        format!("one read asks at most {MAX_MESSAGES_PER_HOST} messages of a host");
                    heard.push((host.clone(), Err((subject, reason))));
                }
            }
        }
        for chunk in asked.chunks(MAX_READ_MESSAGES) {
            requests.push((host.clone(), chunk.to_vec()));
        }
    }
    match state.client_relay.clone() {
        None => {
            for (host, subjects) in requests {
                let reason = "this member has no peer relay".to_owned();
                heard.extend(
                    subjects
                        .into_iter()
                        .map(|subject| (host.clone(), Err((subject, reason.clone())))),
                );
            }
        }
        Some(relay) => {
            let answers =
                futures_util::future::join_all(requests.into_iter().map(|(host, subjects)| {
                    let relay = relay.clone();
                    async move {
                        let answer = tokio::time::timeout(
                            deadline,
                            relay.read_message_marks(&host, &subjects, deadline),
                        )
                        .await;
                        (host, subjects, answer)
                    }
                }))
                .await;
            for (host, subjects, answer) in answers {
                let reason = match answer {
                    Ok(Ok(Some(answers))) => {
                        kept.keep(answers.messages.iter().cloned());
                        heard.extend(
                            answers
                                .messages
                                .into_iter()
                                .map(|answer| (host.clone(), Ok(answer))),
                        );
                        continue;
                    }
                    Ok(Ok(None)) => {
                        format!("{host} cannot be reached directly or does not answer reads")
                    }
                    Ok(Err(error)) => format!("{host} did not answer: {error:#}"),
                    Err(_) => format!("{host} did not answer in {deadline:?}"),
                };
                kept.fell_silent(&host, &reason);
                heard.extend(
                    subjects
                        .into_iter()
                        .map(|subject| (host.clone(), Err((subject, reason.clone())))),
                );
            }
        }
    }
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || settle(&store, heard))
        .await
        .unwrap_or_default()
}

/// A message's host, and what it answered or, with the message, why it did not.
type Heard = (String, Result<MessageMarksAnswer, (String, String)>);

/// What each message's host said, as a status over the seats this member places on that host.
fn settle(
    store: &Store,
    heard: Vec<Heard>,
) -> BTreeMap<String, Relayed> {
    let mut seats = BTreeMap::<String, Result<BTreeSet<String>, String>>::new();
    heard
        .into_iter()
        .map(|(host, heard)| match heard {
            Ok(answer) => {
                let on_host = seats.entry(host.clone()).or_insert_with(|| {
                    seats_on(store, &host).map_err(|error| format!("{error:#}"))
                });
                let relayed = match on_host {
                    Ok(on_host) => Relayed::Answered {
                        host,
                        held: answer.held,
                        status: status_of(&answer, on_host),
                        latest: latest_of(&answer, on_host),
                    },
                    Err(reason) => Relayed::Unknown {
                        host,
                        reason: reason.clone(),
                    },
                };
                (answer.subject, relayed)
            }
            Err((subject, reason)) => (subject, Relayed::Unknown { host, reason }),
        })
        .collect()
}

/// [`read`] from a thread outside the runtime, such as a doctor worker, through the runtime
/// `handle` it was started from. Without one, or when the answer does not come back within the
/// deadline, every message reads as unknown.
pub fn read_blocking(
    handle: Option<&tokio::runtime::Handle>,
    state: &AppState,
    asks: BTreeMap<String, Vec<String>>,
    deadline: Duration,
) -> BTreeMap<String, Relayed> {
    if asks.is_empty() {
        return BTreeMap::new();
    }
    let unknown = |asks: BTreeMap<String, Vec<String>>, reason: &str| {
        asks.into_iter()
            .flat_map(|(host, subjects)| {
                let reason = reason.to_owned();
                subjects.into_iter().map(move |subject| {
                    (
                        subject,
                        Relayed::Unknown {
                            host: host.clone(),
                            reason: reason.clone(),
                        },
                    )
                })
            })
            .collect()
    };
    let Some(handle) = handle else {
        return unknown(asks, "no runtime to ask the host from");
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    let (state_copy, asks_copy) = (state.clone(), asks.clone());
    handle.spawn(async move {
        let _ = sender.send(read(&state_copy, asks_copy, deadline).await);
    });
    receiver
        .recv_timeout(deadline + Duration::from_secs(1))
        .unwrap_or_else(|_| unknown(asks, "the host's answer did not come back in time"))
}

/// The messages among `messages` to ask about, grouped by host, and the host of each.
pub fn asks_for<'a>(
    store: &Store,
    messages: impl IntoIterator<Item = &'a MessageView>,
) -> anyhow::Result<BTreeMap<String, Vec<String>>> {
    let mut asks = BTreeMap::<String, Vec<String>>::new();
    // Messages to one recipient, of one kind, share a host.
    let mut hosts = HashMap::<(String, bool), Option<String>>::new();
    for message in messages {
        let kind = (
            message.to.clone(),
            crate::store::messages_shard::mailbox_reads::is_stream_item(message),
        );
        let host = match hosts.get(&kind) {
            Some(host) => host.clone(),
            None => {
                let host = relay_host(store, message)?;
                hosts.insert(kind, host.clone());
                host
            }
        };
        if let Some(host) = host {
            asks.entry(host).or_default().push(message.subject.clone());
        }
    }
    Ok(asks)
}

/// Ask the hosts of `messages` how far each got, within [`VIEW_DEADLINE`], and take each
/// answer in: a message whose seat's host keeps its receipts is as far as the further of this
/// member's records and the host's answer, and says in `status_from` which host it asked and
/// whether that host answered. A message the host says is closed leaves the list when
/// `drop_closed`, as a list of open mail drops closed mail of its own. Every other message is
/// left as it is.
pub async fn read_views(
    state: &AppState,
    messages: &mut Vec<MessageView>,
    drop_closed: bool,
) -> anyhow::Result<()> {
    let relayed = read_for_views(state, messages).await?;
    let mut closed = BTreeSet::new();
    for message in messages.iter_mut() {
        let Some(relayed) = relayed.get(&message.subject) else {
            continue;
        };
        take_in(message, relayed);
        if message.status == "closed" {
            closed.insert(message.subject.clone());
        }
    }
    if drop_closed {
        messages.retain(|message| !closed.contains(&message.subject));
    }
    Ok(())
}

/// What the hosts of `messages` said about them, within [`VIEW_DEADLINE`], for the messages whose
/// seat's host keeps their receipts. Asks nobody when none of them has such a host.
pub async fn read_for_views(
    state: &AppState,
    messages: &[MessageView],
) -> anyhow::Result<BTreeMap<String, Relayed>> {
    if messages.is_empty() {
        return Ok(BTreeMap::new());
    }
    let asks = {
        let store = state.store.clone();
        let views = messages.to_vec();
        tokio::task::spawn_blocking(move || asks_for(&store, views.iter())).await??
    };
    if asks.is_empty() {
        return Ok(BTreeMap::new());
    }
    Ok(read_paced(state, asks, VIEW_DEADLINE, VIEW_SILENT_FOR).await)
}

/// [`read_views`] for one message, which is never dropped.
pub async fn read_view(state: &AppState, message: &mut MessageView) -> anyhow::Result<()> {
    let mut messages = vec![message.clone()];
    read_views(state, &mut messages, false).await?;
    if let Some(read) = messages.pop() {
        *message = read;
    }
    Ok(())
}

/// Take a host's answer about `message` in.
pub fn take_in(message: &mut MessageView, relayed: &Relayed) {
    message.status_from = Some(match relayed {
        Relayed::Answered { host, status, .. } => {
            if let Some(status) = status
                && message_marks::rank(status) > message_marks::rank(&message.status)
            {
                message.status = (*status).to_owned();
            }
            StatusFrom {
                host: host.clone(),
                state: "answered".into(),
                reason: None,
            }
        }
        Relayed::Unknown { host, reason } => StatusFrom {
            host: host.clone(),
            state: "unknown".into(),
            reason: Some(reason.clone()),
        },
    });
}
