import Foundation

public enum MembershipPagingFailure: Error, Equatable {
    case refused, inconsistent, restart, obsoleteAttempt
    case server(String)
}

/// One instance per subscribed connection and selected person/container. Serialize access.
/// Keep `complete` visible while `isStale`; window deltas never reconstruct a full collection.
public struct MembershipPagingJoin {
    private struct Frontier: Equatable {
        let host, projection, epoch: String
        let index, count: UInt64
        func sameScope(_ other: Self) -> Bool { host == other.host && projection == other.projection && epoch == other.epoch }
    }
    public let person: String
    public let container: String
    public private(set) var complete: [OrderedMembership]?
    public private(set) var isStale = true
    private var stream: Frontier?, staged: Frontier?, published: Frontier?
    private var attempt: UUID?
    private var rows: [OrderedMembership] = []
    private var members: Set<String> = []

    public init(person: String, container: String) { self.person = person; self.container = container }
    /// Call only after subscribing; retain the token with every HTTP request in this attempt.
    public mutating func beginAttempt() -> UUID {
        let token = UUID(); attempt = token; staged = nil; rows = []; members = []; isStale = true
        return token
    }
    private mutating func invalidate() { attempt = nil; staged = nil; rows = []; members = []; isStale = true }
    public static func validate(contract: String?, membership: OrderedMembershipState?) throws {
        guard contract == "collection-v1", membership?.epoch != nil else { throw MembershipPagingFailure.refused }
    }
    public static func validate(_ frame: OrderedMembershipCollectionFrame) throws {
        if frame.kind == "resync" { throw MembershipPagingFailure.restart }
        if frame.kind == "error" {
            if frame.code == "membership-collection-changed" || frame.code == "membership-frontier-reset" { throw MembershipPagingFailure.restart }
            throw MembershipPagingFailure.server(frame.code ?? "unknown")
        }
        guard frame.kind == "snapshot" || frame.kind == "changes", frame.snapshot != nil else { throw MembershipPagingFailure.refused }
        try validate(contract: frame.pagingContract, membership: frame.membership)
    }
    private func frontier(_ snapshot: Snapshot, _ state: OrderedMembershipState) throws -> Frontier {
        guard state.container == container else { throw MembershipPagingFailure.restart }
        guard let epoch = state.epoch else { throw MembershipPagingFailure.refused }
        guard state.changedIndex <= snapshot.storeIndex else { throw MembershipPagingFailure.inconsistent }
        return Frontier(host: snapshot.hostID, projection: snapshot.projectionVersion, epoch: epoch, index: state.changedIndex, count: state.liveCount)
    }
    private func compare(_ lhs: Frontier, _ rhs: Frontier) throws {
        guard lhs.sameScope(rhs) else { throw MembershipPagingFailure.restart }
        if lhs.index == rhs.index && lhs.count != rhs.count { throw MembershipPagingFailure.inconsistent }
    }
    public mutating func observe(_ frame: OrderedMembershipCollectionFrame) throws {
        do {
            try Self.validate(frame)
            guard let snapshot = frame.snapshot, let state = frame.membership else { throw MembershipPagingFailure.refused }
            let incoming = try frontier(snapshot, state)
            if let stream, !incoming.sameScope(stream) { self.stream = incoming; throw MembershipPagingFailure.restart }
            if let stream { try compare(incoming, stream); if incoming.index < stream.index { return } }
            stream = incoming
            if let staged {
                try compare(incoming, staged)
                if incoming.index > staged.index { throw MembershipPagingFailure.restart }
            } else if let published {
                try compare(incoming, published)
                if incoming.index > published.index { throw MembershipPagingFailure.restart }
            }
        } catch { invalidate(); throw error }
    }
    /// A 409 abandons the attempt, never retries its cursor. All other errors remain errors.
    public mutating func reject(_ error: ErrorEnvelope, attempt token: UUID) throws {
        guard attempt == token else { throw MembershipPagingFailure.obsoleteAttempt }
        invalidate()
        if error.code == .membershipCollectionChanged || error.code == .membershipFrontierReset { throw MembershipPagingFailure.restart }
        throw error
    }
    /// Returns the complete canonical set only on a consistent terminal page.
    public mutating func append(_ response: Envelope<OrderedMembershipPage>, attempt token: UUID) throws -> [OrderedMembership]? {
        guard attempt == token else { throw MembershipPagingFailure.obsoleteAttempt }
        do {
            let page = response.value
            try Self.validate(contract: page.pagingContract, membership: page.membership)
            guard let state = page.membership else { throw MembershipPagingFailure.refused }
            let incoming = try frontier(response.snapshot, state)
            guard page.filters["person"] == person else { throw MembershipPagingFailure.restart }
            if let staged { try compare(incoming, staged); guard incoming.index == staged.index else { throw MembershipPagingFailure.restart } }
            if let stream {
                if !incoming.sameScope(stream) { self.stream = nil; throw MembershipPagingFailure.restart }
                try compare(incoming, stream)
                guard incoming.index >= stream.index else { throw MembershipPagingFailure.restart }
            }
            staged = incoming
            for row in page.items {
                guard row.container == container, members.insert(row.member).inserted else { throw MembershipPagingFailure.inconsistent }
                if let previous = rows.last {
                    let left = [previous.position.bucket ?? "", previous.position.key, previous.member]
                    let right = [row.position.bucket ?? "", row.position.key, row.member]
                    guard let first = zip(left, right).first(where: { $0 != $1 }), first.0.utf8.lexicographicallyPrecedes(first.1.utf8) else { throw MembershipPagingFailure.inconsistent }
                }
                rows.append(row)
            }
            guard UInt64(rows.count) <= incoming.count else { throw MembershipPagingFailure.inconsistent }
            if page.page.hasMore {
                guard !page.items.isEmpty, page.page.nextCursor != nil else { throw MembershipPagingFailure.inconsistent }
                return nil
            }
            guard UInt64(rows.count) == incoming.count else { throw MembershipPagingFailure.inconsistent }
            complete = rows; published = incoming; isStale = false; attempt = nil
            return complete
        } catch { invalidate(); throw error }
    }
}
