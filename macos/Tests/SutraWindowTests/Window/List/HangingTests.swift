import Foundation
@testable import SutraWindow
import Testing

/// Where a hanging row's guides run: each down the middle of the chevron of the line it hangs from,
/// in the room the rows under that line leave by standing a step further in.
@MainActor
struct HangingTests {
    @Test(arguments: [0, 1, 2, 3])
    func aLinesGuideRunsDownItsChevronBetweenWhereTheLineStartsAndWhereItsRowsStart(depth: Int) {
        let line = CGFloat(depth) * Hanging.step
        let rows = CGFloat(depth + 1) * Hanging.step
        let chevron = line + FoldingLine.inset
        #expect(Hanging.guide(at: depth) == chevron + FoldingLine.chevronWidth / 2)
        #expect(line < Hanging.guide(at: depth) && Hanging.guide(at: depth) < rows)
    }
}
