//! How far a message got on its recipient's host, read through the seat-freshness relay (db
//! break-up step 4b).
//!
//! A host that advertises `message_offsets` keeps its seats' delivery, read and close of graph
//! mail as local marks and writes no lifecycle claims for them, and every host keeps a stream
//! item's lifecycle in its own shard. Another member that needs that state, such as
//! `conversations status` or doctor's mail checks, asks the host and never guesses. A host that
//! does not answer within the deadline reads as unknown, never as sent.
//!
//! Answers are kept a few seconds, so repeated reads of one message ask the host once. One read
//! asks at most [`MAX_HOSTS`] hosts, all at once, and at most [`MAX_MESSAGES_PER_HOST`] messages
//! of each, so a slow host costs one deadline and a large backlog a bounded number of requests.

use super::*;
use crate::peer::{MAX_READ_MESSAGES, MessageMarksAnswer, SEAT_FRESHNESS_READS_FEATURE};
use crate::store::message_marks;
use std::collections::HashMap;

/// How long `conversations status` waits for the recipient's host.
pub const STATUS_DEADLINE: Duration = Duration::from_secs(3);
/// How long a doctor check, cleanup or backlog read waits for every host it asks.
pub const CHECK_DEADLINE: Duration = Duration::from_secs(10);
/// How long a host's answer about a message is reused.
const KEEP_FOR: Duration = Duration::from_secs(5);
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
    /// false when the host does not hold the message at all.
    Answered {
        host: String,
        held: bool,
        status: Option<&'static str>,
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

/// Answers this member read from hosts, by message, for [`KEEP_FOR`].
#[derive(Default)]
pub struct KeptAnswers(std::sync::Mutex<HashMap<String, (Instant, MessageMarksAnswer)>>);

impl KeptAnswers {
    fn get(&self, subject: &str) -> Option<MessageMarksAnswer> {
        let kept = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        kept.get(subject)
            .filter(|(at, _)| at.elapsed() < KEEP_FOR)
            .map(|(_, answer)| answer.clone())
    }

    fn keep(&self, answers: impl IntoIterator<Item = MessageMarksAnswer>) {
        let mut kept = self
            .0
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
/// `message_offsets`, so a reader that finds none asks nobody.
pub fn relay_hosts(store: &Store) -> anyhow::Result<BTreeMap<String, BTreeSet<String>>> {
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
    let kept = state.store.extension::<KeptAnswers>();
    // Each message's host, and what it answered or why it did not.
    let mut heard = Vec::<(String, Result<MessageMarksAnswer, (String, String)>)>::new();
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
        for subject in subjects {
            match kept.get(&subject) {
                Some(answer) => heard.push((host.clone(), Ok(answer))),
                None if asked.len() < MAX_MESSAGES_PER_HOST => asked.push(subject),
                None => {
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

/// What each message's host said, as a status over the seats this member places on that host.
fn settle(
    store: &Store,
    heard: Vec<(String, Result<MessageMarksAnswer, (String, String)>)>,
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
    for message in messages {
        if let Some(host) = relay_host(store, message)? {
            asks.entry(host).or_default().push(message.subject.clone());
        }
    }
    Ok(asks)
}
