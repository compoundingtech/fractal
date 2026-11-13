//! Thread mounts share the authenticated feed and native editor. The UI keeps disclosure and
//! drafts local. Accepted metadata, message history and unread state come from the member.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fractal_client::{
    Client, ClientError, Envelope, Fence, Snapshot, ThreadCollectionRows, ThreadReadParameters,
    ThreadRenameParameters, ThreadReplyParameters, ThreadStartParameters, ThreadState,
    ThreadSummary, ThreadTargetParameters, ThreadTimelineChange, ThreadTimelineEvent,
};
use fractal_feed::{ThreadAccess, ThreadInterest, model::Model};
use ratatui::text::{Line, Span};

use super::{
    composer,
    doc::{Doc, DocExt, Hit},
    edit,
    pane::Pane,
    text, theme, thread,
};

const DRAFT_COLLISION: &str = "This item has different saved drafts. Both drafts are kept. Combine saved drafts to edit both together; nothing will be sent.";
const DRAFT_LOCKED_COLLISION: &str = "Both saved drafts are kept. Settle the original thread delivery before combining them; its request cannot be rewritten.";
const DRAFT_COMBINED: &str = "Both saved drafts were combined for editing. Nothing has been sent.";

fn start_draft_text<'a>(
    texts: &'a HashMap<String, String>,
    key: &str,
    item: &str,
    incoming: Option<&'a str>,
) -> Result<&'a str, &'static str> {
    let canonical = texts
        .get(key)
        .map(String::as_str)
        .filter(|text| !text.is_empty());
    let legacy = texts
        .get(&format!("chat:{item}"))
        .map(String::as_str)
        .filter(|text| !text.is_empty());
    let selected = incoming.or(canonical).or(legacy).unwrap_or("");
    if canonical
        .into_iter()
        .chain(legacy)
        .any(|saved| saved != selected)
    {
        return Err(DRAFT_COLLISION);
    }
    Ok(selected)
}

#[derive(Clone)]
struct Origin {
    context: ThreadInterest,
    agent: String,
    title: String,
    snapshot: String,
}

#[derive(Default)]
struct Draft {
    quote: Option<String>,
    rename_observed: Option<Envelope<ThreadSummary>>,
    delivery: Option<Delivery>,
}

#[derive(Clone, PartialEq, Eq)]
enum Input {
    Start { item: String, episode: String },
    Reply(String),
    Rename(String),
}

impl Input {
    fn key(&self) -> String {
        match self {
            Self::Start { item, episode } => format!("thread-start:{item}:{episode}"),
            Self::Reply(id) => format!("thread-reply:{id}"),
            Self::Rename(id) => format!("thread-rename:{id}"),
        }
    }
}

#[derive(Clone)]
enum Mutation {
    Start(ThreadStartParameters),
    Reply(ThreadReplyParameters),
    Rename(ThreadRenameParameters),
    MarkRead(ThreadReadParameters),
    Close,
    Reopen,
}

#[derive(Clone)]
pub(super) struct Write {
    key: String,
    id: String,
    host: String,
    input: Option<String>,
    fence: Fence,
    mutation: Mutation,
    client: Option<Client>,
    agent: String,
}

pub(super) struct Receipt {
    write: Write,
    confirmation: Confirmation,
}

enum Confirmation {
    Applied(Option<Envelope<ThreadSummary>>),
    Rejected,
    Failed(ClientError),
    NotSent(ClientError),
}

#[derive(Clone)]
struct Delivery {
    write: Write,
    pending: bool,
    unknown: bool,
}

impl Delivery {
    fn new(write: Write) -> Self {
        Self {
            write,
            pending: true,
            unknown: false,
        }
    }

    fn failed(&mut self, error: &ClientError) -> bool {
        self.pending = false;
        self.unknown |= error.outcome_unknown()
            || matches!(error, ClientError::Protocol(_))
            || error.applied() != Some("none")
                && matches!(
                    error,
                    ClientError::Api(
                        fractal_client::ErrorCode::Internal
                            | fractal_client::ErrorCode::Unknown
                            | fractal_client::ErrorCode::RemoteUnavailable
                            | fractal_client::ErrorCode::ResponseBudgetExceeded,
                        ..
                    )
                );
        self.unknown
    }
}

impl Draft {
    fn locked(&self) -> bool {
        self.delivery
            .as_ref()
            .is_some_and(|delivery| delivery.pending || delivery.unknown)
    }
}

impl Write {
    pub(super) fn not_sent(self, error: ClientError) -> Receipt {
        Receipt {
            write: self,
            confirmation: Confirmation::NotSent(error),
        }
    }

    pub(super) fn offline(self) -> Receipt {
        Receipt {
            write: self,
            confirmation: Confirmation::NotSent(ClientError::Transport(
                "Reconnect to send this thread action".into(),
            )),
        }
    }
    pub(super) async fn perform(mut self, client: Client) -> Receipt {
        if let Err(error) = self.prepare(client) {
            return Receipt {
                write: self,
                confirmation: Confirmation::NotSent(error),
            };
        }
        let client = self.client.as_ref().unwrap().clone();
        let action = format!("action/{}", self.key);
        let key = self.key.as_str();
        let fence = self.fence.clone();
        let target = || ThreadTargetParameters {
            target_id: self.id.clone(),
        };
        let result = match &self.mutation {
            Mutation::Start(value) => {
                client
                    .thread_start(&action, key, fence, value.clone())
                    .await
            }
            Mutation::Reply(value) => {
                client
                    .thread_reply(&action, key, fence, value.clone())
                    .await
            }
            Mutation::Rename(value) => {
                client
                    .thread_rename(&action, key, fence, value.clone())
                    .await
            }
            Mutation::MarkRead(value) => {
                client.thread_read(&action, key, fence, value.clone()).await
            }
            Mutation::Close => client.thread_close(&action, key, fence, target()).await,
            Mutation::Reopen => client.thread_reopen(&action, key, fence, target()).await,
        };
        let confirmation = match result {
            Ok(result)
                if result.value.kind != "action-result"
                    || result.value.action_id != action
                    || result.value.operation_id.is_empty()
                    || result.value.snapshot_id.is_empty()
                    || result.value.affected_ids.iter().any(String::is_empty) =>
            {
                Confirmation::Failed(ClientError::Protocol(
                    "The thread action receipt did not match the original action".into(),
                ))
            }
            Ok(result) if result.value.status == fractal_client::ActionStatus::Rejected => {
                Confirmation::Rejected
            }
            Ok(result)
                if result.value.status == fractal_client::ActionStatus::Completed
                    && self.matches_receipt(&result.value) =>
            {
                Confirmation::Applied(client.thread_get(&self.id).await.ok())
            }
            Ok(_) => Confirmation::Failed(ClientError::Protocol(
                "Fractal has not confirmed the original thread action".into(),
            )),
            Err(error) => Confirmation::Failed(error),
        };
        Receipt {
            write: self,
            confirmation,
        }
    }

    fn prepare(&mut self, client: Client) -> Result<(), ClientError> {
        let client = self.client.as_ref().unwrap_or(&client);
        let prepared = match &mut self.mutation {
            Mutation::Start(parameters) => client.prepare_thread_start(&self.key, parameters),
            Mutation::Reply(parameters) => {
                client.prepare_thread_reply(&self.key, &self.agent, parameters)
            }
            _ => Ok(()),
        };
        prepared.map_err(|error| ClientError::Protocol(error.to_string()))?;
        self.client = Some(client.clone());
        Ok(())
    }

    fn matches_receipt(&self, receipt: &fractal_client::ActionResult) -> bool {
        match &self.mutation {
            Mutation::MarkRead(request) => receipt
                .affected_ids
                .iter()
                .all(|id| request.seen_message_ids.contains(id)),
            Mutation::Start(_) | Mutation::Reply(_) => {
                receipt.affected_ids.len() == 2
                    && receipt.affected_ids.contains(&self.id)
                    && receipt
                        .affected_ids
                        .iter()
                        .any(|id| id.starts_with("message/") && id.len() > "message/".len())
            }
            Mutation::Rename(_) | Mutation::Close | Mutation::Reopen => {
                receipt.affected_ids.contains(&self.id)
                    && receipt
                        .metadata_claim_id
                        .as_ref()
                        .is_some_and(|revision| !revision.is_empty())
            }
        }
    }

    fn label(&self) -> &'static str {
        match self.mutation {
            Mutation::Start(_) => "Send first message",
            Mutation::Reply(_) => "Send reply",
            Mutation::Rename(_) => "Rename thread",
            Mutation::MarkRead(_) => "Mark visible replies read",
            Mutation::Close => "Close thread",
            Mutation::Reopen => "Reopen thread",
        }
    }
}

#[derive(Default)]
struct History {
    head: Vec<ThreadTimelineEvent>,
    earlier: Vec<ThreadTimelineEvent>,
    has_more: bool,
    cursor: Option<String>,
    paged: bool,
    loading: bool,
}

impl History {
    fn rows(&self) -> impl Iterator<Item = &ThreadTimelineEvent> {
        let mut seen = BTreeSet::new();
        self.head
            .iter()
            .chain(&self.earlier)
            .filter(move |event| seen.insert(event.id.as_str()))
    }

    fn more(&self) -> bool {
        if self.paged {
            self.cursor.is_some()
        } else {
            self.has_more
        }
    }
}

pub(super) struct Earlier {
    pub id: String,
    cursor: Option<String>,
    tail: String,
}

impl Earlier {
    pub(super) async fn read(
        &self,
        client: &Client,
    ) -> Result<fractal_client::ThreadTimelinePage, String> {
        let cursor = if let Some(cursor) = &self.cursor {
            Some(cursor.clone())
        } else {
            let head = client
                .thread_timeline(&self.id, None, Some(50))
                .await
                .map_err(|error| error.human())?
                .value;
            if head.items.last().is_none_or(|row| row.id != self.tail) {
                return Err("The thread moved while loading history. Try again.".into());
            }
            head.next_cursor
        };
        match cursor {
            Some(cursor) => client
                .thread_timeline(&self.id, Some(&cursor), Some(50))
                .await
                .map(|result| result.value)
                .map_err(|error| error.human()),
            None => Ok(fractal_client::ThreadTimelinePage {
                items: Vec::new(),
                next_cursor: None,
            }),
        }
    }
}

#[derive(Default)]
pub(super) struct Threads {
    available: bool,
    access: ThreadAccess,
    origins: BTreeMap<String, Origin>,
    retained_origins: BTreeMap<String, Origin>,
    unread_items: BTreeSet<String>,
    sessions: BTreeMap<String, String>,
    lists: BTreeMap<ThreadInterest, Vec<String>>,
    summaries: BTreeMap<String, Envelope<ThreadSummary>>,
    histories: BTreeMap<String, History>,
    earlier: Vec<Earlier>,
    failures: BTreeMap<ThreadInterest, String>,
    open: BTreeMap<String, BTreeSet<String>>,
    selected: BTreeMap<String, String>,
    start_slot: Option<(ThreadInterest, String)>,
    folded_inbox_details: BTreeSet<String>,
    focused: Option<(String, Input)>,
    drafts: BTreeMap<String, Draft>,
    writes: Vec<Write>,
    pending_actions: BTreeMap<String, Delivery>,
    hover: Option<String>,
}

impl Threads {
    pub(super) fn enable(&mut self, available: bool) {
        self.available = available;
        if !available {
            self.access = ThreadAccess::default();
            self.lists.clear();
            self.unread_items.clear();
            self.start_slot = None;
            self.summaries.clear();
            self.histories.clear();
            self.earlier.clear();
            self.failures.clear();
            self.focused = None;
            self.writes.clear();
            for delivery in self.pending_actions.values_mut().chain(
                self.drafts
                    .values_mut()
                    .filter_map(|draft| draft.delivery.as_mut()),
            ) {
                if delivery.pending {
                    delivery.pending = false;
                    delivery.unknown = true;
                }
            }
        }
    }

    pub(super) fn set_access(&mut self, access: ThreadAccess) {
        self.access = if self.available {
            access
        } else {
            ThreadAccess::default()
        };
        let allowed = self
            .focused
            .as_ref()
            .is_none_or(|(_, input)| self.allowed(input));
        if !allowed {
            self.focused = None;
        }
    }

    fn allowed(&self, input: &Input) -> bool {
        match input {
            Input::Start { .. } => self.access.start,
            Input::Reply(_) => self.access.reply,
            Input::Rename(_) => self.access.rename,
        }
    }

    pub(super) fn available(&self) -> bool {
        self.available
    }

    pub(super) fn can_start(&self, id: &str) -> bool {
        self.available && self.access.start && self.origins.contains_key(id)
    }

    pub(super) fn editing_in(&self, pane: &Pane) -> bool {
        self.focused
            .as_ref()
            .is_some_and(|(host, _)| host == &pane.key())
    }

    pub(super) fn begin(
        &mut self,
        host: &str,
        item: &str,
        texts: &mut HashMap<String, String>,
    ) -> bool {
        if !self.available || !self.access.start {
            return false;
        }
        let Some(input) = self.start_input(item) else {
            return false;
        };
        let key = input.key();
        let selected = start_draft_text(texts, &key, item, None).and_then(|selected| {
            if self.drafts.get(&key).and_then(|draft| draft.delivery.as_ref()).is_some_and(|delivery| {
                matches!(&delivery.write.mutation, Mutation::Start(start) if start.content != selected)
            }) {
                return Err(DRAFT_COLLISION);
            }
            Ok(selected)
        });
        let selected = match selected {
            Ok(selected) => selected.to_owned(),
            Err(message) => {
                let message = if self.drafts.get(&key).is_some_and(Draft::locked) {
                    DRAFT_LOCKED_COLLISION
                } else {
                    message
                };
                if let Some(context) = self.start_origin(item).map(|origin| origin.context.clone())
                {
                    self.failures.insert(context, message.into());
                }
                return false;
            }
        };
        self.clear_draft_collision(item);
        texts.remove(&format!("chat:{item}"));
        texts.insert(key.clone(), selected);
        self.drafts.entry(key).or_default();
        self.focused = Some((host.into(), input));
        true
    }

    pub(super) fn start_with_text(
        &mut self,
        host: &str,
        item: &str,
        text: &str,
        texts: &mut HashMap<String, String>,
    ) -> Result<(), String> {
        if !self.available || !self.access.start {
            return Err("This connection cannot start a thread; your feedback is kept".into());
        }
        if text.trim().is_empty()
            || text.starts_with("doc/")
            || text.len() > fractal_client::device::INLINE_MAX_BYTES
        {
            return Err(
                "Use nonempty message text within the thread message limit; your feedback is kept"
                    .into(),
            );
        }
        let origin = self
            .origins
            .get(item)
            .ok_or("This Inbox item has no current thread recipient")?;
        let ThreadInterest::Inbox { source, episode } = &origin.context else {
            return Err("The Inbox origin is unavailable".into());
        };
        let input = Input::Start {
            item: item.into(),
            episode: episode.clone(),
        };
        let key = input.key();
        start_draft_text(texts, &key, item, Some(text))?;
        let empty = Draft::default();
        let draft = self.drafts.get(&key).unwrap_or(&empty);
        if let Some(delivery) = &draft.delivery {
            let Mutation::Start(start) = &delivery.write.mutation else {
                return Err("The original action has a different intent".into());
            };
            if start.agent_id != origin.agent
                || start.origin.source_id != *source
                || start.origin.episode != *episode
                || start.content != text
            {
                return Err(
                    "The original thread action has a different recipient, origin or text".into(),
                );
            }
        }
        let write = self.prepare_write(host, &input, text, draft)?;
        // Preparation has validated the current origin, recipient and immutable retry. From
        // here there are no refusal paths: caller feedback is released only after retention.
        texts.remove(&format!("chat:{item}"));
        texts.insert(key, text.into());
        self.focused = Some((host.into(), input.clone()));
        self.retain_and_queue(&input, write);
        self.clear_draft_collision(item);
        Ok(())
    }

    fn clear_draft_collision(&mut self, item: &str) {
        if let Some(context) = self.start_origin(item).map(|origin| origin.context.clone())
            && self.failures.get(&context).is_some_and(|message| {
                matches!(
                    message.as_str(),
                    DRAFT_COLLISION | DRAFT_LOCKED_COLLISION | DRAFT_COMBINED
                )
            })
        {
            self.failures.remove(&context);
        }
    }

    fn can_combine_saved_drafts(&self, item: &str, texts: &HashMap<String, String>) -> bool {
        let Some(input) = self.start_input(item) else {
            return false;
        };
        let key = input.key();
        self.available
            && self.access.start
            && !self
                .drafts
                .get(&key)
                .is_some_and(|draft| draft.delivery.is_some())
            && matches!((texts.get(&key), texts.get(&format!("chat:{item}"))), (Some(canonical), Some(legacy)) if !canonical.is_empty() && !legacy.is_empty() && canonical != legacy)
    }

    fn combine_saved_drafts(
        &mut self,
        host: &str,
        item: &str,
        texts: &mut HashMap<String, String>,
    ) {
        let Some(input) = self.start_input(item) else {
            return;
        };
        let key = input.key();
        if !self.can_combine_saved_drafts(item, texts) {
            if self.drafts.get(&key).is_some_and(Draft::locked)
                && let Some(context) = self.start_origin(item).map(|origin| origin.context.clone())
            {
                self.failures.insert(context, DRAFT_LOCKED_COLLISION.into());
            }
            return;
        }
        let combined = format!(
            "{}\n\n--- Earlier saved draft ---\n\n{}",
            texts[&key],
            texts[&format!("chat:{item}")]
        );
        // Retain both raw texts in the editable destination before deleting the duplicate
        // legacy slot. Combining is local recovery, never a request or an implicit send.
        texts.insert(key.clone(), combined);
        texts.remove(&format!("chat:{item}"));
        self.drafts.entry(key).or_default();
        self.focused = Some((host.into(), input));
        if let Some(context) = self.start_origin(item).map(|origin| origin.context.clone()) {
            self.failures.insert(context, DRAFT_COMBINED.into());
        }
    }

    fn start_origin(&self, item: &str) -> Option<&Origin> {
        let retained = self.retained_origins.get(item).filter(|origin| {
            let ThreadInterest::Inbox { episode, .. } = &origin.context else {
                return false;
            };
            self.drafts
                .get(
                    &Input::Start {
                        item: item.into(),
                        episode: episode.clone(),
                    }
                    .key(),
                )
                .and_then(|draft| draft.delivery.as_ref())
                .is_some_and(|delivery| delivery.pending || delivery.unknown)
        });
        retained
            .or_else(|| self.origins.get(item))
            .or_else(|| self.retained_origins.get(item))
    }

    fn start_input(&self, item: &str) -> Option<Input> {
        let ThreadInterest::Inbox { episode, .. } = &self.start_origin(item)?.context else {
            return None;
        };
        Some(Input::Start {
            item: item.into(),
            episode: episode.clone(),
        })
    }

    pub(super) fn shortcut(&self, pane: &Pane, key: KeyEvent) -> Option<thread::Tap> {
        if !self.available
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            || self
                .focused
                .as_ref()
                .is_some_and(|(host, _)| host == &pane.key())
        {
            return None;
        }
        let id = match pane {
            Pane::Thread(id) => id,
            Pane::Home(Some(id))
                if id.starts_with("thread/")
                    && !self.folded_inbox_details.contains(&pane.key()) =>
            {
                id
            }
            _ => {
                let id = self.selected.get(&pane.key())?;
                if !self
                    .open
                    .get(&pane.key())
                    .is_some_and(|expanded| expanded.contains(id))
                {
                    return None;
                }
                id
            }
        };
        let summary = self.summary(id)?;
        if key.code == KeyCode::Char('R') || key.code == KeyCode::Char('r') {
            let original = self
                .pending_actions
                .get(id)
                .into_iter()
                .chain(
                    self.drafts
                        .values()
                        .filter_map(|draft| draft.delivery.as_ref()),
                )
                .find(|delivery| {
                    delivery.write.id == *id && delivery.unknown && !delivery.pending
                })?;
            return Some(thread::Tap::Retry(original.write.key.clone()));
        }
        Some(match key.code {
            KeyCode::Char('Z' | 'z') if summary.state == ThreadState::Open && self.access.close => {
                thread::Tap::Close(id.clone())
            }
            KeyCode::Char('Z' | 'z')
                if summary.state == ThreadState::Closed && self.access.reopen =>
            {
                thread::Tap::Reopen(id.clone())
            }
            KeyCode::Char('N' | 'n') if self.access.rename => thread::Tap::Rename(id.clone()),
            KeyCode::Char('V' | 'v') if self.can_mark_read(id) => {
                thread::Tap::MarkVisibleRead(id.clone())
            }
            KeyCode::Char('P' | 'p') if !matches!(pane, Pane::Thread(_)) => {
                thread::Tap::OpenTab(id.clone())
            }
            _ => return None,
        })
    }

    pub(super) fn model(&mut self, model: &Model) {
        if !self.available {
            return;
        }
        self.sessions.clear();
        self.sessions.extend(
            model
                .sessions
                .items
                .iter()
                .filter_map(|row| {
                    let fractal_client::Resource::Session(session) = row else {
                        return None;
                    };
                    Some((session.header.id.clone(), session.owner_id.clone()))
                })
                .chain(
                    model
                        .opened_sessions
                        .iter()
                        .map(|(id, session)| (id.clone(), session.owner_id.clone())),
                ),
        );
        self.origins.clear();
        self.unread_items.clear();
        let Some(snapshot) = &model.now.snapshot else {
            return;
        };
        for item in model.attention() {
            if item.thread_unread == Some(true) {
                self.unread_items.insert(item.header.id.clone());
            }
            if item.episode.is_empty() || item.source_id.is_empty() {
                continue;
            }
            let Some(agent) = item
                .conversation_id
                .as_ref()
                .filter(|id| id.starts_with("agent/"))
            else {
                continue;
            };
            self.origins.insert(
                item.header.id.clone(),
                Origin {
                    context: ThreadInterest::Inbox {
                        source: item.source_id.clone(),
                        episode: item.episode.clone(),
                    },
                    agent: agent.clone(),
                    title: item.title.clone(),
                    snapshot: snapshot.id.clone(),
                },
            );
        }
    }

    fn context(&self, pane: &Pane) -> Option<ThreadInterest> {
        match pane {
            Pane::Home(Some(id)) => self.start_origin(id).map(|origin| origin.context.clone()),
            Pane::Agent(Some(id)) if id.starts_with("agent/") => {
                Some(ThreadInterest::Agent(id.clone()))
            }
            Pane::Agent(Some(id)) => self
                .sessions
                .get(id)
                .filter(|agent| agent.starts_with("agent/"))
                .cloned()
                .map(ThreadInterest::Agent),
            Pane::Mission(Some(id)) => Some(ThreadInterest::Mission(id.clone())),
            _ => None,
        }
    }

    pub(super) fn interests(&self, pane: &Pane) -> Vec<ThreadInterest> {
        if !self.available {
            return Vec::new();
        }
        let mut out: Vec<_> = self.context(pane).into_iter().collect();
        if matches!(pane, Pane::Home(_)) {
            out.push(ThreadInterest::RecentlyClosed);
        }
        if let Pane::Thread(id) = pane {
            out.push(ThreadInterest::Summary(id.clone()));
            out.push(ThreadInterest::Timeline(id.clone()));
        } else if let Pane::Home(Some(id)) = pane
            && id.starts_with("thread/")
        {
            out.push(ThreadInterest::Summary(id.clone()));
            if !self.folded_inbox_details.contains(&pane.key()) {
                out.push(ThreadInterest::Timeline(id.clone()));
            }
        } else if let Some(expanded) = self.open.get(&pane.key()) {
            for id in expanded {
                out.push(ThreadInterest::Summary(id.clone()));
                out.push(ThreadInterest::Timeline(id.clone()));
            }
        }
        out
    }

    pub(super) fn keep_windows(&mut self, wanted: &[ThreadInterest]) {
        self.lists.retain(|interest, _| wanted.contains(interest));
        if self
            .start_slot
            .as_ref()
            .is_some_and(|(interest, _)| !wanted.contains(interest))
        {
            self.start_slot = None;
        }
        self.failures
            .retain(|interest, _| wanted.contains(interest));
        let held = |id: &str| {
            wanted.iter().any(
                |interest| matches!(interest, ThreadInterest::Timeline(thread) if thread == id),
            )
        };
        self.histories.retain(|id, _| held(id));
        self.summaries.retain(|id, _| {
            held(id)
                || self.lists.values().any(|ids| ids.contains(id))
                || self
                    .start_slot
                    .as_ref()
                    .is_some_and(|(_, started)| started == id)
        });
    }

    pub(super) fn summary(&self, id: &str) -> Option<&ThreadSummary> {
        self.summaries.get(id).map(|row| &row.value)
    }

    fn put_summary(&mut self, summary: Envelope<ThreadSummary>) {
        let id = &summary.value.id;
        if !self.summaries.get(id).is_some_and(|old| {
            old.snapshot.host_id == summary.snapshot.host_id
                && old.snapshot.store_index > summary.snapshot.store_index
        }) {
            self.summaries.insert(id.clone(), summary);
        }
    }

    pub(super) fn window(
        &mut self,
        interest: ThreadInterest,
        snapshot: Snapshot,
        replace: bool,
        rows: ThreadCollectionRows,
        removes: Vec<String>,
        order: Vec<String>,
        has_more: bool,
    ) {
        self.failures.remove(&interest);
        match rows {
            ThreadCollectionRows::Summaries(rows) => {
                if replace || self.lists.contains_key(&interest) {
                    self.lists.insert(interest, order);
                    for value in rows {
                        self.put_summary(Envelope {
                            snapshot: snapshot.clone(),
                            value,
                            request_id: String::new(),
                            api_version: fractal_client::API_VERSION.into(),
                        });
                    }
                }
            }
            ThreadCollectionRows::Timeline(rows) => {
                let ThreadInterest::Timeline(id) = interest else {
                    return;
                };
                if !replace && !self.histories.contains_key(&id) {
                    return;
                }
                let history = self.histories.entry(id).or_default();
                let mut by_id: BTreeMap<_, _> = if replace {
                    BTreeMap::new()
                } else {
                    history
                        .head
                        .iter()
                        .cloned()
                        .map(|row| (row.id.clone(), row))
                        .collect()
                };
                for id in removes {
                    by_id.remove(&id);
                }
                by_id.extend(rows.into_iter().map(|row| (row.id.clone(), row)));
                let next: Vec<_> = order
                    .into_iter()
                    .filter_map(|id| by_id.remove(&id))
                    .collect();
                if history.paged {
                    let overlap = history
                        .head
                        .iter()
                        .rposition(|old| next.iter().any(|new| old.id == new.id));
                    if let Some(index) = overlap {
                        // A bounded live head evicts its oldest rows as new ones arrive.
                        // Keep those rows between the new head and the pinned older page.
                        history.earlier.splice(..0, history.head.drain(index + 1..));
                    } else {
                        history.earlier.clear();
                        history.cursor = None;
                        history.paged = false;
                    }
                }
                history.head = next;
                history.has_more = has_more;
            }
        }
    }

    pub(super) fn failure(&mut self, interest: ThreadInterest, message: String, permanent: bool) {
        if permanent {
            match &interest {
                ThreadInterest::Timeline(id) => {
                    self.histories.remove(id);
                    self.summaries.remove(id);
                }
                ThreadInterest::Summary(id) => {
                    self.summaries.remove(id);
                }
                _ => {}
            }
            self.lists.remove(&interest);
            if self
                .start_slot
                .as_ref()
                .is_some_and(|(ask, _)| ask == &interest)
            {
                self.start_slot = None;
            }
        }
        self.failures.insert(interest, message);
    }

    pub(super) fn mark_inbox_unread(&self, list: &mut super::screens::Listing) {
        for item in &mut list.items {
            if let super::screens::Item::Row { index, second, .. } = item
                && list
                    .ids
                    .get(*index)
                    .is_some_and(|id| self.unread_items.contains(id))
            {
                second.push(Span::styled(
                    " · unread thread replies",
                    theme::fg(theme::SAPPHIRE),
                ));
            }
        }
    }

    pub(super) fn recently_closed(&self, list: &mut super::screens::Listing) {
        let Some(ids) = self
            .lists
            .get(&ThreadInterest::RecentlyClosed)
            .filter(|ids| !ids.is_empty())
        else {
            return;
        };
        list.items.push(super::screens::Item::Header {
            title: "Recently closed threads".into(),
            count: ids.len(),
            color: theme::OVERLAY1,
        });
        for id in ids {
            let Some(row) = self.summary(id) else {
                continue;
            };
            let index = list.ids.len();
            list.ids.push(id.clone());
            let title = text::sanitize(&row.title).replace('\n', " ");
            list.items.push(super::screens::Item::Row {
                index,
                first: vec![Span::styled(title, theme::soft())],
                right: row
                    .message_count
                    .map(|count| Span::styled(format!("{count}"), theme::dim()))
                    .into_iter()
                    .chain(
                        (row.unread == Some(true)).then(|| Span::styled(" · unread", theme::dim())),
                    )
                    .collect(),
                second: Vec::new(),
                children: Vec::new(),
            });
        }
        if matches!(list.state, super::screens::ListState::Empty(_)) {
            list.state = super::screens::ListState::Ready;
        }
    }

    pub(super) fn mount(
        &self,
        pane: &Pane,
        width: usize,
        cursor: &edit::Cursor,
        world: &super::view::World,
        texts: &HashMap<String, String>,
    ) -> Doc {
        let mut doc = Doc::new();
        if !self.available {
            if matches!(pane, Pane::Thread(_)) {
                doc.line(Line::from(Span::styled(
                    "Threads are unavailable on this member.",
                    theme::dim(),
                )));
            }
            return doc;
        }
        let host = pane.key();
        if let Pane::Thread(id) = pane {
            self.panel(&mut doc, &host, id, true, true, width, cursor, world, texts);
            return doc;
        }
        if let Pane::Home(Some(id)) = pane
            && id.starts_with("thread/")
        {
            self.panel(
                &mut doc, &host, id, true, false, width, cursor, world, texts,
            );
            return doc;
        }
        if let Some(context) = self.context(pane) {
            let ids = self.lists.get(&context);
            let start = match pane {
                Pane::Home(Some(item)) if ids.is_some() => self.start_input(item),
                _ => self.retained_origins.iter().find_map(|(item, origin)| {
                    if context != ThreadInterest::Agent(origin.agent.clone()) {
                        return None;
                    }
                    let input = self.start_input(item)?;
                    self.drafts
                        .get(&input.key())
                        .and_then(|draft| draft.delivery.as_ref())
                        .filter(|delivery| delivery.unknown || delivery.pending)
                        .map(|_| input)
                }),
            };
            let drafting = start
                .as_ref()
                .filter(|input| texts.contains_key(&input.key()));
            // Keep the new thread at the draft's position. The authoritative collection
            // still orders the other threads and may arrive after the start receipt.
            let pinned = self
                .start_slot
                .as_ref()
                .filter(|(interest, _)| ids.is_some() && interest == &context && drafting.is_none())
                .map(|(_, id)| id);
            if ids.is_some_and(|ids| !ids.is_empty()) || drafting.is_some() || pinned.is_some() {
                doc.blank();
                doc.section("Threads", None, width);
            }
            if let Some(input) = drafting {
                if let Input::Start { item, .. } = input
                    && let Some(origin) = self.start_origin(item)
                {
                    doc.line(thread::heading(
                        &origin.title,
                        &format!(" · Draft · to {}", origin.agent),
                        true,
                        width,
                    ));
                }
                self.editor(
                    &mut doc,
                    &host,
                    input,
                    "First message in thread",
                    cursor,
                    width,
                    texts,
                );
            }
            let submitted = drafting
                .and_then(|input| self.drafts.get(&input.key()))
                .and_then(|draft| draft.delivery.as_ref())
                .map(|delivery| &delivery.write.id);
            let siblings = ids
                .into_iter()
                .flatten()
                .filter(|id| Some(*id) != pinned && Some(*id) != submitted);
            for id in pinned.into_iter().chain(siblings) {
                self.panel(
                    &mut doc,
                    &host,
                    id,
                    self.open
                        .get(&host)
                        .is_some_and(|expanded| expanded.contains(id)),
                    false,
                    width,
                    cursor,
                    world,
                    texts,
                );
            }
            if let Some(message) = self.failures.get(&context) {
                doc.wrap(&[text::run(message, theme::dim())], width);
                if message == DRAFT_COLLISION
                    && let Pane::Home(Some(item)) = pane
                    && self.can_combine_saved_drafts(item, texts)
                {
                    doc.buttons(&[(
                        "",
                        "Combine saved drafts",
                        Hit::Thread(host.clone(), thread::Tap::CombineSavedDrafts(item.clone())),
                        theme::SAPPHIRE,
                    )]);
                }
            }
            if let Some(Input::Start { item, .. }) = start.as_ref()
                && drafting.is_none()
            {
                doc.buttons(&[(
                    "T",
                    "Start thread",
                    Hit::Thread(host, thread::Tap::Start(item.clone())),
                    theme::SAPPHIRE,
                )]);
            }
        }
        doc
    }

    #[allow(clippy::too_many_arguments)]
    fn panel(
        &self,
        doc: &mut Doc,
        host: &str,
        id: &str,
        open: bool,
        standalone: bool,
        width: usize,
        cursor: &edit::Cursor,
        world: &super::view::World,
        texts: &HashMap<String, String>,
    ) {
        let timeline = ThreadInterest::Timeline(id.into());
        let summary = self.summary(id);
        let panel_open = open && self.histories.contains_key(id);
        let reply_editor_visible = panel_open && self.access.reply;
        let rename_editor_visible =
            self.focused.as_ref() == Some(&(host.into(), Input::Rename(id.into())));
        if open {
            let missing = summary.is_none() || !self.histories.contains_key(id);
            if let Some(reason) = self.failures.get(&timeline) {
                // A paused live window keeps its cached body/draft, but must not pretend
                // those rows are current or remain an unexplained loading placeholder.
                doc.wrap(&[text::run(reason, theme::dim())], width);
            } else if missing {
                doc.wrap(&[text::run("Loading thread…", theme::dim())], width);
            }
            if missing && (standalone || summary.is_none()) {
                self.panel_feedback(doc, host, id, false, false, width);
                return;
            }
        }
        let Some(summary) = summary else {
            return;
        };
        let source = self.histories.get(id);
        let history: Vec<_> = source
            .into_iter()
            .flat_map(History::rows)
            .cloned()
            .collect();
        let input = Input::Reply(id.into());
        let key = input.key();
        let draft = self.drafts.get(&key);
        let draft_text = texts.get(&key).map_or("", String::as_str);
        let focus = self
            .focused
            .as_ref()
            .is_some_and(|(where_, mode)| where_ == host && mode == &input);
        let quote = draft.and_then(|draft| draft.quote.as_deref());
        let reply_to = quote
            .and_then(|id| {
                history.iter().find_map(|event| match &event.change {
                    ThreadTimelineChange::Message { content, .. } if event.id == id => {
                        Some(content.as_str())
                    }
                    _ => None,
                })
            })
            .or(quote);
        let rendered = thread::client_panel(
            summary,
            &history,
            &thread::State {
                open: panel_open,
                standalone,
                draft: draft_text,
                cursor: focus.then(|| cursor.at(&key, draft_text)),
                reply_to,
                hovered_message: self.hover.as_deref(),
                has_earlier: source.is_some_and(History::more),
                loading_earlier: source.is_some_and(|history| history.loading),
                can_mark_read: self.can_mark_read(id),
                can_reply: self.access.reply,
                pending_reply: draft
                    .and_then(|draft| draft.delivery.as_ref())
                    .is_some_and(|delivery| delivery.pending),
                hotkeys: self.hotkeys_for(host, id),
                can_rename: self.access.rename,
                can_close: self.access.close,
                can_reopen: self.access.reopen,
            },
            &world.person,
            |actor| {
                world
                    .agents
                    .items()
                    .iter()
                    .find(|agent| agent.id == actor)
                    .map(|agent| agent.name.clone())
            },
            &chrono::Utc::now().to_rfc3339(),
            width,
        );
        if let Some(rendered) = rendered {
            let base = doc.lines.len();
            for (message, line) in &rendered.entries {
                doc.targets.push(super::doc::Target {
                    line: base + *line,
                    column: 0,
                    width: width as u16,
                    hit: Hit::Thread(host.into(), thread::Tap::MessageHeader(message.clone())),
                });
            }
            doc.append(shell_doc(rendered, host, id), 0);
            if open {
                self.panel_feedback(
                    doc,
                    host,
                    id,
                    reply_editor_visible,
                    rename_editor_visible,
                    width,
                );
            }
        }
        if rename_editor_visible {
            self.editor(
                doc,
                host,
                &Input::Rename(id.into()),
                "Thread title",
                cursor,
                width,
                texts,
            );
        }
    }

    fn panel_feedback(
        &self,
        doc: &mut Doc,
        host: &str,
        id: &str,
        reply_editor_visible: bool,
        rename_editor_visible: bool,
        width: usize,
    ) {
        if let Some(delivery) = self.pending_actions.get(id) {
            self.feedback(doc, host, delivery, width);
        }
        for (input, draft) in &self.drafts {
            if let Some(delivery) = &draft.delivery
                && delivery.write.id == id
                && !(input == &Input::Reply(id.into()).key()
                    && delivery.pending
                    && reply_editor_visible)
                && !(input == &Input::Rename(id.into()).key() && rename_editor_visible)
            {
                self.feedback(doc, host, delivery, width);
            }
        }
    }

    fn editor(
        &self,
        doc: &mut Doc,
        host: &str,
        mode: &Input,
        hint: &str,
        cursor: &edit::Cursor,
        width: usize,
        texts: &HashMap<String, String>,
    ) {
        let key = mode.key();
        let draft = texts.get(&key).map_or("", String::as_str);
        if !self.allowed(mode) {
            doc.wrap(
                &[text::run(
                    "Draft kept; this connection cannot send it",
                    theme::dim(),
                )],
                width,
            );
            doc.lines(text::markdown(draft, width, theme::dim()));
            if let Some(delivery) = self
                .drafts
                .get(&key)
                .and_then(|draft| draft.delivery.as_ref())
            {
                self.feedback(doc, host, delivery, width);
            }
            return;
        }
        let focused = self
            .focused
            .as_ref()
            .is_some_and(|(place, input)| place == host && input == mode);
        doc.line(composer::divider(width, focused));
        doc.lines(composer::text(
            width,
            focused,
            draft,
            focused.then(|| cursor.at(&key, draft)),
            hint.into(),
        ));
        let (tap, label) = match mode {
            Input::Start { item, .. } => (thread::Tap::Start(item.clone()), "Send first message"),
            Input::Rename(id) => (thread::Tap::Rename(id.clone()), "Save title"),
            Input::Reply(_) => return,
        };
        let delivery = self
            .drafts
            .get(&key)
            .and_then(|draft| draft.delivery.as_ref());
        if let Some(delivery) = delivery.filter(|delivery| delivery.pending) {
            // Keep the action's status in its existing row, without leaving a second
            // active-looking Send control or pushing neighboring thread panels down.
            doc.line(composer::pending(delivery.write.label()));
        } else {
            doc.buttons(&[("enter", label, Hit::Thread(host.into(), tap), theme::ACCENT)]);
        }
        doc.blank();
        if let Some(delivery) = delivery.filter(|delivery| !delivery.pending) {
            self.feedback(doc, host, delivery, width);
        }
    }

    pub(super) fn hover(&mut self, message: Option<&str>) -> bool {
        if self.hover.as_deref() == message {
            return false;
        }
        self.hover = message.map(str::to_owned);
        true
    }

    pub(super) fn tap(
        &mut self,
        host: &str,
        tap: thread::Tap,
        texts: &mut HashMap<String, String>,
    ) -> Option<String> {
        if !self.available {
            return None;
        }
        let selected = match &tap {
            thread::Tap::Compose(id)
            | thread::Tap::Rename(id)
            | thread::Tap::Send(id)
            | thread::Tap::Close(id)
            | thread::Tap::Reopen(id)
            | thread::Tap::OpenTab(id)
            | thread::Tap::Earlier(id)
            | thread::Tap::LeaveComposer(id)
            | thread::Tap::MarkVisibleRead(id) => Some(id.clone()),
            thread::Tap::Reply { thread, .. } => Some(thread.clone()),
            thread::Tap::Retry(key) => self
                .pending_actions
                .values()
                .chain(
                    self.drafts
                        .values()
                        .filter_map(|draft| draft.delivery.as_ref()),
                )
                .find(|delivery| delivery.write.key == *key)
                .map(|delivery| delivery.write.id.clone()),
            _ => None,
        };
        if let Some(id) = selected {
            self.selected.insert(host.into(), id);
        }
        match tap {
            thread::Tap::Toggle(id) => {
                if host.starts_with("home:thread/") {
                    if !self.folded_inbox_details.remove(host) {
                        self.folded_inbox_details.insert(host.into());
                        self.blur_thread(host, &id);
                        if self.selected.get(host) == Some(&id) {
                            self.selected.remove(host);
                        }
                    } else if self.access.reply && self.summary(&id).is_some() {
                        self.focused = Some((host.into(), Input::Reply(id)));
                    }
                    return None;
                }
                if self
                    .open
                    .get(host)
                    .is_some_and(|expanded| expanded.contains(&id))
                {
                    let expanded = self.open.get_mut(host).unwrap();
                    expanded.remove(&id);
                    if expanded.is_empty() {
                        self.open.remove(host);
                    }
                    self.blur_thread(host, &id);
                    if self.selected.get(host) == Some(&id) {
                        self.selected.remove(host);
                    }
                } else {
                    self.open.entry(host.into()).or_default().insert(id.clone());
                    self.selected.insert(host.into(), id.clone());
                    if self.access.reply && self.summary(&id).is_some() {
                        self.focused = Some((host.into(), Input::Reply(id)));
                    }
                }
            }
            thread::Tap::Compose(id) if self.access.reply => {
                self.focused = Some((host.into(), Input::Reply(id)))
            }
            thread::Tap::Compose(_) => {}
            thread::Tap::Reply { thread, message } => {
                if !self.access.reply {
                    return None;
                }
                let input = Input::Reply(thread);
                let draft = self.drafts.entry(input.key()).or_default();
                if draft.locked() {
                    return None;
                }
                draft.quote = Some(message);
                draft.delivery = None;
                self.focused = Some((host.into(), input));
            }
            thread::Tap::Start(item) => {
                if let Some(input) = self.start_input(&item) {
                    if self.focused.as_ref() == Some(&(host.into(), input.clone())) {
                        self.submit(host, input, texts);
                    } else {
                        self.begin(host, &item, texts);
                    }
                }
            }
            thread::Tap::CombineSavedDrafts(item) => self.combine_saved_drafts(host, &item, texts),
            thread::Tap::Rename(id) => {
                if !self.access.rename {
                    return None;
                }
                self.activate(host, Input::Rename(id), texts);
            }
            thread::Tap::Send(id) => self.submit(host, Input::Reply(id), texts),
            thread::Tap::Retry(key) => self.retry(&key),
            thread::Tap::LeaveComposer(_) => self.focused = None,
            thread::Tap::Close(id) => self.lifecycle(host, id, Mutation::Close),
            thread::Tap::Reopen(id) => self.lifecycle(host, id, Mutation::Reopen),
            thread::Tap::OpenTab(id) => return Some(id),
            thread::Tap::Earlier(id) => {
                if let Some(history) = self.histories.get_mut(&id)
                    && history.more()
                    && !history.loading
                    && let Some(tail) = history.head.last()
                {
                    history.loading = true;
                    self.earlier.push(Earlier {
                        id,
                        cursor: history.cursor.clone(),
                        tail: tail.id.clone(),
                    });
                }
            }
            thread::Tap::MessageHeader(_) | thread::Tap::MarkVisibleRead(_) => {}
        }
        None
    }

    fn blur_thread(&mut self, host: &str, id: &str) {
        if self.focused.as_ref().is_some_and(|(place, mode)| {
            place == host
                && matches!(mode, Input::Reply(target) | Input::Rename(target) if target == id)
        }) {
            self.focused = None;
        }
    }

    fn hotkeys_for(&self, host: &str, id: &str) -> bool {
        !self
            .focused
            .as_ref()
            .is_some_and(|(place, _)| place == host)
            && (host == format!("thread:{id}")
                || host == format!("home:{id}") && !self.folded_inbox_details.contains(host)
                || self
                    .selected
                    .get(host)
                    .is_some_and(|selected| selected == id))
    }

    fn retry(&mut self, key: &str) {
        let delivery = self
            .pending_actions
            .values_mut()
            .chain(
                self.drafts
                    .values_mut()
                    .filter_map(|draft| draft.delivery.as_mut()),
            )
            .find(|delivery| delivery.write.key == key);
        if let Some(delivery) = delivery
            && !delivery.pending
            && delivery.unknown
        {
            delivery.pending = true;
            self.writes.push(delivery.write.clone());
        }
    }

    fn feedback(&self, doc: &mut Doc, host: &str, delivery: &Delivery, width: usize) {
        if delivery.pending {
            doc.wrap(
                &[text::run(
                    format!("{}…", delivery.write.label()),
                    theme::dim(),
                )],
                width,
            );
        } else if delivery.unknown {
            doc.wrap(
                &[text::run(
                    format!(
                        "{}: outcome not confirmed. Retry the original action.",
                        delivery.write.label()
                    ),
                    theme::dim(),
                )],
                width,
            );
            doc.buttons(&[(
                if self.hotkeys_for(host, &delivery.write.id) {
                    "R"
                } else {
                    ""
                },
                &format!("Retry {}", delivery.write.label()),
                Hit::Thread(host.into(), thread::Tap::Retry(delivery.write.key.clone())),
                theme::ACCENT,
            )]);
        }
    }

    fn activate(&mut self, host: &str, input: Input, texts: &mut HashMap<String, String>) {
        if let Input::Rename(id) = &input
            && self.pending_actions.contains_key(id)
        {
            return;
        }
        if self.focused.as_ref() == Some(&(host.into(), input.clone())) {
            self.submit(host, input, texts);
        } else {
            let key = input.key();
            if let Input::Rename(id) = &input
                && !self.drafts.get(&key).is_some_and(Draft::locked)
            {
                let observed = self.summaries.get(id).cloned();
                texts.entry(key.clone()).or_insert_with(|| {
                    observed
                        .as_ref()
                        .map(|row| row.value.title.clone())
                        .unwrap_or_default()
                });
                self.drafts.entry(key).or_default().rename_observed = observed;
            } else {
                texts.entry(key.clone()).or_default();
                self.drafts.entry(key).or_default();
            }
            self.focused = Some((host.into(), input));
        }
    }

    pub(super) fn key(
        &mut self,
        pane: &Pane,
        key: KeyEvent,
        cursor: &edit::Cursor,
        texts: &mut HashMap<String, String>,
    ) -> bool {
        if !self.available {
            return false;
        }
        let host = pane.key();
        if self
            .focused
            .as_ref()
            .is_some_and(|(_, mode)| !self.allowed(mode))
        {
            self.focused = None;
            return false;
        }
        if self
            .focused
            .as_ref()
            .is_some_and(|(place, _)| place != &host)
        {
            self.focused = None;
        }
        if self.focused.is_none() {
            if key.code == KeyCode::Char('t')
                && key.modifiers.is_empty()
                && let Pane::Home(Some(item)) = pane
                && self.start_input(item).is_some()
            {
                self.begin(&host, item, texts);
                return true;
            }
            if let Pane::Thread(id) = pane
                && self.access.reply
                && matches!(key.code, KeyCode::Char(_))
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && self.summary(id).is_some()
            {
                self.focused = Some((host.clone(), Input::Reply(id.clone())));
            } else {
                return false;
            }
        }
        let (_, mode) = self.focused.clone().unwrap();
        let key_id = mode.key();
        let draft = self.drafts.entry(key_id.clone()).or_default();
        let text = texts.entry(key_id.clone()).or_default();
        match key.code {
            KeyCode::Esc => self.focused = None,
            KeyCode::Enter
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && !matches!(mode, Input::Rename(_)) =>
            {
                if !draft.locked() {
                    edit::insert(text, cursor, &key_id, "\n");
                    draft.delivery = None;
                }
            }
            KeyCode::Enter => self.submit(&host, mode, texts),
            KeyCode::Char('r')
                if key.modifiers == KeyModifiers::CONTROL
                    && matches!(mode, Input::Reply(_))
                    && !draft.locked() =>
            {
                if let Input::Reply(id) = &mode {
                    let agent = self
                        .summaries
                        .get(id)
                        .map(|row| row.value.agent_id.as_str());
                    draft.quote = self
                        .histories
                        .get(id)
                        .into_iter()
                        .flat_map(History::rows)
                        .find_map(|event| {
                            (Some(event.actor_id.as_str()) == agent
                                && matches!(&event.change, ThreadTimelineChange::Message { .. }))
                            .then(|| event.id.clone())
                        });
                    draft.delivery = None;
                }
            }
            _ if !draft.locked() => {
                let prior = text.clone();
                if !edit::edit(text, cursor, &key_id, key) {
                    return false;
                }
                if *text != prior {
                    draft.delivery = None;
                }
            }
            _ => {}
        }
        true
    }

    pub(super) fn paste(
        &mut self,
        pane: &Pane,
        text: &str,
        cursor: &edit::Cursor,
        texts: &mut HashMap<String, String>,
    ) -> bool {
        let Some((host, mode)) = &self.focused else {
            return false;
        };
        if host != &pane.key() {
            return false;
        }
        let key = mode.key();
        let draft = self.drafts.entry(key.clone()).or_default();
        if !draft.locked() {
            let text = if matches!(mode, Input::Rename(_)) {
                text.lines().next().unwrap_or_default()
            } else {
                text
            };
            edit::insert(texts.entry(key.clone()).or_default(), cursor, &key, text);
            draft.delivery = None;
        }
        true
    }

    fn fence(&self, id: &str) -> Option<Fence> {
        self.summaries.get(id).map(metadata_fence)
    }

    fn prepare_write(
        &self,
        host: &str,
        input: &Input,
        text: &str,
        draft: &Draft,
    ) -> Result<Write, String> {
        if !self.available || !self.allowed(input) || text.trim().is_empty() {
            return Err("The thread is not ready to send".into());
        }
        if draft
            .delivery
            .as_ref()
            .is_some_and(|delivery| delivery.pending)
        {
            return Err("The original action is still being sent".into());
        }
        let key = input.key();
        let write = if let Some(delivery) = &draft.delivery {
            delivery.write.clone()
        } else {
            let new_key = uuid::Uuid::now_v7().to_string();
            let (id, fence, mutation) = match input {
                Input::Start {
                    item,
                    episode: observed_episode,
                } => {
                    let Some(ask) = self.origins.get(item) else {
                        return Err("The thread context is not ready".into());
                    };
                    let ThreadInterest::Inbox { source, episode } = &ask.context else {
                        return Err("The thread context is not ready".into());
                    };
                    if episode != observed_episode {
                        return Err("The thread context is not ready".into());
                    }
                    let target_id = format!("thread/{}", uuid::Uuid::now_v7());
                    (
                        target_id.clone(),
                        Fence {
                            snapshot_id: ask.snapshot.clone(),
                            ..Fence::default()
                        },
                        Mutation::Start(ThreadStartParameters {
                            target_id,
                            agent_id: ask.agent.clone(),
                            origin: fractal_client::ThreadStartOrigin {
                                source_id: source.clone(),
                                episode: episode.clone(),
                            },
                            title: ask.title.clone(),
                            content: text.to_owned(),
                            signature: None,
                        }),
                    )
                }
                Input::Reply(id) => {
                    let Some(observed) = self.summaries.get(id) else {
                        return Err("The thread context is not ready".into());
                    };
                    (
                        id.clone(),
                        Fence {
                            snapshot_id: observed.snapshot.id.clone(),
                            ..Fence::default()
                        },
                        Mutation::Reply(ThreadReplyParameters {
                            target_id: id.clone(),
                            content: text.to_owned(),
                            thread_quote_id: draft.quote.clone(),
                            signature: None,
                        }),
                    )
                }
                Input::Rename(id) => {
                    let Some(observed) = draft
                        .rename_observed
                        .as_ref()
                        .or_else(|| self.summaries.get(id))
                    else {
                        return Err("The thread context is not ready".into());
                    };
                    (
                        id.clone(),
                        metadata_fence(observed),
                        Mutation::Rename(ThreadRenameParameters {
                            target_id: id.clone(),
                            title: text.trim().to_owned(),
                            title_revision: observed.value.title_revision.clone(),
                        }),
                    )
                }
            };
            let agent = match &mutation {
                Mutation::Start(start) => start.agent_id.clone(),
                _ => self
                    .summaries
                    .get(&id)
                    .ok_or("The thread recipient is unavailable")?
                    .value
                    .agent_id
                    .clone(),
            };
            Write {
                agent,
                key: new_key,
                id,
                host: host.into(),
                input: Some(key.clone()),
                fence,
                mutation,
                client: None,
            }
        };
        Ok(write)
    }

    fn retain_and_queue(&mut self, input: &Input, write: Write) {
        let key = input.key();
        let draft = self.drafts.entry(key).or_default();
        let unknown = draft
            .delivery
            .as_ref()
            .is_some_and(|delivery| delivery.unknown);
        draft.delivery = Some(Delivery {
            unknown,
            ..Delivery::new(write.clone())
        });
        if let Input::Start { item, .. } = input
            && let Some(origin) = self.start_origin(item)
        {
            self.retained_origins.insert(item.clone(), origin.clone());
        }
        if let Input::Start { item, .. } = input {
            self.clear_draft_collision(item);
        }
        self.writes.push(write);
    }

    fn submit(&mut self, host: &str, input: Input, texts: &HashMap<String, String>) {
        let key = input.key();
        if let (Some(draft), Some(text)) = (self.drafts.get(&key), texts.get(&key))
            && let Ok(write) = self.prepare_write(host, &input, text, draft)
        {
            self.retain_and_queue(&input, write);
        }
    }

    fn has_edit(&self, id: &str) -> bool {
        self.pending_actions.contains_key(id)
            || self
                .drafts
                .get(&Input::Rename(id.into()).key())
                .is_some_and(|draft| draft.delivery.is_some())
    }

    fn can_mark_read(&self, id: &str) -> bool {
        self.access.mark_read
            && !self.has_edit(id)
            && self
                .summary(id)
                .is_some_and(|summary| summary.unread == Some(true))
    }

    pub(super) fn mark_visible_read(
        &mut self,
        host: &str,
        id: String,
        visible: &HashSet<String>,
    ) -> bool {
        if !self.can_mark_read(&id) {
            return false;
        }
        let Some(observed) = self.summaries.get(&id) else {
            return false;
        };
        let seen_message_ids: Vec<_> = self
            .histories
            .get(&id)
            .into_iter()
            .flat_map(History::rows)
            .filter(|event| {
                visible.contains(&event.id)
                    && event.actor_id == observed.value.agent_id
                    && matches!(&event.change, ThreadTimelineChange::Message { .. })
            })
            .take(50)
            .map(|event| event.id.clone())
            .collect();
        let Some(anchor) = seen_message_ids.first().cloned() else {
            return false;
        };
        let write = Write {
            key: uuid::Uuid::now_v7().to_string(),
            id: id.clone(),
            host: host.into(),
            input: None,
            fence: Fence {
                snapshot_id: observed.snapshot.id.clone(),
                ..Fence::default()
            },
            client: None,
            agent: observed.value.agent_id.clone(),
            mutation: Mutation::MarkRead(ThreadReadParameters {
                target_id: id.clone(),
                through_message_id: Some(anchor),
                seen_message_ids,
            }),
        };
        self.selected.insert(host.into(), id.clone());
        self.pending_actions
            .insert(id, Delivery::new(write.clone()));
        self.writes.push(write);
        true
    }

    fn lifecycle(&mut self, host: &str, id: String, mutation: Mutation) {
        let allowed = match mutation {
            Mutation::Close => self.access.close,
            Mutation::Reopen => self.access.reopen,
            _ => false,
        };
        if !allowed {
            return;
        }
        if self.has_edit(&id) {
            return;
        }
        let Some(fence) = self.fence(&id) else {
            return;
        };
        let write = Write {
            key: uuid::Uuid::now_v7().to_string(),
            id: id.clone(),
            host: host.into(),
            input: None,
            fence,
            mutation,
            client: None,
            agent: self.summary(&id).unwrap().agent_id.clone(),
        };
        self.pending_actions
            .insert(id, Delivery::new(write.clone()));
        self.writes.push(write);
    }

    pub(super) fn take_earlier(&mut self) -> Vec<Earlier> {
        std::mem::take(&mut self.earlier)
    }

    pub(super) fn earlier_loaded(
        &mut self,
        id: &str,
        page: Result<fractal_client::ThreadTimelinePage, String>,
    ) -> Option<String> {
        let history = self.histories.get_mut(id)?;
        history.loading = false;
        match page {
            Ok(page) => {
                history.earlier.extend(page.items);
                history.cursor = page.next_cursor;
                history.paged = true;
                None
            }
            Err(message) => Some(message),
        }
    }

    pub(super) fn bind(&mut self, write: &mut Write, client: Client) -> Result<(), ClientError> {
        write.prepare(client)?;
        let retained = if let Some(key) = &write.input {
            self.drafts
                .get_mut(key)
                .and_then(|draft| draft.delivery.as_mut())
        } else {
            self.pending_actions.get_mut(&write.id)
        };
        if let Some(retained) = retained.filter(|retained| retained.write.key == write.key) {
            retained.write = write.clone();
        }
        Ok(())
    }

    pub(super) fn writes(&mut self) -> Vec<Write> {
        std::mem::take(&mut self.writes)
    }

    pub(super) fn complete(
        &mut self,
        receipt: Receipt,
        texts: &mut HashMap<String, String>,
    ) -> Option<String> {
        let Receipt {
            write,
            confirmation,
        } = receipt;
        let belongs = if let Some(key) = &write.input {
            self.drafts
                .get(key)
                .and_then(|draft| draft.delivery.as_ref())
        } else {
            self.pending_actions.get(&write.id)
        }
        .is_some_and(|delivery| delivery.write.key == write.key);
        if !belongs {
            return None;
        }
        let not_sent = matches!(&confirmation, Confirmation::NotSent(_));
        if let Confirmation::Failed(error) | Confirmation::NotSent(error) = confirmation {
            let delivery = if let Some(key) = &write.input {
                self.drafts
                    .get_mut(key)
                    .and_then(|draft| draft.delivery.as_mut())
            } else {
                self.pending_actions.get_mut(&write.id)
            }
            .unwrap();
            // Capture the original authenticated member even if a reconnect changes the active client.
            delivery.write = write.clone();
            let keep = if not_sent {
                delivery.pending = false;
                delivery.unknown
            } else {
                delivery.failed(&error)
            };
            if !keep {
                if matches!(write.mutation, Mutation::Start(_))
                    && let Some(item) = write.host.strip_prefix("home:")
                {
                    self.retained_origins.remove(item);
                }
                if let Some(key) = &write.input {
                    let draft = self.drafts.get_mut(key).unwrap();
                    draft.delivery = None;
                    draft.rename_observed = None;
                } else {
                    self.pending_actions.remove(&write.id);
                }
            }
            return Some(error.human());
        }
        if write.input.is_none() {
            self.pending_actions.remove(&write.id);
        }
        if matches!(confirmation, Confirmation::Rejected) {
            if matches!(write.mutation, Mutation::Start(_))
                && let Some(item) = write.host.strip_prefix("home:")
            {
                self.retained_origins.remove(item);
            }
            if let Some(key) = &write.input {
                let draft = self.drafts.get_mut(key).unwrap();
                draft.delivery = None;
                draft.rename_observed = None;
            }
            return Some("The original thread action was rejected".into());
        }
        let Confirmation::Applied(summary) = confirmation else {
            unreachable!()
        };
        if let Some(summary) = summary {
            self.put_summary(summary);
        }
        if let Some(key) = &write.input {
            self.drafts.remove(key);
            texts.remove(key);
        }
        match write.mutation {
            Mutation::Start(start) => {
                if let Some(item) = write.host.strip_prefix("home:") {
                    self.retained_origins.remove(item);
                }
                self.start_slot = Some((
                    ThreadInterest::Inbox {
                        source: start.origin.source_id,
                        episode: start.origin.episode,
                    },
                    write.id.clone(),
                ));
                self.open
                    .entry(write.host.clone())
                    .or_default()
                    .insert(write.id.clone());
                self.selected.insert(write.host.clone(), write.id.clone());
                self.focused = self
                    .access
                    .reply
                    .then_some((write.host, Input::Reply(write.id)));
            }
            Mutation::Reopen => {
                self.focused = self
                    .access
                    .reply
                    .then_some((write.host, Input::Reply(write.id)))
            }
            Mutation::Rename(_) => self.focused = None,
            // Closing does not end the conversation or blur its composer.
            _ => {}
        }
        None
    }
}

fn metadata_fence(observed: &Envelope<ThreadSummary>) -> Fence {
    Fence {
        snapshot_id: observed.snapshot.id.clone(),
        subject_revisions: [(observed.value.id.clone(), observed.value.revision.clone())].into(),
        ..Fence::default()
    }
}

fn shell_doc(doc: thread::Doc, host: &str, thread: &str) -> Doc {
    let mut targets: Vec<_> = doc
        .targets
        .into_iter()
        .map(|target| super::doc::Target {
            line: target.line,
            column: target.column,
            width: target.width,
            hit: Hit::Thread(host.into(), target.hit),
        })
        .collect();
    for (message, body) in doc.messages {
        for line in body {
            let width = doc.lines[line].width() as u16;
            if width > 0 {
                targets.push(super::doc::Target {
                    line,
                    column: 0,
                    width,
                    hit: Hit::ThreadBody {
                        host: host.into(),
                        thread: thread.into(),
                        message: message.clone(),
                    },
                });
            }
        }
    }
    Doc {
        lines: doc.lines,
        messages: Vec::new(),
        entries: doc.entries,
        targets,
    }
}

#[cfg(test)]
mod tests;
