//! Thread mounts share the authenticated feed and native editor. The UI keeps disclosure and
//! drafts local. Accepted metadata, message history and unread state come from the member.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use fractal_client::{
    Client, ClientError, Envelope, Fence, Snapshot, ThreadCollectionRows, ThreadRenameParameters,
    ThreadReplyParameters, ThreadStartParameters, ThreadState, ThreadSummary,
    ThreadTargetParameters, ThreadTimelineChange, ThreadTimelineEvent,
};
use fractal_feed::{ThreadInterest, model::Model};
use ratatui::text::{Line, Span};

use super::{
    composer,
    doc::{Doc, DocExt, Hit},
    edit,
    pane::Pane,
    text, theme, thread,
};

#[derive(Clone)]
struct Ask {
    context: ThreadInterest,
    agent: String,
    title: String,
    snapshot: String,
}

#[derive(Default)]
struct Draft {
    quote: Option<String>,
    rename_observed: Option<Envelope<ThreadSummary>>,
    retry: Option<Write>,
    pending: bool,
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
}

pub(super) struct Receipt {
    write: Write,
    result: Result<Option<Envelope<ThreadSummary>>, ClientError>,
}

impl Write {
    pub(super) fn offline(self) -> Receipt {
        Receipt {
            write: self,
            result: Err(ClientError::Transport(
                "Reconnect to send this thread action".into(),
            )),
        }
    }
    pub(super) async fn perform(self, client: Client) -> Receipt {
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
            Mutation::Close => client.thread_close(&action, key, fence, target()).await,
            Mutation::Reopen => client.thread_reopen(&action, key, fence, target()).await,
        };
        let result = match result {
            Ok(result) if result.value.status == fractal_client::ActionStatus::Completed => {
                Ok(client.thread_get(&self.id).await.ok())
            }
            Ok(result) => Err(ClientError::Protocol(format!(
                "thread action returned {:?}",
                result.value.status
            ))),
            Err(error) => Err(error),
        };
        Receipt {
            write: self,
            result,
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
    asks: BTreeMap<String, Ask>,
    sessions: BTreeMap<String, String>,
    lists: BTreeMap<ThreadInterest, Vec<String>>,
    summaries: BTreeMap<String, Envelope<ThreadSummary>>,
    histories: BTreeMap<String, History>,
    earlier: Vec<Earlier>,
    failures: BTreeMap<ThreadInterest, String>,
    open: BTreeMap<String, String>,
    folded_inbox_details: BTreeSet<String>,
    focused: Option<(String, Input)>,
    drafts: BTreeMap<String, Draft>,
    writes: Vec<Write>,
    pending_actions: BTreeMap<String, Write>,
    hover: Option<String>,
}

impl Threads {
    pub(super) fn enable(&mut self, available: bool) {
        self.available = available;
        if !available {
            self.lists.clear();
            self.summaries.clear();
            self.histories.clear();
            self.earlier.clear();
            self.failures.clear();
            self.focused = None;
            self.writes.clear();
            self.pending_actions.clear();
            for draft in self.drafts.values_mut() {
                draft.pending = false;
            }
        }
    }

    pub(super) fn available(&self) -> bool {
        self.available
    }

    pub(super) fn can_start(&self, id: &str) -> bool {
        self.available && self.asks.contains_key(id)
    }

    fn start_input(&self, item: &str) -> Option<Input> {
        let ThreadInterest::Ask { episode, .. } = &self.asks.get(item)?.context else {
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
            Pane::Home(Some(id)) if id.starts_with("thread/") => id,
            _ => self.open.get(&pane.key())?,
        };
        let summary = self.summary(id)?;
        Some(match key.code {
            KeyCode::Char('Z' | 'z') if summary.state == ThreadState::Open => {
                thread::Tap::Close(id.clone())
            }
            KeyCode::Char('Z' | 'z') => thread::Tap::Reopen(id.clone()),
            KeyCode::Char('N' | 'n') => thread::Tap::Rename(id.clone()),
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
        self.asks.clear();
        let Some(snapshot) = &model.now.snapshot else {
            return;
        };
        for item in model.attention() {
            if item.episode.is_empty() || !item.source_id.starts_with("step-run/") {
                continue;
            }
            let Some(agent) = item
                .requester_id
                .as_ref()
                .filter(|id| id.starts_with("agent/"))
            else {
                continue;
            };
            self.asks.insert(
                item.header.id.clone(),
                Ask {
                    context: ThreadInterest::Ask {
                        id: item.source_id.clone(),
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
            Pane::Home(Some(id)) => self.asks.get(id).map(|ask| ask.context.clone()),
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
            out.push(ThreadInterest::Timeline(id.clone()));
        } else if let Pane::Home(Some(id)) = pane
            && id.starts_with("thread/")
        {
            if !self.folded_inbox_details.contains(&pane.key()) {
                out.push(ThreadInterest::Timeline(id.clone()));
            }
        } else if let Some(id) = self.open.get(&pane.key()) {
            out.push(ThreadInterest::Timeline(id.clone()));
        }
        out
    }

    pub(super) fn keep_windows(&mut self, wanted: &[ThreadInterest]) {
        self.lists.retain(|interest, _| wanted.contains(interest));
        self.failures
            .retain(|interest, _| wanted.contains(interest));
        let held = |id: &str| {
            wanted.iter().any(
                |interest| matches!(interest, ThreadInterest::Timeline(thread) if thread == id),
            )
        };
        self.histories.retain(|id, _| held(id));
        self.summaries
            .retain(|id, _| held(id) || self.lists.values().any(|ids| ids.contains(id)));
    }

    pub(super) fn summary(&self, id: &str) -> Option<&ThreadSummary> {
        self.summaries.get(id).map(|row| &row.value)
    }

    pub(super) fn detail(
        &mut self,
        id: &str,
        result: Result<Envelope<ThreadSummary>, ClientError>,
    ) {
        match result {
            Ok(summary) => {
                self.failures.remove(&ThreadInterest::Timeline(id.into()));
                self.put_summary(summary);
            }
            Err(error) => {
                self.failures
                    .insert(ThreadInterest::Timeline(id.into()), error.human());
                self.summaries.remove(id);
            }
        }
    }

    fn put_summary(&mut self, summary: Envelope<ThreadSummary>) {
        let id = &summary.value.id;
        if !self.summaries.get(id).is_some_and(|old| {
            old.snapshot.host_id == summary.snapshot.host_id
                && old.snapshot.store_index > summary.snapshot.store_index
        }) {
            if summary.value.state == ThreadState::Closed
                && self
                    .focused
                    .as_ref()
                    .is_some_and(|(_, mode)| matches!(mode, Input::Reply(active) if active == id))
            {
                self.focused = None;
            }
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
            if let ThreadInterest::Timeline(id) = &interest {
                self.histories.remove(id);
                self.summaries.remove(id);
            }
            self.lists.remove(&interest);
        }
        self.failures.insert(interest, message);
    }

    pub(super) fn incoming_visible(&self, id: &str, person: &str) -> bool {
        self.histories
            .values()
            .flat_map(History::rows)
            .any(|event| {
                event.id == id
                    && event.actor_id != person
                    && matches!(&event.change, ThreadTimelineChange::Message { .. })
            })
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
            if ids.is_some_and(|ids| !ids.is_empty()) {
                doc.blank();
                doc.section("Threads", None, width);
            }
            for id in ids.into_iter().flatten() {
                self.panel(
                    &mut doc,
                    &host,
                    id,
                    self.open.get(&host) == Some(id),
                    false,
                    width,
                    cursor,
                    world,
                    texts,
                );
            }
            if let Some(message) = self.failures.get(&context) {
                doc.wrap(&[text::run(message, theme::dim())], width);
            }
            if let Pane::Home(Some(item)) = pane
                && ids.is_some()
                && let Some(input) = self.start_input(item)
            {
                if self.focused.as_ref() == Some(&(host.clone(), input.clone()))
                    || texts.contains_key(&input.key())
                {
                    self.editor(
                        &mut doc,
                        &host,
                        &input,
                        "First message in thread",
                        cursor,
                        width,
                        texts,
                    );
                } else {
                    doc.buttons(&[(
                        "T",
                        "Start thread",
                        Hit::Thread(host.clone(), thread::Tap::Start(item.clone())),
                        theme::SAPPHIRE,
                    )]);
                }
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
        if open && (summary.is_none() || !self.histories.contains_key(id)) {
            let reason = self
                .failures
                .get(&timeline)
                .map(String::as_str)
                .unwrap_or("Loading thread…");
            doc.wrap(&[text::run(reason, theme::dim())], width);
            if standalone || summary.is_none() {
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
                open: open && self.histories.contains_key(id),
                standalone,
                draft: draft_text,
                cursor: focus.then(|| cursor.at(&key, draft_text)),
                reply_to,
                hovered_message: self.hover.as_deref(),
                has_earlier: source.is_some_and(History::more),
                loading_earlier: source.is_some_and(|history| history.loading),
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
            doc.append(shell_doc(rendered, host), 0);
            if draft.is_some_and(|draft| draft.pending) {
                doc.line(Line::from(Span::styled("Sending…", theme::dim())));
            }
        }
        if self.focused.as_ref() == Some(&(host.into(), Input::Rename(id.into()))) {
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
        doc.buttons(&[("enter", label, Hit::Thread(host.into(), tap), theme::ACCENT)]);
        doc.blank();
    }

    pub(super) fn hover(&mut self, message: Option<String>) {
        self.hover = message;
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
        match tap {
            thread::Tap::Toggle(id) => {
                if host.starts_with("home:thread/") {
                    if !self.folded_inbox_details.remove(host) {
                        self.folded_inbox_details.insert(host.into());
                        self.focused = None;
                    } else if self
                        .summary(&id)
                        .is_some_and(|row| row.state == ThreadState::Open)
                    {
                        self.focused = Some((host.into(), Input::Reply(id)));
                    }
                    return None;
                }
                if self.open.get(host) == Some(&id) {
                    self.open.remove(host);
                    self.focused = None;
                } else {
                    self.open.insert(host.into(), id.clone());
                    if self
                        .summary(&id)
                        .is_some_and(|row| row.state == ThreadState::Open)
                    {
                        self.focused = Some((host.into(), Input::Reply(id)));
                    }
                }
            }
            thread::Tap::Compose(id) => self.focused = Some((host.into(), Input::Reply(id))),
            thread::Tap::Reply { thread, message } => {
                let input = Input::Reply(thread);
                let draft = self.drafts.entry(input.key()).or_default();
                draft.quote = Some(message);
                draft.retry = None;
                self.focused = Some((host.into(), input));
            }
            thread::Tap::Start(item) => {
                if let Some(input) = self.start_input(&item) {
                    self.activate(host, input, texts);
                }
            }
            thread::Tap::Rename(id) => {
                self.activate(host, Input::Rename(id), texts);
            }
            thread::Tap::Send(id) => self.submit(host, Input::Reply(id), texts),
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
            thread::Tap::MessageHeader(_) => {}
        }
        None
    }

    fn activate(&mut self, host: &str, input: Input, texts: &mut HashMap<String, String>) {
        if self.focused.as_ref() == Some(&(host.into(), input.clone())) {
            self.submit(host, input, texts);
        } else {
            let key = input.key();
            if let Input::Rename(id) = &input {
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
            .is_some_and(|(place, _)| place != &host)
        {
            self.focused = None;
        }
        if self.focused.is_none() {
            if key.code == KeyCode::Char('t')
                && key.modifiers.is_empty()
                && let Pane::Home(Some(item)) = pane
                && let Some(input) = self.start_input(item)
            {
                self.activate(&host, input, texts);
                return true;
            }
            if let Pane::Thread(id) = pane
                && matches!(key.code, KeyCode::Char(_))
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && self
                    .summary(id)
                    .is_some_and(|row| row.state == ThreadState::Open)
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
                if !draft.pending {
                    edit::insert(text, cursor, &key_id, "\n");
                    draft.retry = None;
                }
            }
            KeyCode::Enter => self.submit(&host, mode, texts),
            KeyCode::Char('r')
                if key.modifiers == KeyModifiers::CONTROL && matches!(mode, Input::Reply(_)) =>
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
                    draft.retry = None;
                }
            }
            _ if !draft.pending => {
                let prior = text.clone();
                if !edit::edit(text, cursor, &key_id, key) {
                    return false;
                }
                if *text != prior {
                    draft.retry = None;
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
        if !draft.pending {
            let text = if matches!(mode, Input::Rename(_)) {
                text.lines().next().unwrap_or_default()
            } else {
                text
            };
            edit::insert(texts.entry(key.clone()).or_default(), cursor, &key, text);
            draft.retry = None;
        }
        true
    }

    fn fence(&self, id: &str) -> Option<Fence> {
        self.summaries.get(id).map(metadata_fence)
    }

    fn submit(&mut self, host: &str, input: Input, texts: &HashMap<String, String>) {
        let key = input.key();
        let Some(draft) = self.drafts.get(&key) else {
            return;
        };
        let Some(text) = texts.get(&key).filter(|text| !text.trim().is_empty()) else {
            return;
        };
        if draft.pending {
            return;
        }
        let write = if let Some(retry) = &draft.retry {
            retry.clone()
        } else {
            let new_key = uuid::Uuid::now_v7().to_string();
            let (id, fence, mutation) = match &input {
                Input::Start {
                    item,
                    episode: observed_episode,
                } => {
                    let Some(ask) = self.asks.get(item) else {
                        return;
                    };
                    let ThreadInterest::Ask { id, episode } = &ask.context else {
                        return;
                    };
                    if episode != observed_episode {
                        return;
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
                            origin_ask: id.clone(),
                            origin_ask_episode: episode.clone(),
                            title: ask.title.clone(),
                            content: text.clone(),
                            signature: None,
                        }),
                    )
                }
                Input::Reply(id) => {
                    let Some(observed) = self.summaries.get(id) else {
                        return;
                    };
                    if observed.value.state != ThreadState::Open {
                        return;
                    }
                    (
                        id.clone(),
                        Fence {
                            snapshot_id: observed.snapshot.id.clone(),
                            ..Fence::default()
                        },
                        Mutation::Reply(ThreadReplyParameters {
                            target_id: id.clone(),
                            content: text.clone(),
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
                        return;
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
            Write {
                key: new_key,
                id,
                host: host.into(),
                input: Some(key.clone()),
                fence,
                mutation,
            }
        };
        if let Some(draft) = self.drafts.get_mut(&key) {
            draft.pending = true;
            draft.retry = Some(write.clone());
        }
        self.writes.push(write);
    }

    fn lifecycle(&mut self, host: &str, id: String, mutation: Mutation) {
        if self.pending_actions.contains_key(&id) {
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
        };
        self.pending_actions.insert(id, write.clone());
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

    pub(super) fn writes(&mut self) -> Vec<Write> {
        std::mem::take(&mut self.writes)
    }

    pub(super) fn complete(
        &mut self,
        receipt: Receipt,
        texts: &mut HashMap<String, String>,
    ) -> Option<String> {
        let Receipt { write, result } = receipt;
        self.pending_actions.remove(&write.id);
        if let Some(key) = &write.input
            && let Some(draft) = self.drafts.get_mut(key)
        {
            draft.pending = false;
        }
        match result {
            Ok(summary) => {
                if let Some(summary) = summary {
                    self.put_summary(summary);
                }
                if let Some(key) = &write.input {
                    self.drafts.remove(key);
                    texts.remove(key);
                }
                match write.mutation {
                    Mutation::Start(_) => {
                        self.open.insert(write.host.clone(), write.id.clone());
                        self.focused = Some((write.host, Input::Reply(write.id)));
                    }
                    Mutation::Reopen => self.focused = Some((write.host, Input::Reply(write.id))),
                    Mutation::Rename(_) | Mutation::Close => self.focused = None,
                    _ => {}
                }
                None
            }
            Err(error) => {
                if matches!(&error, ClientError::Api(..))
                    && let Some(key) = &write.input
                    && let Some(draft) = self.drafts.get_mut(key)
                {
                    draft.retry = None;
                    draft.rename_observed = None;
                }
                Some(error.human())
            }
        }
    }
}

fn metadata_fence(observed: &Envelope<ThreadSummary>) -> Fence {
    Fence {
        snapshot_id: observed.snapshot.id.clone(),
        subject_revisions: [(observed.value.id.clone(), observed.value.revision.clone())].into(),
        ..Fence::default()
    }
}

fn shell_doc(doc: thread::Doc, host: &str) -> Doc {
    Doc {
        lines: doc.lines,
        messages: doc.messages,
        entries: doc.entries,
        targets: doc
            .targets
            .into_iter()
            .map(|target| super::doc::Target {
                line: target.line,
                column: target.column,
                width: target.width,
                hit: Hit::Thread(host.into(), target.hit),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
