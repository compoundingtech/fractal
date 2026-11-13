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

  /// Shows a page (callers animate it); the previous receipt is cleared, since the person moved
  /// on. A card's page then needs `readOpenPage()`.
  public func open(_ next: PanelRoute) {
    route = next
    receipt = nil
    backFrom = nil
  }

  /// Back to the top level (Back, Esc), which then returns focus to what opened the page.
  public func back() {
    backFrom = route
    route = .home
  }

  /// Reads the open card's detail and its asker's conversation concurrently, each kept on
  /// failure, then any image attachment not read yet. Nothing for other pages.
  public func readOpenPage(now: Date? = nil) async {
    guard let adapter, case .attention(let id) = route, let record = record(id: id) else { return }
    async let context = Self.capture { try await adapter.readContext(record) }
    async let excerpt = Self.capture { try await adapter.readExcerpt(record) }
    let (contextResult, excerptResult) = await (context, excerpt)
    var contextSection = contexts[id] ?? LiveSection()
    Self.applyPage(contextResult, to: &contextSection, lastRead: &contextReadAt[id], now: now ?? .now)
    var excerptSection = excerpts[id] ?? LiveSection()
    Self.applyPage(excerptResult, to: &excerptSection, lastRead: &excerptReadAt[id], now: now ?? .now)
    // The card may have left while these were read.
    guard self.record(id: id) != nil else { return }
    // Fresh timestamps alone are not a content change. Keep the observable sections stable
    // on a successful identical read; failures and recovery still publish their state.
    if !Self.samePageSection(contexts[id], contextSection) { contexts[id] = contextSection }
    if !Self.samePageSection(excerpts[id], excerptSection) { excerpts[id] = excerptSection }
    await readAttachments(contextSection.value?.request?.attachments ?? [], from: adapter, now: now)
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
