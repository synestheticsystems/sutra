import Foundation
@testable import SutraWindow
import Testing

/// Where a rhythm stands at each moment of its cycle, which a recording that draws the window frame
/// by frame shows as the live animation would.
struct RhythmTests {
    @Test(arguments: [Rhythm.throb, .pulse])
    func eachCycleRisesFromRestToThePeakHalfwayAndFallsBack(rhythm: Rhythm) {
        let period = rhythm.period
        #expect(abs(rhythm.level(at: 0)) < 1e-9)
        #expect(abs(rhythm.level(at: period / 2) - 1) < 1e-9)
        #expect(abs(rhythm.level(at: period)) < 1e-9)
    }

    @Test(arguments: [Rhythm.throb, .pulse])
    func theFallMirrorsTheRiseAndEveryCycleRepeatsTheFirst(rhythm: Rhythm) {
        let period = rhythm.period
        for tenth in 1..<10 {
            let moment = period / 2 * Double(tenth) / 10
            #expect(abs(rhythm.level(at: period - moment) - rhythm.level(at: moment)) < 1e-9)
            #expect(abs(rhythm.level(at: moment + 7 * period) - rhythm.level(at: moment)) < 1e-9)
        }
    }

    @Test(arguments: [Rhythm.throb, .pulse])
    func theRiseEasesInAndOutAsHalfACosineDoes(rhythm: Rhythm) {
        for twentieth in 0...20 {
            let along = Double(twentieth) / 20
            let cosine = (1 - cos(.pi * along)) / 2
            #expect(abs(rhythm.level(at: along * rhythm.period / 2) - cosine) < 0.005)
        }
    }
}
