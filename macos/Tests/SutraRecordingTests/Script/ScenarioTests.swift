import Foundation
@testable import SutraRecording
import SutraCore
import Testing

/// The take as the scenario scripts it, read from the script at moments of the take.
struct ScenarioTests {
    private let layout = "/Demo/Projects/cedar-web-layout"
    private let search = "/Demo/Projects/cedar-web-search"

    private func contents(at moment: TimeInterval) -> WindowContents {
        Arrangement().contents(at: moment)
    }

    /// The layout fix's card at `moment`.
    private func layoutFix(at moment: TimeInterval) throws -> Card {
        try #require(card(at: layout, in: contents(at: moment)))
    }

    @Test func theLayoutFixStopsToAskPermissionAndWorksOnOnceItHasIt() throws {
        let asked = Scenario.permissionAsked
        #expect(try !layoutFix(at: asked - 1).waiting)
        #expect(try layoutFix(at: asked - 1).tray?.sessions.first?.look == .working)
        #expect(contents(at: asked - 1).title == "Sutra")

        #expect(try layoutFix(at: asked).waiting)
        let session = try #require(layoutFix(at: asked).tray?.sessions.first)
        #expect(session.look == .waiting)
        #expect(session.tooltip == """
            waiting for permission
            Fix the sidebar overflow
            Claude \u{b7} claude-sonnet-5-5
            """)
        #expect(contents(at: asked).title == "Sutra \u{b7} 1 waiting")
        #expect(contents(at: asked).dockBadge == "1")

        let given = Scenario.permissionGiven
        #expect(try !layoutFix(at: given).waiting)
        #expect(try layoutFix(at: given).tray?.sessions.first?.look == .working)
        #expect(contents(at: given).title == "Sutra")
        #expect(contents(at: given).dockBadge == nil)
    }

    @Test func theSearchPageFinishesWithItsSubagents() throws {
        let done = Scenario.searchFinished
        let atWork = try #require(card(at: search, in: contents(at: done - 1))?.tray)
        #expect(atWork.subagents.count == 2)
        #expect(atWork.sessions.first?.tooltip.hasPrefix("Busy, 2 subagents at work\n") == true)

        let finished = try #require(card(at: search, in: contents(at: done))?.tray)
        #expect(finished.subagents.isEmpty)
        #expect(finished.sessions.first?.look == .still)
        #expect(finished.sessions.first?.tooltip.hasPrefix("Idle\n") == true)
    }

    @Test func theTakeKeepsTheScenariosTimeAndNotTheMacs() throws {
        let lantern = "/Demo/Projects/lantern-docs"
        #expect(try #require(card(at: lantern, in: contents(at: 0))).age == "38m")
        #expect(try #require(card(at: lantern, in: contents(at: 120))).age == "40m")
    }

    @Test func everySessionIsInAPhaseFromTheMomentTheTakeOpens() {
        let sessions = Scenario.projects.flatMap(\.sessions)
        #expect(sessions.allSatisfy { $0.phases.first.map { $0.from < 0 } == true })
    }

    @Test func everyCueFallsOnAWholeSecondAsTheClockLooksOnlyThen() {
        let cues = [Scenario.permissionAsked, Scenario.permissionGiven, Scenario.searchFinished]
        #expect(cues.allSatisfy { $0 == $0.rounded() })
    }

    @Test func everyFolderOfTheInventedWorkIsUnderTheInventedHome() {
        let folders = Scenario.projects.flatMap { project in
            [project.path] + (project.worktree.map { [$0.commonDir] } ?? [])
        }
        #expect(folders.allSatisfy { $0.hasPrefix(Scenario.home + "/") })
    }
}
