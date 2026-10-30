//! A harness-prompt card's prompt: the call the harness asks about while it has no result in the
//! seat's conversation, the answers st offers and whether a client may answer it, or why st
//! cannot read the conversation (docs/clients/ui-contract.md, Attention row).

use super::*;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use notify::Watcher as _;

/// The most of a call's input a card carries, as JSON text.
const INPUT_BYTES: usize = 4 * 1024;

/// The longest a prompt card shows the calls it read from a transcript here without reading it
/// again, though no change to it was seen: a change whose event was lost shows within this.
const CALLS_HERE_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(60);

/// How far back st looks for a call without a result: the conversation's newest entries, a page
/// as a client or a peer reads it. A harness waits on the calls it made last, so an older one
/// without a result was abandoned.
const TAIL: usize = super::CLIENT_MAX_PAGE_ITEMS;

/// Reads the tool calls without a result in a seat's conversation, oldest first, each
/// `{call_id, name, arguments, complete}`, or says why st cannot.
pub(crate) type PendingCalls<'a> = &'a dyn Fn(&str) -> Result<Vec<Value>, String>;

/// The prompt on a harness-prompt row's presentation. `pending` reads the seat's calls without a
/// result, and only when the harness names the call it asks about.
pub(crate) fn prompt_card(row: &Value, pending: impl FnOnce() -> Result<Vec<Value>, String>) -> Value {
    let respond = &row["action_parameters"]["prompt.respond"];
    let answers = respond["answers"].as_array().cloned().unwrap_or_default();
    // Only the call st names: any other call without a result (a parent's open Task call, or one
    // whose result st has not read yet) may not be the one asked about. A prompt that names none
    // shows no call and is answered in its terminal.
    let Some(named) = respond["call_id"].as_str() else {
        return json!({"calls": [], "answers": answers, "allow": false});
    };
    let calls = match pending() {
        Ok(calls) => calls,
        Err(reason) => {
            return json!({"calls": null, "answers": answers, "allow": false, "unknown_reason": reason});
        }
    };
    let calls = calls
        .iter()
        .filter(|call| call["call_id"] == named)
        .take(1)
        .map(|call| {
            let input = match &call["arguments"] {
                Value::String(text) => text.clone(),
                arguments => arguments.to_string(),
            };
            let cut = cut(&input, INPUT_BYTES);
            json!({
                "call_id": call["call_id"],
                "tool": call["name"],
                "input": cut,
                // The timeline may already hold only a bounded preview of the call.
                "input_complete": call["complete"] != false && cut.len() == input.len(),
            })
        })
        .collect::<Vec<_>>();
    // A client answers only the call its person can read in full.
    let allow = !answers.is_empty() && calls.first().is_some_and(|call| call["input_complete"] == true);
    json!({"calls": calls, "answers": answers, "allow": allow})
}

/// `text` within `bytes`, cut at a character boundary.
fn cut(text: &str, bytes: usize) -> &str {
    if text.len() <= bytes {
        return text;
    }
    let mut end = bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The tool calls without a result among a conversation's entries, newest first as a slice
/// reads them: oldest first, each once. A call is not `complete` when the timeline holds only a
/// bounded preview of its record (`_preview_of_bytes`) or a truncation marker in place of its
/// arguments (`_oversized_bytes`).
fn pending_calls(items: &[Value]) -> Vec<Value> {
    let answered = items
        .iter()
        .filter(|item| item["type"] == "tool_result")
        .filter_map(|item| item["body"]["call_id"].as_str())
        .collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut calls = items
        .iter()
        .filter(|item| item["type"] == "tool_call")
        .filter(|item| {
            item["body"]["call_id"]
                .as_str()
                .is_some_and(|id| !answered.contains(id) && seen.insert(id))
        })
        .map(|item| {
            let body = &item["body"];
            let complete = !(item["_preview_of_bytes"].is_number() || item["_oversized_bytes"].is_number());
            json!({"call_id": body["call_id"], "name": body["name"], "arguments": body["arguments"], "complete": complete})
        })
        .collect::<Vec<_>>();
    calls.reverse();
    calls
}

/// The tool calls without a result in `seat`'s conversation, read on this node or, when another
/// member runs the seat, through the client-read relay from that host; or why st cannot read
/// it: the seat has no session yet, its harness keeps none st reads, or its host cannot be
/// reached. `follow` is given the conversation before it is read, so a change after the read is
/// never missed; when st cannot follow it, the card says so instead. For a transcript here it
/// returns the calls it read last while nothing changed the transcript since, or `None` for the
/// card to read it now. For a conversation another member keeps it returns the calls its
/// follower read last: the fold never waits on another member.
pub fn seat_pending_calls(
    state: &AppState,
    seat: &str,
    follow: impl FnOnce(Follow<'_>) -> Result<Option<Vec<Value>>, String>,
) -> Result<Vec<Value>, String> {
    match seat_conversation(state, seat)? {
        Conversation::Here(source) => match follow(Follow::Transcript(&source))? {
            Some(calls) => Ok(calls),
            None => Ok(pending_calls(&tail(&source)?)),
        },
        Conversation::Elsewhere(remote) => follow(Follow::Remote(&remote))?
            .ok_or_else(|| format!("st cannot follow this seat's conversation on {}", remote.host)),
    }
}

/// What a prompt card follows for changes: a transcript on this node, or a conversation another
/// member keeps.
pub enum Follow<'a> {
    Transcript(&'a crate::external_sessions::ExternalSession),
    Remote(&'a RemoteConversation),
}

/// Where a seat's conversation is kept.
enum Conversation {
    Here(crate::external_sessions::ExternalSession),
    Elsewhere(RemoteConversation),
}

/// A seat's conversation another member keeps, read through the client-read relay as the seat's
/// person, as their own client's reads of it are.
#[derive(Clone)]
pub struct RemoteConversation {
    relay: crate::peer::ClientRelay,
    host: String,
    session_id: String,
    person: String,
}

/// The most a relayed read waits for a change (the relay's own limit).
const REMOTE_CHANGE_WAIT_MS: u64 = 10_000;

/// How long a follower whose relayed read failed waits before reading again, doubling to the
/// most while its host stays out of reach.
const REMOTE_RETRY: std::time::Duration = std::time::Duration::from_secs(1);
const REMOTE_RETRY_MOST: std::time::Duration = std::time::Duration::from_secs(30);

impl RemoteConversation {
    async fn read(&self, request: crate::peer::ClientReadOperation) -> Result<Value, String> {
        let request = crate::peer::ClientReadRequest { authority_actor: self.person.clone(), relay: None, request };
        self.relay
            .read(&self.host, &request)
            .await
            .map_err(|error| format!("st cannot read this seat's conversation on {}: {error:#}", self.host))
    }

    /// The calls without a result among the conversation's newest entries, and where the
    /// conversation was before they were read, for the follower to wait on changes from.
    async fn pending(&self) -> Result<(Option<String>, Vec<Value>), String> {
        let (after, _) = self.changes(None, 0).await?;
        let page = self
            .read(crate::peer::ClientReadOperation::Timeline {
                session_id: self.session_id.clone(),
                limit: TAIL,
                cursor: None,
            })
            .await?;
        Ok((after, pending_calls(page["items"].as_array().map(Vec::as_slice).unwrap_or_default())))
    }

    /// The conversation's cursor after `after`, waiting up to `wait_ms` for a change, and
    /// whether it changed.
    async fn changes(&self, after: Option<String>, wait_ms: u64) -> Result<(Option<String>, bool), String> {
        let changes = self
            .read(crate::peer::ClientReadOperation::ConversationChanges {
                session_id: self.session_id.clone(),
                after: after.clone(),
                wait_ms,
            })
            .await?;
        let changed = changes["items"].as_array().is_some_and(|items| !items.is_empty());
        Ok((changes["next_cursor"].as_str().map(str::to_owned).or(after), changed))
    }

    /// Read the conversation, publish its calls in `latest` and wake the refresher whenever they
    /// change, until the follower is stopped. A read that fails is published too, so a card
    /// offers no answer from what it read before, and read again after a pause.
    async fn follow(self, shared: Arc<Delivery>, latest: Arc<Mutex<Option<RemoteRead>>>) {
        let publish = |read: RemoteRead| {
            let mut latest = latest.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if latest.as_ref() != Some(&read) {
                *latest = Some(read);
                drop(latest);
                shared.changed();
            }
        };
        let mut pause = REMOTE_RETRY;
        loop {
            let failed = match self.pending().await {
                Ok((mut after, calls)) => {
                    publish(Ok(calls));
                    pause = REMOTE_RETRY;
                    loop {
                        match self.changes(after, REMOTE_CHANGE_WAIT_MS).await {
                            Ok((_, true)) => break None,
                            Ok((next, false)) => after = next,
                            Err(error) => break Some(error),
                        }
                    }
                }
                Err(error) => Some(error),
            };
            if let Some(error) = failed {
                publish(Err(error));
                tokio::time::sleep(pause).await;
                pause = (pause * 2).min(REMOTE_RETRY_MOST);
            }
        }
    }

    /// A follower reads as the person it was made for: when the seat's person changes, a new
    /// follower reads as the new one, and the old one, read by no card, is stopped.
    fn key(&self) -> RemoteKey {
        (self.host.clone(), self.session_id.clone(), self.person.clone())
    }
}

/// Where `seat`'s conversation is kept, or why st cannot read it.
fn seat_conversation(state: &AppState, seat: &str) -> Result<Conversation, String> {
    let unreadable = |error: ApiError| format!("st cannot read this seat's conversation: {}", error.message);
    let session = super::client_v0::conversation_session_id(state, seat).map_err(unreadable)?;
    let index = state.store.index().map_err(|error| format!("st cannot read this seat's conversation: {error}"))?;
    let routing = super::client_v0::routing_session_owner_at(&state.store, index, &session)
        .map_err(|error| format!("st cannot read this seat's conversation: {error}"))?;
    let origin = routing.as_ref().and_then(|(_, _, origin)| origin.as_deref());
    if let Some(origin) = origin.filter(|origin| *origin != state.store.origin()) {
        let host = super::client_host_id(origin);
        let relay = state
            .client_relay
            .clone()
            .filter(|relay| relay.reaches(&host))
            .ok_or_else(|| format!("st cannot read this seat's conversation: no peer reaches {host}, which runs it"))?;
        let person = state
            .store
            .agent_person(seat)
            .map_err(|error| format!("st cannot read this seat's conversation: {error}"))?
            .ok_or_else(|| format!("st cannot read this seat's conversation on {host}: the seat has no person to read it as"))?;
        return Ok(Conversation::Elsewhere(RemoteConversation { relay, host, session_id: session, person }));
    }
    let managed = managed_session_owner_at(&state.store, index, &session)
        .map_err(|error| format!("st cannot read this seat's conversation: {error}"))?;
    super::client_v0::conversation_blocks::source(state, &session, managed.as_ref())
        .map(Conversation::Here)
        .map_err(unreadable)
}

#[cfg(test)]
thread_local! {
    /// How many times this thread read a conversation's newest entries.
    static TAIL_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The newest entries of a conversation, as a prompt card reads them.
fn tail(source: &crate::external_sessions::ExternalSession) -> Result<Vec<Value>, String> {
    #[cfg(test)]
    TAIL_READS.with(|reads| reads.set(reads.get() + 1));
    let timeline = crate::external_sessions::LineTimeline::acquire(source);
    let slice = timeline
        .hold()
        .slice(source, crate::external_sessions::TimelineOrder::Sequence, None, TAIL)
        .map_err(|error| format!("st cannot read this seat's conversation: {error}"))?;
    Ok(slice.items)
}

/// Whether `call` waits among a conversation's newest entries `items` (a page, in either
/// order): made, with no result. A card may have shown it as waiting from a read before its
/// result, so an answer that lets it through checks again; a call that has its result, or that
/// a card could not have shown, does not wait.
pub fn call_waits_among(items: &[Value], call: &str) -> bool {
    pending_calls(items).iter().any(|pending| pending["call_id"] == call)
}

/// The tool calls without a result in a conversation `source`, following it from before the
/// read.
pub(crate) fn source_pending_calls(
    source: &crate::external_sessions::ExternalSession,
    follow: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<Vec<Value>, String> {
    follow(&source.transcript)?;
    Ok(pending_calls(&tail(source)?))
}

/// Wakes the attention refresher when a conversation a published prompt card read changes: a
/// seat's conversation changes no claim attention reads. It watches each transcript's
/// directory, so a transcript created or replaced later, and a SQLite source's write-ahead log
/// and journal beside it, are followed too. An event the watcher could not deliver in full (an
/// error, an overflow that asks for a rescan, or one that names no path) or a change to a watched
/// directory itself (removed or renamed, which drops its watch) means a change may have been
/// lost and the subscriptions may no longer hold: the next refresh makes the watcher again and
/// subscribes each card's conversation anew before reading it. Reading a watched directory
/// (listing it, or measuring it with `du`) changes nothing. A change to what is watched
/// counts as a change too: FSEvents (macOS) restarts its stream from now on every watch and
/// unwatch, so an event around it can be lost. A transcript it cannot watch is not followed:
/// its card says so and offers no answer.
///
/// A transcript here is read again only after a change to it, a lost event, a change to what is
/// watched, a refresh in which no card read it, or a minute since its last read (in case an event
/// was lost unreported); until then each refresh takes the calls it read last, so a fold does not
/// reread every card's conversation.
///
/// A conversation another member keeps is followed through the client-read relay by a task of
/// its own: it reads the conversation, waits on its changes from where it was before that read,
/// and reads it again when any arrive, waking the refresher whenever the calls it read change.
/// The refresher's fold only takes the task's latest read, so a host that is slow or out of
/// reach delays no other card. Until a first read arrives, or while the host cannot be read,
/// the card says so and offers no answer.
pub struct PromptWatch {
    shared: Arc<Delivery>,
    watcher: Option<notify::RecommendedWatcher>,
    /// The transcripts followed, and those the refresh in progress read.
    followed: HashSet<PathBuf>,
    read: HashSet<PathBuf>,
    /// The conversations other members keep that are followed, by host, session and the person
    /// read as, and those the refresh in progress read.
    remote: std::collections::HashMap<RemoteKey, RemoteFollower>,
    remote_read: HashSet<RemoteKey>,
    /// The calls without a result each followed transcript held when it was last read, and when.
    /// An entry stands only while no change to its transcript has been seen since that read, and
    /// for at most `calls_max_age`.
    calls: std::collections::HashMap<PathBuf, (std::time::Instant, Vec<Value>)>,
    calls_max_age: std::time::Duration,
    /// How many times the watcher was made again after a delivery failure.
    rebuilds: usize,
}

/// What the watcher's callback shares with the refresher.
struct Delivery {
    wake: Arc<tokio::sync::Notify>,
    /// The files whose changes wake the refresher: each followed transcript and its siblings.
    files: Mutex<HashSet<PathBuf>>,
    /// Those of them that changed since a card last read their transcript.
    dirty: Mutex<HashSet<PathBuf>>,
    /// The directories watched: an event on one itself drops or moves its watch.
    directories: Mutex<HashSet<PathBuf>>,
    changed: AtomicBool,
    /// The watcher reported an error or lost events: no subscription is trusted until made again.
    broken: AtomicBool,
}

/// The calls a follower read last from a conversation another member keeps, or why it could not.
type RemoteRead = Result<Vec<Value>, String>;

/// A followed conversation another member keeps: its host, session and the person read as.
type RemoteKey = (String, String, String);

/// A task following a conversation another member keeps, and the calls it read last or why it
/// could not.
struct RemoteFollower {
    task: tokio::task::JoinHandle<()>,
    latest: Arc<Mutex<Option<RemoteRead>>>,
}

impl Drop for PromptWatch {
    fn drop(&mut self) {
        for follower in self.remote.values() {
            follower.task.abort();
        }
    }
}

impl Delivery {
    fn changed(&self) {
        self.changed.store(true, Ordering::Release);
        self.wake.notify_one();
    }

    fn deliver(&self, event: notify::Result<notify::Event>) {
        let relevant = match &event {
            Err(_) => {
                self.broken.store(true, Ordering::Release);
                true
            }
            Ok(event)
                if event.need_rescan()
                    || !matches!(event.kind, notify::EventKind::Access(_))
                        && (event.paths.is_empty() || {
                            let directories =
                                self.directories.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                            event.paths.iter().any(|path| directories.contains(path))
                        }) =>
            {
                self.broken.store(true, Ordering::Release);
                true
            }
            Ok(event) => {
                !matches!(event.kind, notify::EventKind::Access(_)) && {
                    let files = self.files.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let changed = event.paths.iter().filter(|path| files.contains(*path)).cloned().collect::<Vec<_>>();
                    drop(files);
                    !changed.is_empty() && {
                        self.dirty.lock().unwrap_or_else(std::sync::PoisonError::into_inner).extend(changed);
                        true
                    }
                }
            }
        };
        if relevant {
            self.changed();
        }
    }
}

/// A transcript and the files beside it a SQLite source commits through.
fn change_files(transcript: &Path) -> [PathBuf; 3] {
    let beside = |suffix: &str| {
        let mut name = OsString::from(transcript.as_os_str());
        name.push(suffix);
        PathBuf::from(name)
    };
    [transcript.to_owned(), beside("-wal"), beside("-journal")]
}

impl PromptWatch {
    pub fn new(wake: Arc<tokio::sync::Notify>) -> Self {
        let mut watch = Self {
            shared: Arc::new(Delivery {
                wake,
                files: Mutex::default(),
                dirty: Mutex::default(),
                directories: Mutex::default(),
                changed: AtomicBool::new(false),
                broken: AtomicBool::new(false),
            }),
            watcher: None,
            followed: HashSet::new(),
            read: HashSet::new(),
            remote: std::collections::HashMap::new(),
            remote_read: HashSet::new(),
            calls: std::collections::HashMap::new(),
            calls_max_age: CALLS_HERE_MAX_AGE,
            rebuilds: 0,
        };
        // A watcher that cannot be made now is made again when a card first follows a seat.
        let _ = watch.watcher();
        watch
    }

    fn watcher(&mut self) -> Result<&mut notify::RecommendedWatcher, String> {
        if self.watcher.is_none() {
            let shared = Arc::clone(&self.shared);
            let watcher = notify::recommended_watcher(move |event| shared.deliver(event))
            .map_err(|error| format!("st cannot follow this seat's conversation: {error}"))?;
            self.watcher = Some(watcher);
        }
        Ok(self.watcher.as_mut().expect("made above"))
    }

    /// Whether a followed conversation changed since this was last asked.
    pub fn take_changed(&self) -> bool {
        self.shared.changed.swap(false, Ordering::AcqRel)
    }

    /// Ask again at the next refresh: the refresh that read the change failed.
    pub(crate) fn keep_changed(&self) {
        self.shared.changed.store(true, Ordering::Release);
    }

    /// What is watched changed: read again soon, since an event around the change may be lost.
    fn watched_changed(&mut self) {
        self.calls.clear();
        self.keep_changed();
        self.shared.wake.notify_one();
    }

    /// A refresh is about to present the prompts. After a delivery failure nothing followed is
    /// trusted: the watcher is made again, and each card subscribes anew before it reads.
    pub fn begin(&mut self) {
        self.read.clear();
        self.remote_read.clear();
        if self.shared.broken.swap(false, Ordering::AcqRel) {
            self.watcher = None;
            self.followed.clear();
            self.calls.clear();
            self.rebuilds += 1;
            self.shared.files.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
            self.shared.dirty.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
            self.shared.directories.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clear();
        }
    }

    /// Follow a card's conversation from now on, before it is read, and give its calls without a
    /// result: a transcript here as read last while nothing changed it, else read now; a
    /// conversation another member keeps as its follower read it last.
    pub fn follow_conversation(&mut self, conversation: Follow<'_>) -> Result<Option<Vec<Value>>, String> {
        match conversation {
            Follow::Transcript(source) => {
                self.follow(&source.transcript)?;
                self.calls_here(source).map(Some)
            }
            Follow::Remote(remote) => self.follow_remote(remote).map(Some),
        }
    }

    /// The calls without a result in a followed transcript here. A change seen since the last
    /// read, a watcher that lost events, or a read older than `calls_max_age` has it read again; a
    /// change during the read leaves it to be read again at the next refresh, which that change
    /// wakes.
    fn calls_here(&mut self, source: &crate::external_sessions::ExternalSession) -> Result<Vec<Value>, String> {
        let mut changed = false;
        let mut dirty = self.shared.dirty.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        for file in change_files(&source.transcript) {
            changed |= dirty.remove(&file);
        }
        drop(dirty);
        if !changed
            && !self.shared.broken.load(Ordering::Acquire)
            && let Some((read_at, calls)) = self.calls.get(&source.transcript)
            && read_at.elapsed() < self.calls_max_age
        {
            return Ok(calls.clone());
        }
        let read_at = std::time::Instant::now();
        match tail(source) {
            Ok(items) => {
                let calls = pending_calls(&items);
                self.calls.insert(source.transcript.clone(), (read_at, calls.clone()));
                Ok(calls)
            }
            Err(error) => {
                self.calls.remove(&source.transcript);
                Err(error)
            }
        }
    }

    /// Follow a conversation another member keeps, starting its follower the first time, and
    /// give the calls it read last.
    fn follow_remote(&mut self, remote: &RemoteConversation) -> Result<Vec<Value>, String> {
        let key = remote.key();
        let follower = match self.remote.entry(key.clone()) {
            std::collections::hash_map::Entry::Occupied(follower) => follower.into_mut(),
            std::collections::hash_map::Entry::Vacant(vacant) => {
                let runtime = tokio::runtime::Handle::try_current()
                    .map_err(|_| format!("st cannot follow this seat's conversation on {}: no runtime", remote.host))?;
                let latest = Arc::new(Mutex::new(None));
                let task = runtime.spawn(remote.clone().follow(Arc::clone(&self.shared), Arc::clone(&latest)));
                vacant.insert(RemoteFollower { task, latest })
            }
        };
        let latest = follower.latest.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
        self.remote_read.insert(key);
        latest.unwrap_or_else(|| Err(format!("st is still reading this seat's conversation on {}", remote.host)))
    }

    /// Follow `transcript` from now on, before it is read.
    pub(crate) fn follow(&mut self, transcript: &Path) -> Result<(), String> {
        if !self.followed.contains(transcript) {
            let directory = transcript
                .parent()
                .ok_or_else(|| format!("st cannot follow this seat's conversation: {} has no directory", transcript.display()))?
                .to_owned();
            let shared = self.followed.iter().any(|other| other.parent() == Some(directory.as_path()));
            if !shared {
                self.watcher()?
                    .watch(&directory, notify::RecursiveMode::NonRecursive)
                    .map_err(|error| format!("st cannot follow this seat's conversation: {error}"))?;
                self.shared
                    .directories
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(directory.clone());
                self.watched_changed();
            }
            self.shared
                .files
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend(change_files(transcript));
            self.followed.insert(transcript.to_owned());
        }
        self.read.insert(transcript.to_owned());
        Ok(())
    }

    /// The refresh presented the prompts: stop following what no card read.
    pub fn finish(&mut self) {
        self.remote.retain(|key, follower| {
            let read = self.remote_read.contains(key);
            if !read {
                follower.task.abort();
            }
            read
        });
        let gone = self.followed.difference(&self.read).cloned().collect::<Vec<_>>();
        for transcript in gone {
            self.followed.remove(&transcript);
            // Unfollowed, its changes are no longer seen: read it again if a card reads it later.
            self.calls.remove(&transcript);
            let mut files = self.shared.files.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let mut dirty = self.shared.dirty.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            for file in change_files(&transcript) {
                files.remove(&file);
                dirty.remove(&file);
            }
            drop(dirty);
            drop(files);
            let directory = transcript.parent();
            if !self.followed.iter().any(|other| other.parent() == directory)
                && let (Some(directory), Some(watcher)) = (directory, self.watcher.as_mut())
            {
                self.shared
                    .directories
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(directory);
                let _ = watcher.unwatch(directory);
                self.watched_changed();
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A follower reads as the person it was made for. When the seat's person changes, the card
    /// reads from a new follower as the new person, with nothing the old one read, and the old
    /// one stops once no card reads it.
    #[tokio::test]
    async fn a_remote_follower_reads_as_the_seats_person_now() {
        let root = tempfile::tempdir().unwrap();
        let secret = root.path().join("fleet-secret");
        std::fs::write(&secret, [7_u8; 32]).unwrap();
        std::fs::set_permissions(&secret, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        let relay = crate::peer::ClientRelay::from_config(&crate::config::Config {
            node: "prompt-gateway".into(),
            fleet_id: Some("fleet-test".into()),
            shared_secret_file: Some(secret),
            // Nothing listens: every read fails.
            peers: vec![crate::config::PeerConfig { name: "prompt-host".into(), url: "http://127.0.0.1:9".into() }],
            ..Default::default()
        })
        .unwrap()
        .unwrap();
        let conversation = |person: &str| RemoteConversation {
            relay: relay.clone(),
            host: "host/prompt-host".into(),
            session_id: "session-prompt".into(),
            person: person.into(),
        };
        let mut watch = PromptWatch::new(Arc::new(tokio::sync::Notify::new()));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        let first = loop {
            watch.begin();
            let read = watch.follow_remote(&conversation("person/one")).unwrap_err();
            watch.finish();
            if !read.contains("still reading") {
                break read;
            }
            assert!(tokio::time::Instant::now() < deadline, "the first follower never read");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        assert!(first.contains("on host/prompt-host"), "{first}");

        watch.begin();
        let read = watch.follow_remote(&conversation("person/two")).unwrap_err();
        watch.finish();
        assert!(read.contains("still reading"), "{read}");
        let persons = watch.remote.keys().map(|(_, _, person)| person.as_str()).collect::<Vec<_>>();
        assert_eq!(persons, ["person/two"]);
    }

    fn row(answers: Value, call: Value) -> Value {
        json!({"action_parameters": {"prompt.respond": {"answers": answers, "call_id": call}}})
    }

    fn entry(sequence: u64, kind: &str, body: Value) -> Value {
        json!({"sequence": sequence, "type": kind, "body": body})
    }

    #[test]
    fn a_prompt_names_the_call_st_names_and_allows_an_answer_to_it_whole() {
        // Newest first, as a slice reads them.
        let items = [
            entry(5, "tool_call", json!({"call_id": "call-c", "name": "Bash", "arguments": {"command": "make"}})),
            entry(4, "tool_result", json!({"call_id": "call-b"})),
            entry(3, "tool_call", json!({"call_id": "call-b", "name": "Read", "arguments": {"path": "a"}})),
            entry(2, "content", json!({"text": "Building."})),
        ];
        let calls = pending_calls(&items);
        assert_eq!(calls, [json!({"call_id": "call-c", "name": "Bash", "arguments": {"command": "make"}, "complete": true})]);

        let prompt = prompt_card(&row(json!(["allow", "deny"]), json!("call-c")), || Ok(calls.clone()));
        assert_eq!(
            prompt,
            json!({
                "calls": [{"call_id": "call-c", "tool": "Bash", "input": "{\"command\":\"make\"}", "input_complete": true}],
                "answers": ["allow", "deny"],
                "allow": true,
            })
        );
        // The decoded card is the client's type.
        serde_json::from_value::<fractal_client::AttentionPrompt>(prompt).unwrap();

        // Beside another call without a result (a parent's open Task call), the card names only
        // the call st names.
        let task = json!({"call_id": "call-task", "name": "Task", "arguments": {"prompt": "build"}});
        let both = vec![task, calls[0].clone()];
        let prompt = prompt_card(&row(json!(["allow", "deny"]), json!("call-c")), || Ok(both.clone()));
        assert_eq!(prompt["calls"].as_array().unwrap().len(), 1);
        assert_eq!(prompt["calls"][0]["call_id"], "call-c");
        assert_eq!(prompt["allow"], true);

        // A named call that has a result, or no answers: not answered here.
        let prompt = prompt_card(&row(json!(["allow", "deny"]), json!("call-b")), || Ok(both.clone()));
        assert_eq!((prompt["calls"].clone(), prompt["allow"].clone()), (json!([]), json!(false)));
        assert_eq!(prompt_card(&row(json!([]), json!("call-c")), || Ok(calls.clone()))["allow"], false);
    }

    #[test]
    fn a_prompt_that_names_no_call_shows_none_and_reads_no_conversation() {
        for call in [Value::Null, json!(7)] {
            let prompt = prompt_card(&row(json!(["allow", "deny"]), call), || unreachable!("read the conversation"));
            assert_eq!(prompt, json!({"calls": [], "answers": ["allow", "deny"], "allow": false}));
            serde_json::from_value::<fractal_client::AttentionPrompt>(prompt).unwrap();
        }
        let missing = json!({"action_parameters": {"prompt.respond": {"answers": ["allow", "deny"]}}});
        assert_eq!(prompt_card(&missing, || unreachable!("read the conversation"))["calls"], json!([]));
    }

    #[test]
    fn a_prompt_whose_call_input_was_cut_is_answered_in_the_terminal() {
        let long = "é".repeat(INPUT_BYTES);
        let prompt = prompt_card(&row(json!(["allow", "deny"]), json!("call-a")), || {
            Ok(vec![json!({"call_id": "call-a", "name": "Write", "arguments": {"text": long}})])
        });
        let input = prompt["calls"][0]["input"].as_str().unwrap();
        assert!(input.len() <= INPUT_BYTES && input.len() > INPUT_BYTES - 2, "{}", input.len());
        assert_eq!(prompt["calls"][0]["input_complete"], false);
        assert_eq!(prompt["allow"], false);
    }

    /// The timeline may hold only a bounded preview of a large record, or a truncation marker in
    /// place of its arguments: short enough to fit a card, but not what the call would run.
    #[test]
    fn a_call_the_timeline_holds_only_in_part_is_answered_in_the_terminal() {
        for marker in ["_preview_of_bytes", "_oversized_bytes"] {
            let mut call = entry(3, "tool_call", json!({"call_id": "call-a", "name": "Write",
                "arguments": "[st truncated this native timeline value: size limit; 40000000 bytes; fetch from owner]"}));
            call[marker] = json!(40_000_000);
            let calls = pending_calls(&[call]);
            assert_eq!(calls[0]["complete"], false, "{marker}");
            let prompt = prompt_card(&row(json!(["allow", "deny"]), json!("call-a")), || Ok(calls));
            assert_eq!(prompt["calls"][0]["input_complete"], false, "{marker}");
            assert_eq!(prompt["allow"], false, "{marker}");
        }
    }

    /// Wait for a followed conversation to change, or say it did not within `within`.
    pub(crate) async fn changed(watch: &PromptWatch, wake: &tokio::sync::Notify, within: Duration) -> bool {
        changed_by(|| watch.take_changed(), wake, within).await
    }

    /// `changed`, asking `take` whether a followed conversation changed.
    pub(crate) async fn changed_by(take: impl Fn() -> bool, wake: &tokio::sync::Notify, within: Duration) -> bool {
        // A wake left from an earlier event returns at once: wait on until the flag is set.
        let deadline = tokio::time::Instant::now() + within;
        loop {
            if take() {
                return true;
            }
            if tokio::time::timeout_at(deadline, wake.notified()).await.is_err() {
                return take();
            }
        }
    }

    fn append(path: &Path) {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
        writeln!(file, r#"{{"type":"response_item"}}"#).unwrap();
    }

    /// A change to a followed transcript wakes the refresher once. Another file in its directory
    /// does not, nor does a transcript no card read any more.
    #[tokio::test]
    async fn a_followed_transcript_that_changes_wakes_the_refresher() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let root = tempfile::tempdir().unwrap();
            let (oak, elm) = (root.path().join("oak.jsonl"), root.path().join("elm.jsonl"));
            std::fs::write(&oak, "{}\n").unwrap();
            let wake = Arc::new(tokio::sync::Notify::new());
            let mut watch = PromptWatch::new(Arc::clone(&wake));
            watch.begin();
            watch.follow(&oak).unwrap();
            watch.finish();
            // What is watched changed: read again, since FSEvents can lose an event around it.
            assert!(watch.take_changed());
            append(&oak);
            assert!(changed(&watch, &wake, Duration::from_secs(5)).await);
            assert!(!watch.take_changed(), "asked once");

            // Another seat's transcript beside it changes nothing here.
            append(&elm);
            // CI scratch-space accounting opens watched directories without changing them.
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
            assert!(!changed(&watch, &wake, Duration::from_millis(500)).await);

            // No card read it at the last refresh: it is no longer followed.
            watch.begin();
            watch.finish();
            tokio::time::sleep(Duration::from_millis(200)).await;
            watch.take_changed();
            append(&oak);
            assert!(!changed(&watch, &wake, Duration::from_millis(500)).await);
        })
        .await
        .expect("prompt transcript watch test did not finish within 30 seconds");
    }

    /// A SQLite source (OpenCode's) commits to its write-ahead log and leaves the database file
    /// as it was until a checkpoint: a commit there wakes the refresher too.
    #[tokio::test]
    async fn a_commit_to_a_followed_databases_write_ahead_log_wakes_the_refresher() {
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("opencode.db");
        let connection = rusqlite::Connection::open(&database).unwrap();
        connection.pragma_update(None, "journal_mode", "WAL").unwrap();
        connection.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        connection.execute_batch("CREATE TABLE part (id TEXT PRIMARY KEY, data TEXT)").unwrap();
        let wake = Arc::new(tokio::sync::Notify::new());
        let mut watch = PromptWatch::new(Arc::clone(&wake));
        watch.begin();
        watch.follow(&database).unwrap();
        watch.finish();
        watch.take_changed();
        connection
            .execute("INSERT INTO part VALUES ('call-a', '{\"type\":\"tool\"}')", [])
            .unwrap();
        assert!(wal_holds_a_commit(&database), "the commit is in the log");
        assert!(changed(&watch, &wake, Duration::from_secs(5)).await);
    }

    /// Whether `database`'s write-ahead log holds a commit not yet checkpointed.
    pub(crate) fn wal_holds_a_commit(database: &Path) -> bool {
        let mut log = OsString::from(database.as_os_str());
        log.push("-wal");
        std::fs::metadata(PathBuf::from(log)).is_ok_and(|log| log.len() > 0)
    }

    /// Listing a followed transcript's directory opens it, which inotify reports as an access to
    /// the directory itself: the watch holds, and the next refresh keeps the watcher.
    #[tokio::test]
    async fn reading_a_watched_directory_keeps_its_watch() {
        let root = tempfile::tempdir().unwrap();
        let oak = root.path().join("oak.jsonl");
        std::fs::write(&oak, "{}\n").unwrap();
        let wake = Arc::new(tokio::sync::Notify::new());
        let mut watch = PromptWatch::new(Arc::clone(&wake));
        watch.begin();
        watch.follow(&oak).unwrap();
        watch.finish();
        watch.take_changed();

        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        // The watcher delivers events in order: once the append after the listing wakes the
        // refresher, the listing's events have been delivered too.
        append(&oak);
        assert!(changed(&watch, &wake, Duration::from_secs(5)).await);
        watch.begin();
        assert_eq!(watch.rebuilds, 0, "listing the directory was taken for a lost watch");
    }

    /// A watched directory renamed away drops its watch with it; an overflow asks for a rescan;
    /// an event that names no path says nothing of what changed. After each the next refresh
    /// makes the watcher again and subscribes anew, and a transcript that is gone cannot be.
    #[tokio::test]
    async fn a_lost_watch_or_a_rescan_makes_the_watcher_again() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("oak");
        std::fs::create_dir_all(&directory).unwrap();
        let oak = directory.join("oak.jsonl");
        std::fs::write(&oak, "{}\n").unwrap();
        let wake = Arc::new(tokio::sync::Notify::new());
        let mut watch = PromptWatch::new(Arc::clone(&wake));
        let refresh = |watch: &mut PromptWatch| {
            watch.begin();
            let followed = watch.follow(&oak);
            watch.finish();
            followed
        };
        refresh(&mut watch).unwrap();
        watch.take_changed();
        assert_eq!(watch.rebuilds, 0);

        std::fs::rename(&directory, root.path().join("oak-moved")).unwrap();
        assert!(changed(&watch, &wake, Duration::from_secs(5)).await);
        let error = refresh(&mut watch).unwrap_err();
        assert!(error.starts_with("st cannot follow this seat's conversation"), "{error}");
        assert_eq!(watch.rebuilds, 1, "the renamed directory's watch was not trusted");

        std::fs::rename(root.path().join("oak-moved"), &directory).unwrap();
        refresh(&mut watch).unwrap();
        for event in [
            notify::Event::new(notify::EventKind::Other).set_flag(notify::event::Flag::Rescan),
            notify::Event::new(notify::EventKind::Modify(notify::event::ModifyKind::Any)),
        ] {
            tokio::time::sleep(Duration::from_millis(100)).await;
            watch.take_changed();
            let before = watch.rebuilds;
            watch.shared.deliver(Ok(event));
            assert!(watch.take_changed());
            refresh(&mut watch).unwrap();
            assert_eq!(watch.rebuilds, before + 1);
        }
    }

    /// After the watcher reports an error or lost events, no subscription it holds is trusted:
    /// the next refresh subscribes each card's conversation anew before reading it, and a card
    /// whose conversation can no longer be subscribed says so and offers no answer.
    #[tokio::test]
    async fn after_a_delivery_failure_each_card_subscribes_again_before_it_reads() {
        let root = tempfile::tempdir().unwrap();
        let (oak_directory, elm_directory) = (root.path().join("oak"), root.path().join("elm"));
        std::fs::create_dir_all(&oak_directory).unwrap();
        std::fs::create_dir_all(&elm_directory).unwrap();
        let (oak, elm) = (oak_directory.join("oak.jsonl"), elm_directory.join("elm.jsonl"));
        std::fs::write(&oak, "{}\n").unwrap();
        std::fs::write(&elm, "{}\n").unwrap();
        let wake = Arc::new(tokio::sync::Notify::new());
        let mut watch = PromptWatch::new(Arc::clone(&wake));
        watch.begin();
        watch.follow(&oak).unwrap();
        watch.follow(&elm).unwrap();
        watch.finish();
        let calls = || Ok(vec![json!({"call_id": "call-a", "name": "Bash", "arguments": {"command": "make"}, "complete": true})]);
        let card = |watch: &mut PromptWatch, transcript: &Path| {
            prompt_card(&row(json!(["allow", "deny"]), json!("call-a")), || {
                watch.follow(transcript)?;
                calls()
            })
        };
        assert_eq!(card(&mut watch, &oak)["allow"], true);

        // The watcher lost events: the refresher is woken to read again.
        watch.shared.deliver(Err(notify::Error::generic("inotify queue overflowed")));
        assert!(watch.take_changed());
        // Oak's directory went away meanwhile, so its card cannot subscribe again.
        std::fs::remove_dir_all(&oak_directory).unwrap();
        watch.begin();
        let prompt = card(&mut watch, &oak);
        assert_eq!(prompt["calls"], Value::Null);
        assert!(prompt["unknown_reason"].as_str().unwrap().starts_with("st cannot follow this seat's conversation"), "{prompt}");
        assert_eq!(prompt["allow"], false);
        // Elm's subscribes anew, and its changes wake the refresher through the new watcher.
        assert_eq!(card(&mut watch, &elm)["allow"], true);
        watch.finish();
        // Forget what the removal told the old watcher, then change elm.
        tokio::time::sleep(Duration::from_millis(200)).await;
        watch.take_changed();
        append(&elm);
        assert!(changed(&watch, &wake, Duration::from_secs(5)).await);
    }

    /// A Codex transcript here, in `directory`, with no calls yet.
    fn transcript_here(directory: &Path, name: &str) -> crate::external_sessions::ExternalSession {
        let transcript = directory.join(format!("{name}.jsonl"));
        let meta = json!({"type": "session_meta", "timestamp": "2026-09-24T12:00:00Z",
            "payload": {"id": format!("native-{name}"), "cwd": directory, "source": "test"}});
        std::fs::write(&transcript, format!("{meta}\n")).unwrap();
        crate::external_sessions::ExternalSession {
            id: format!("session-{name}"),
            revision: "revision".into(),
            driver: crate::external_sessions::ExternalDriver::Codex,
            native_id: format!("native-{name}"),
            transcript,
            codex_home: None,
            cwd: None,
            title: None,
            started_at_unix_ms: 0,
            updated_at_unix_ms: 0,
            process: None,
        }
    }

    /// The harness's calls in a Codex transcript, each `(call, result)`: it calls `call`, or
    /// gives its result. One write, so a watcher sees one change.
    fn codex_lines(transcript: &Path, lines: &[(&str, bool)]) {
        use std::io::Write as _;
        let text = lines
            .iter()
            .map(|&(call, result)| {
                let payload = if result {
                    json!({"type": "function_call_output", "call_id": call, "output": "done"})
                } else {
                    json!({"type": "function_call", "name": "shell", "arguments": "{\"command\":[\"make\"]}", "call_id": call})
                };
                format!("{}\n", json!({"type": "response_item", "timestamp": "2026-09-24T12:00:01Z", "payload": payload}))
            })
            .collect::<String>();
        let mut file = std::fs::OpenOptions::new().append(true).open(transcript).unwrap();
        file.write_all(text.as_bytes()).unwrap();
    }

    fn reads() -> usize {
        TAIL_READS.with(std::cell::Cell::get)
    }

    /// One refresh: each card follows its conversation and takes its calls, by call ID.
    fn refresh_cards(watch: &mut PromptWatch, sources: &[&crate::external_sessions::ExternalSession]) -> Vec<Vec<String>> {
        watch.begin();
        let calls = sources
            .iter()
            .map(|source| {
                watch
                    .follow_conversation(Follow::Transcript(source))
                    .unwrap()
                    .unwrap()
                    .iter()
                    .map(|call| call["call_id"].as_str().unwrap().to_owned())
                    .collect()
            })
            .collect();
        watch.finish();
        calls
    }

    /// A fold presents every prompt card. A transcript here is read once while nothing changes
    /// it: each later refresh takes the calls it read. A change to it, a lost event, or a refresh
    /// in which no card read it has it read again.
    #[tokio::test]
    async fn a_transcript_here_is_read_again_only_after_it_changes() {
        let root = tempfile::tempdir().unwrap();
        let (oak, elm) = (transcript_here(root.path(), "oak"), transcript_here(root.path(), "elm"));
        codex_lines(&oak.transcript, &[("call-a", false)]);
        codex_lines(&elm.transcript, &[("call-e", false)]);
        let wake = Arc::new(tokio::sync::Notify::new());
        let mut watch = PromptWatch::new(Arc::clone(&wake));
        // Only changes reread here, however slowly the test runs.
        watch.calls_max_age = Duration::from_secs(3600);
        let start = reads();
        assert_eq!(refresh_cards(&mut watch, &[&oak, &elm]), [["call-a"], ["call-e"]]);
        assert_eq!(reads() - start, 2);
        tokio::time::sleep(Duration::from_millis(200)).await;
        watch.take_changed();
        assert_eq!(refresh_cards(&mut watch, &[&oak, &elm]), [["call-a"], ["call-e"]]);
        assert_eq!(reads() - start, 2, "an unchanged transcript was read again");

        // Oak's harness gets its result and calls again: only oak is read again.
        codex_lines(&oak.transcript, &[("call-a", true), ("call-b", false)]);
        assert!(changed(&watch, &wake, Duration::from_secs(5)).await);
        assert_eq!(refresh_cards(&mut watch, &[&oak, &elm]), [["call-b"], ["call-e"]]);
        assert_eq!(reads() - start, 3);

        // No card reads elm, so it is no longer followed, and a change to it then is not seen:
        // when a card reads it again, it is read again.
        assert_eq!(refresh_cards(&mut watch, &[&oak]), [["call-b"]]);
        codex_lines(&elm.transcript, &[("call-e", true)]);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(refresh_cards(&mut watch, &[&oak, &elm]), [vec!["call-b"], vec![]]);
        assert_eq!(reads() - start, 4);

        // The watcher lost events: nothing it followed is trusted, so each is read again.
        watch.shared.deliver(Err(notify::Error::generic("inotify queue overflowed")));
        assert_eq!(refresh_cards(&mut watch, &[&oak, &elm]), [vec!["call-b"], vec![]]);
        assert_eq!(reads() - start, 6);
    }

    /// An event can be lost without the watcher saying so. A card whose transcript changed no
    /// file it saw still reads it again once its last read is older than the most a card shows
    /// a read, so a new prompt is never hidden for longer than that.
    #[tokio::test]
    async fn a_transcript_here_is_read_again_after_its_read_grows_old_without_an_event() {
        let root = tempfile::tempdir().unwrap();
        let oak = transcript_here(root.path(), "oak");
        codex_lines(&oak.transcript, &[("call-a", false)]);
        let mut watch = PromptWatch::new(Arc::new(tokio::sync::Notify::new()));
        watch.calls_max_age = Duration::from_secs(3600);
        let start = reads();
        assert_eq!(refresh_cards(&mut watch, &[&oak]), [["call-a"]]);
        assert_eq!(refresh_cards(&mut watch, &[&oak]), [["call-a"]]);
        assert_eq!(reads() - start, 1, "a fresh read was read again");
        // No file event: the read is past its age, so the next refresh reads again.
        watch.calls_max_age = Duration::ZERO;
        assert_eq!(refresh_cards(&mut watch, &[&oak]), [["call-a"]]);
        assert_eq!(reads() - start, 2, "an old read was served without reading again");
    }

    /// A transcript st cannot follow is not read as if it could: its card says why.
    #[test]
    fn a_transcript_st_cannot_follow_says_so() {
        let mut watch = PromptWatch::new(Arc::new(tokio::sync::Notify::new()));
        let error = watch.follow(Path::new("/no/such/directory/oak.jsonl")).unwrap_err();
        assert!(error.starts_with("st cannot follow this seat's conversation"), "{error}");
        let prompt = prompt_card(&row(json!(["allow", "deny"]), json!("call-a")), || Err(error.clone()));
        assert_eq!(prompt["calls"], Value::Null);
        assert_eq!(prompt["unknown_reason"], json!(error));
        assert_eq!(prompt["allow"], false);
    }

    #[test]
    fn a_prompt_st_cannot_read_says_why_and_allows_no_answer() {
        let prompt = prompt_card(&row(json!(["allow", "deny"]), json!("call-a")), || Err("the seat runs elsewhere".into()));
        assert_eq!(
            prompt,
            json!({"calls": null, "answers": ["allow", "deny"], "allow": false, "unknown_reason": "the seat runs elsewhere"})
        );
        serde_json::from_value::<fractal_client::AttentionPrompt>(prompt).unwrap();
    }
}
