import Foundation

/// Reserves a More row only when the inbox overflows. At least three asks remain
/// visible (when present), even when a short screen must scroll the glance panel.
public struct InboxCap: Sendable, Equatable {
  public let visibleCount: Int
  public let hiddenCount: Int

  public init(
    totalCount: Int, availableHeight: Double, fixedHeight: Double, rowHeight: Double,
    moreRowHeight: Double
  ) {
    precondition(totalCount >= 0 && rowHeight > 0 && moreRowHeight >= 0)
    let inboxHeight = availableHeight - fixedHeight
    if Double(totalCount) * rowHeight <= inboxHeight {
      visibleCount = totalCount
    } else {
      let fitting = floor((inboxHeight - moreRowHeight) / rowHeight)
      visibleCount = fitting >= Double(totalCount) ? totalCount : max(min(3, totalCount), Int(max(0, fitting)))
    }
    hiddenCount = totalCount - visibleCount
  }
}
