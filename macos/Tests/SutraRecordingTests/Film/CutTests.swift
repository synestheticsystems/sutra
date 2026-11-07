@testable import SutraRecording
@testable import SutraWindow
import Testing

/// The preview's cut against the store's limits and the take's script: inside the store's length
/// and frame rate, every cue on a frame of its own, and each beat where the take plays it.
struct CutTests {
    @Test func theFilmFitsTheStoresLimitsForAPreview() {
        #expect(Cut.framesPerSecond <= 30)
        #expect((15...30).contains(Cut.length))
        #expect(Cut.moment(of: Cut.frames) == Cut.length)
    }

    @Test func everyCueFallsOnAFrameOfItsOwn() {
        for cue in [Cut.continues, Cut.folds, Cut.unfolds] {
            #expect(Cut.moment(of: Cut.frame(at: cue)) == cue)
        }
    }

    @Test func cedarWebFoldsAndOpensAgainWhileTheLayoutFixWaits() {
        let asked = Cut.continues + Scenario.permissionAsked
        let given = Cut.continues + Scenario.permissionGiven
        #expect(asked < Cut.folds && Cut.folds < Cut.unfolds && Cut.unfolds < given)
    }

    @Test func theFilmEndsWithTheSearchPageFinished() {
        #expect(Cut.continues + Scenario.searchFinished < Cut.length)
    }

    @Test func theLayoutFixsCardThrobsInFromRest() {
        #expect(Rhythm.throb.level(at: Cut.continues + Scenario.permissionAsked) < 1e-9)
    }
}
