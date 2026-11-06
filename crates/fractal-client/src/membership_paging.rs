//! Explicit collection-v1 adoption, without capability advertisement or automatic retries.
use crate::*;
use std::collections::HashSet;

impl Client {
    async fn require_membership_paging(&self) -> Result<(), ClientError> {
        if !self.capabilities_internal().await?.value.grants("ordered-memberships.paging.v1", 1) {
            return Err(ClientError::Protocol("unsupported-membership-paging-contract: capability not granted".into()));
        }
        Ok(())
    }
    /// Request authoritative collection metadata, refusing legacy or unknown contracts.
    pub async fn arrangements_memberships_collection_v1(&self, person_name: &str, uuid: &str, person: &str, cursor: Option<&str>, limit: Option<usize>) -> Result<Envelope<OrderedMembershipPage>, ClientError> {
        self.require_membership_paging().await?;
        let mut path = format!("/v1/client/arrangements/{}/{}/memberships?person={}&paging_contract=collection-v1", percent_encode(person_name), percent_encode(uuid), percent_encode(person));
        if let Some(cursor) = cursor { path.push_str(&format!("&cursor={}", percent_encode(cursor))); }
        if let Some(limit) = limit { path.push_str(&format!("&limit={limit}")); }
        let response: Envelope<OrderedMembershipPage> = self.get(&path).await?;
        Frontier::read(&response.snapshot, response.value.paging_contract.as_deref(), response.value.membership.as_ref())
            .map_err(|code| ClientError::Protocol(format!("collection-v1 metadata refused: {code:?}")))?;
        Ok(response)
    }
}
impl Client {
    /// Subscribe FIRST. Keep this joiner only for this connection and subscription generation.
    pub async fn subscribe_ordered_memberships_collection_v1(&self, stream: &mut CollectionStream, id: &str, person: &str, subject: &str, limit: usize) -> Result<MembershipJoin, ClientError> {
        self.require_membership_paging().await?;
        stream.send(&serde_json::json!({"kind":"subscribe","id":id,"collection":"ordered-memberships","person":person,"subject":subject,"limit":limit,"paging_contract":"collection-v1"})).await?;
        Ok(MembershipJoin::after_subscription(id, person, subject))
    }
}
#[derive(Clone, Debug)]
struct Frontier { host: String, projection: String, state: OrderedMembershipState }
impl Frontier {
    fn read(snapshot: &Snapshot, contract: Option<&str>, state: Option<&OrderedMembershipState>) -> Result<Self, ErrorCode> {
        if contract != Some("collection-v1") { return Err(ErrorCode::UnsupportedMembershipPagingContract); }
        let state = state.filter(|s| s.epoch.as_ref().is_some_and(|epoch| !epoch.is_empty())).ok_or(ErrorCode::MembershipFrontierInconsistent)?;
        if state.changed_index > snapshot.store_index { return Err(ErrorCode::MembershipFrontierInconsistent); }
        Ok(Self { host: snapshot.host_id.clone(), projection: snapshot.projection_version.clone(), state: state.clone() })
    }
    fn compare(&self, other: &Self) -> Result<std::cmp::Ordering, ErrorCode> {
        if self.host != other.host || self.projection != other.projection || self.state.container != other.state.container || self.state.epoch != other.state.epoch {
            return Err(ErrorCode::MembershipFrontierReset);
        }
        let order = self.state.changed_index.cmp(&other.state.changed_index);
        if order.is_eq() && self.state.live_count != other.state.live_count { return Err(ErrorCode::MembershipFrontierInconsistent); }
        Ok(order)
    }
}
fn row_key(row: &OrderedMembership) -> (&str, &str, &str) {
    (row.position.bucket.as_deref().unwrap_or(""), row.position.key.as_str(), row.member.as_str())
}
/// Attempt identity: pass it back with every HTTP response, including late responses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MembershipAttempt(u64);
#[derive(Debug, PartialEq, Eq)]
pub enum MembershipProgress { Pending, Complete, Ignored, Restart }
/// Stages canonical pages separately from the last complete view. Never applies window deltas.
/// Person/container are fixed; replace the joiner on selection or connection changes.
#[derive(Debug)]
pub struct MembershipJoin {
    id: String, person: String, container: String, generation: u64,
    stream: Option<Frontier>, staged: Option<Frontier>, accepted: Option<Frontier>,
    rows: Vec<OrderedMembership>, members: HashSet<String>, complete: Vec<OrderedMembership>, stale: bool, published: bool,
}
impl MembershipJoin {
    /// For externally managed subscriptions; call only AFTER sending the collection-v1 subscribe.
    pub fn after_subscription(id: &str, person: &str, container: &str) -> Self {
        Self { id: id.into(), person: person.into(), container: container.into(), generation: 0, stream: None, staged: None, accepted: None, rows: Vec::new(), members: HashSet::new(), complete: Vec::new(), stale: true, published: false }
    }
    /// Start at page one; never reuse a continuation from an invalidated attempt.
    pub fn begin(&mut self) -> MembershipAttempt {
        self.generation = self.generation.checked_add(1).expect("membership attempt exhausted");
        self.staged = None; self.rows.clear(); self.members.clear();
        MembershipAttempt(self.generation)
    }
    pub fn complete(&self) -> Option<&[OrderedMembership]> { self.published.then_some(self.complete.as_slice()) }
    pub fn is_stale(&self) -> bool { self.stale }
    /// Feed transport/API errors here; the two 409s invalidate the WHOLE attempt, not its cursor.
    pub fn api_error(&mut self, attempt: MembershipAttempt, code: ErrorCode) -> MembershipProgress {
        if attempt != MembershipAttempt(self.generation) { return MembershipProgress::Ignored; }
        self.begin();
        if matches!(code, ErrorCode::MembershipFrontierReset) { self.accepted = None; self.stream = None; }
        if matches!(code, ErrorCode::MembershipCollectionChanged | ErrorCode::MembershipFrontierReset) { self.stale = true; MembershipProgress::Restart } else { MembershipProgress::Ignored }
    }
    pub fn stream(&mut self, event: &CollectionEvent) -> Result<MembershipProgress, ErrorCode> {
        let (id, snapshot, state, contract) = match event {
            CollectionEvent::MembershipSnapshot { id, snapshot, membership, paging_contract, .. } |
            CollectionEvent::MembershipChanges { id, snapshot, membership, paging_contract, .. } => (id, snapshot, membership, paging_contract),
            _ => return Ok(MembershipProgress::Ignored),
        };
        if id != &self.id { return Ok(MembershipProgress::Ignored); }
        let result = (|| {
            let next = Frontier::read(snapshot, contract.as_deref(), Some(state))?;
            if next.state.container != self.container { return Err(ErrorCode::MembershipFrontierReset); }
            for prior in [&self.staged, &self.stream, &self.accepted].into_iter().flatten() {
                if next.compare(prior)?.is_lt() { return Ok(MembershipProgress::Ignored); }
            }
            let refresh = self.staged.as_ref().or(self.accepted.as_ref()).map(|prior| next.compare(prior)).transpose()?.is_some_and(|order| order.is_gt());
            self.stream = Some(next);
            if refresh { self.stale = true; self.begin(); return Ok(MembershipProgress::Restart); }
            Ok(MembershipProgress::Pending)
        })();
        if let Err(code) = &result {
            if matches!(code, ErrorCode::MembershipFrontierReset) { self.accepted = None; }
            self.stream = None; self.stale = true; self.begin();
        }
        result
    }
    pub fn page(&mut self, attempt: MembershipAttempt, response: Envelope<OrderedMembershipPage>) -> Result<MembershipProgress, ErrorCode> {
        if attempt != MembershipAttempt(self.generation) { return Ok(MembershipProgress::Ignored); }
        let result = self.stage(response);
        if let Err(code) = &result {
            if matches!(code, ErrorCode::MembershipFrontierReset) { self.accepted = None; self.stream = None; }
            self.begin(); self.stale = true;
        }
        result
    }
    fn stage(&mut self, response: Envelope<OrderedMembershipPage>) -> Result<MembershipProgress, ErrorCode> {
        let page = response.value;
        let next = Frontier::read(&response.snapshot, page.paging_contract.as_deref(), page.membership.as_ref())?;
        if next.state.container != self.container || page.filters.get("person").is_some_and(|p| p != &self.person) || page.filters.get("subject").is_some_and(|s| s != &self.container) { return Err(ErrorCode::MembershipFrontierReset); }
        if let Some(prior) = &self.staged {
            if !next.compare(prior)?.is_eq() { return Err(ErrorCode::MembershipCollectionChanged); }
        }
        if let Some(stream) = &self.stream {
            if stream.compare(&next)?.is_gt() { return Err(ErrorCode::MembershipCollectionChanged); }
        }
        if let Some(accepted) = &self.accepted {
            if next.compare(accepted)?.is_lt() { return Err(ErrorCode::MembershipCollectionChanged); }
        }
        if page.page.has_more && (page.items.is_empty() || page.page.next_cursor.is_none()) { return Err(ErrorCode::MembershipFrontierInconsistent); }
        for row in page.items {
            if row.id != row.member || row.container != self.container || !self.members.insert(row.member.clone()) || self.rows.last().is_some_and(|prior| row_key(prior) >= row_key(&row)) { return Err(ErrorCode::MembershipFrontierInconsistent); }
            self.rows.push(row);
        }
        if u64::try_from(self.rows.len()).unwrap_or(u64::MAX) > next.state.live_count { return Err(ErrorCode::MembershipFrontierInconsistent); }
        if page.page.has_more { self.staged = Some(next); return Ok(MembershipProgress::Pending); }
        if u64::try_from(self.rows.len()).ok() != Some(next.state.live_count) { return Err(ErrorCode::MembershipFrontierInconsistent); }
        self.complete = std::mem::take(&mut self.rows); self.accepted = Some(next); self.stale = false; self.published = true;
        self.begin();
        Ok(MembershipProgress::Complete)
    }
}
