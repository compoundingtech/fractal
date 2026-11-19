import Foundation

/// What the last answer or control committed: the card it was for and what was done.
public struct Receipt: Sendable, Equatable {
  public let title: String
  public let outcome: String

  public init(title: String, outcome: String) {
    self.title = title
    self.outcome = outcome
  }
}

extension AttentionModel {
  /// Attachments larger than this are left to Fractal web.
  public static let attachmentByteLimit = 8 << 20

  /// Attention is a selection beside the current page, not another navigation level.
  /// Selecting another card replaces it; opening a non-attention page clears it. Other
  /// pages retain their existing stack identity and Back behavior.
  public func open(_ next: PanelRoute) {
    guard next != route else { return }
    if case .attention = route { navigationEntries.removeLast() }
    if let live = navigationEntries.lastIndex(where: { $0.route == next }) {
      navigationEntries.removeSubrange((live + 1)...)
    } else {
      navigationEntries.append(PanelNavigationEntry(next))
    }
    receipt = nil
    backFrom = nil
  }

  /// Back to the page that opened this one (Back, Esc), which returns focus to what opened it.
  /// Nothing at the top level.
  public func back() {
    guard navigationEntries.count > 1 else { return }
    backFrom = navigationEntries.removeLast().route
  }

  /// Synchronous content for every layout. An older revision must never overwrite the list's
  /// current question, reasons or recommendation while enrichment is being refreshed.
  public func detail(for record: AttentionRecord) -> AttentionContext {
    if detailRevisions[record.id] == record.revision, let cached = contexts[record.id]?.value { return cached }
    return record.snapshotContext ?? AttentionContext(
      detail: record.detail.isEmpty ? nil : record.detail, request: nil)
  }

  public static let detailCapacity = 12
  public static let detailWarmCount = 5
  public static let detailFreshness: TimeInterval = 15

  /// Coalesced across opening, polling and warming. Never clears cached content on a reread.
  public func readOpenPage(now: Date? = nil) async {
    guard case .attention(let id) = route, let record = record(id: id) else { return }
    await readDetail(record, now: now)
  }

  /// Warm likely asks first, then visible updates. Serial prefetch bounds background I/O to
  /// one context + one timeline; an explicit open can proceed without waiting for the queue.
  public func warmDetails(now: Date? = nil) async {
    guard isPanelVisible else { return }
    guard !isWarmingDetails else { return }
    isWarmingDetails = true
    defer { isWarmingDetails = false }
    let likely = asks + updates.filter { rowsInPanel[$0.id] != nil }
    for record in likely.prefix(Self.detailWarmCount) {
      guard isPanelVisible, !Task.isCancelled else { return }
      if route == .attention(record.id) { continue }
      await readDetail(record, now: now, images: false)
    }
  }

  func keepDetails(_ page: AttentionPage) {
    let revisions = Dictionary(uniqueKeysWithValues: page.records.map { ($0.id, $0.revision) })
    for (id, read) in detailReads where revisions[id] != read.revision {
      read.task.cancel()
      detailReads[id] = nil
    }
    detailAccess.removeAll { revisions[$0] == nil }
    detailRevisions = detailRevisions.filter { revisions[$0.key] == $0.value }
  }

  private func readDetail(_ record: AttentionRecord, now: Date?, images: Bool = true) async {
    guard let adapter else { return }
    let id = record.id
    detailAccess.removeAll { $0 == id }
    detailAccess.append(id)
    // Admission is bounded before suspension. A late read for an evicted entry must not
    // recreate untracked cache state after another card has taken its place.
    trimDetails()
    if let running = detailReads[id], running.revision == record.revision {
      await running.task.value
    } else {
      let clock = now ?? .now
      let fresh = detailRevisions[id] == record.revision
        && contextReadAt[id].map { (0..<Self.detailFreshness).contains(clock.timeIntervalSince($0)) } == true
        && excerptReadAt[id].map { (0..<Self.detailFreshness).contains(clock.timeIntervalSince($0)) } == true
      if !fresh {
        let task = Task { [weak self] in
          guard let self else { return }
          await withTaskGroup(of: Void.self) { group in
            group.addTask {
              let result = await Self.capture { try await adapter.readContext(record) }
              await self.applyContext(result, record: record, now: now ?? .now)
            }
            group.addTask {
              let result = await Self.capture { try await adapter.readExcerpt(record) }
              await self.applyExcerpt(result, record: record, now: now ?? .now)
            }
          }
        }
        detailReads[id] = (record.revision, task)
        await task.value
        if detailReads[id]?.revision == record.revision { detailReads[id] = nil }
      }
    }
    guard self.record(id: id)?.revision == record.revision, detailAccess.contains(id) else { return }
    if images { await readAttachments(detail(for: record).request?.attachments ?? [], from: adapter, now: now) }
    trimDetails()
  }

  private func applyContext(_ result: Result<AttentionContext, Error>, record: AttentionRecord, now: Date) {
    guard self.record(id: record.id)?.revision == record.revision, detailAccess.contains(record.id), !Task.isCancelled else { return }
    var section = contexts[record.id] ?? LiveSection()
    Self.applyPage(result, to: &section, lastRead: &contextReadAt[record.id], now: now)
    if case .success = result { detailRevisions[record.id] = record.revision }
    if !Self.samePageSection(contexts[record.id], section) { contexts[record.id] = section }
  }

  private func applyExcerpt(_ result: Result<ConversationExcerpt?, Error>, record: AttentionRecord, now: Date) {
    guard self.record(id: record.id)?.revision == record.revision, detailAccess.contains(record.id), !Task.isCancelled else { return }
    var section = excerpts[record.id] ?? LiveSection()
    Self.applyPage(result, to: &section, lastRead: &excerptReadAt[record.id], now: now)
    if !Self.samePageSection(excerpts[record.id], section) { excerpts[record.id] = section }
  }

  private func trimDetails() {
    while detailAccess.count > Self.detailCapacity {
      let id = detailAccess.removeFirst()
      if route == .attention(id) { detailAccess.append(id); continue }
      contexts[id] = nil
      excerpts[id] = nil
      contextReadAt[id] = nil
      excerptReadAt[id] = nil
      detailRevisions[id] = nil
    }
    let named = Set(contexts.values.flatMap { $0.value?.request?.attachments.map(\.sha256) ?? [] })
    attachments = attachments.filter { named.contains($0.key) }
  }

  private func readAttachments(_ refs: [AttachmentRef], from adapter: any AttentionAdapter, now: Date?) async {
    let wanted = refs.filter { $0.isImage && $0.size <= Self.attachmentByteLimit && attachments[$0.sha256]?.value == nil }
    await withTaskGroup(of: (String, Result<Data, Error>).self) { group in
      for ref in wanted {
        group.addTask { (ref.sha256, await Self.capture { try await adapter.readAttachment(ref) }) }
      }
      for await (sha, result) in group {
        var section = attachments[sha] ?? LiveSection()
        Self.apply(result, to: &section, now: now ?? .now)
        if attachments[sha] != section { attachments[sha] = section }
      }
    }
  }

  private static func applyPage<T: Sendable & Equatable>(
    _ result: Result<T, Error>, to section: inout LiveSection<T>, lastRead: inout Date?, now: Date
  ) {
    // Retain the true most-recent successful read for a later stale error, even when the
    // observable fresh section intentionally retains its old timestamp.
    if case .failure = result, let lastRead { section.state = .fresh(lastRead) }
    Self.apply(result, to: &section, now: now)
    if case .success = result { lastRead = now }
  }

  private static func samePageSection<T: Sendable & Equatable>(_ old: LiveSection<T>?, _ new: LiveSection<T>) -> Bool {
    guard let old, old.value == new.value else { return false }
    if case .fresh = old.state, case .fresh = new.state { return true }
    return old == new
  }
}
