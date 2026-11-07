import AppKit
@testable import SutraWindow
import Testing

/// The level a swell's layer holds under each motion.
struct SwellTests {
    /// A throb from 0.2 up to 0.6 and back, over a cycle of 1.6 seconds.
    private let swell = Swell(rhythm: .throb, color: .systemOrange, rest: 0.2, peak: 0.6)

    @Test func theLayerHoldsTheSteadyLevelWhetherOrNotTheRhythmRunsOverIt() {
        #expect(swell.level(under: .live) == swell.steady)
        #expect(swell.level(under: .steady) == swell.steady)
    }

    @Test func aRecordingsMomentHoldsTheLevelOfThatPointInTheCycle() {
        #expect(abs(swell.level(under: .frozen(at: 0)) - 0.2) < 1e-9)
        #expect(abs(swell.level(under: .frozen(at: 0.4)) - 0.4) < 1e-9)
        #expect(abs(swell.level(under: .frozen(at: 0.8)) - 0.6) < 1e-9)
        #expect(abs(swell.level(under: .frozen(at: 2.0)) - 0.4) < 1e-9)
    }
}
