import Foundation
@testable import SutraWindow
import Testing

/// The dots of a stretch of a dotted guide, which each nested card's row draws down its own
/// height, so that the stretches join into one even guide.
struct NestedCardRowTests {
    @Test(arguments: [84.0, 77.0, 69.5, 103.25])
    func aStretchsDotsStartAtItsTopAndStandEvenlyAboutFivePointsApart(height: Double) {
        let depths = DotsDownTheMiddle.depths(down: height)
        #expect(depths.first == 0)
        let gaps = zip(depths.dropFirst(), depths).map { $0 - $1 }
        let gap = height / Double(depths.count)
        #expect(gaps.allSatisfy { abs($0 - gap) < 1e-9 }, "even")
        #expect(abs(gap - 5) <= 2.5 / Double(depths.count), "about 5 pt")
    }

    @Test func theDotAtAStretchsFootIsLeftToTheStretchBelowItsOwnFirst() {
        let depths = DotsDownTheMiddle.depths(down: 80)
        #expect(depths.count == 16)
        #expect(abs(depths[15] - 75) < 1e-9, "one gap short of the foot")
    }
}
