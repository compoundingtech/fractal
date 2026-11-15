import XCTest

@testable import AttentionCore

final class InboxCapTests: XCTestCase {
  func testAvailableHeightSubtractsFixedContentAndMoreRowOnOverflow() {
    let cap = InboxCap(totalCount: 10, availableHeight: 580, fixedHeight: 200, rowHeight: 60, moreRowHeight: 40)
    XCTAssertEqual(cap.visibleCount, 5)
    XCTAssertEqual(cap.hiddenCount, 5)
    let taller = InboxCap(totalCount: 10, availableHeight: 660, fixedHeight: 200, rowHeight: 60, moreRowHeight: 40)
    XCTAssertEqual(taller.visibleCount, 7)
    XCTAssertEqual(taller.hiddenCount, 3)
  }

  func testAllRowsFitWithoutReservingAnUnneededMoreRow() {
    let exact = InboxCap(totalCount: 6, availableHeight: 560, fixedHeight: 200, rowHeight: 60, moreRowHeight: 40)
    XCTAssertEqual(exact.visibleCount, 6)
    XCTAssertEqual(exact.hiddenCount, 0)
    let extra = InboxCap(totalCount: 6, availableHeight: 580, fixedHeight: 200, rowHeight: 60, moreRowHeight: 40)
    XCTAssertEqual(extra.visibleCount, 6)
    XCTAssertEqual(extra.hiddenCount, 0)
  }

  func testOverflowAlwaysKeepsAtLeastThreeRowsEvenOnVeryShortScreens() {
    for available in [0.0, 200, 400] {
      let cap = InboxCap(totalCount: 8, availableHeight: available, fixedHeight: 200, rowHeight: 60, moreRowHeight: 40)
      XCTAssertEqual(cap.visibleCount, 3)
      XCTAssertEqual(cap.hiddenCount, 5)
    }
  }

  func testMinimumNeverInventsRowsAndEmptyInboxHasNoMoreRow() {
    for total in 0...3 {
      let cap = InboxCap(totalCount: total, availableHeight: 0, fixedHeight: 200, rowHeight: 60, moreRowHeight: 40)
      XCTAssertEqual(cap.visibleCount, total)
      XCTAssertEqual(cap.hiddenCount, 0)
    }
  }

  func testUnboundedSnapshotHeightShowsEveryRow() {
    let cap = InboxCap(totalCount: 100, availableHeight: .infinity, fixedHeight: 200, rowHeight: 60, moreRowHeight: 40)
    XCTAssertEqual(cap.visibleCount, 100)
    XCTAssertEqual(cap.hiddenCount, 0)
  }
}
