import os
@testable import SutraRecording
import SutraCore
import Testing

/// What the user can do to the scripted list, at the start of the script: what a recording
/// demonstrates changes the list as the live core would, and the rest changes nothing. And the
/// take a film holds at a moment of its own.
struct ScriptedCoreTests {
    /// A window that counts how often it is told its contents changed.
    private final class CountingWindow: DashboardWindow {
        private let told = OSAllocatedUnfairLock(initialState: 0)

        var changes: Int {
            told.withLock { $0 }
        }

        func changed() {
            told.withLock { $0 += 1 }
        }

        func notify(title: String, body: String) {}
    }

    private let window = CountingWindow()
    private let core: ScriptedCore
    init() {
        core = ScriptedCore(window: window)
    }

    @Test func aTakeOpensOnTheActivitySortWithCedarWebsLineOpenOverItsWorktrees() {
        #expect(shown(core.contents()) == takeOpening)
        #expect(core.contents().toolbar.sort?.current == .activity)
    }

    @Test func foldingCedarWebsLinePutsItsAgentsOnItAndOpeningItBringsItsCardsBack() {
        core.perform(action: .toggleFold(line: Scenario.cedarWebLine))
        let folded = ["cedar-web: folded", "harbor-api: folded", "lantern-docs: folded", "quiet:3"]
        #expect(shown(core.contents()) == folded)
        guard case .rows(let rows) = core.contents().list,
              case .line(let line, _) = rows[0],
              case .folded(let sessions, let subagents, let waiting, let lozenges, let more)
                = line.fold
        else {
            Issue.record("Cedar Web's line comes first, folded")
            return
        }
        #expect(sessions.count == 3 && subagents.count == 2 && !waiting)
        // The site its main worktree serves stays in sight on the line.
        #expect(lozenges.map(\.lozenge.name) == ["web"] && more == nil)
        core.perform(action: .toggleFold(line: Scenario.cedarWebLine))
        #expect(shown(core.contents()) == takeOpening)
        #expect(window.changes == 2)
    }

    @Test func theNameAndDiskSortsOrderTheGroupsAndTheCardsUnderEachLineTheirWay() {
        core.perform(action: .setSort(order: .name))
        #expect(shown(core.contents()) == takeOpening, "already in name order, main first")
        core.perform(action: .setSort(order: .disk))
        #expect(shown(core.contents()) == [
            "harbor-api: folded", "cedar-web: open", "main", "feature/search", "fix/layout",
            "lantern-docs: folded", "quiet:3",
        ])
        #expect(core.contents().toolbar.sort?.current == .disk)
        #expect(window.changes == 2)
    }

    @Test func theQuietRowOpensToListTheQuietGroupsBelowItAndCloses() {
        core.perform(action: .toggleQuietProjects)
        let quiet = ["atlas-notes: folded", "orchard-kit: folded", "Other: folded"]
        #expect(shown(core.contents()) == takeOpening + quiet)
        core.perform(action: .toggleQuietProjects)
        #expect(shown(core.contents()) == takeOpening)
    }

    @Test func whatTheScriptLeavesUndoneChangesNothing() {
        let before = core.contents()
        let undone: [Action] = [
            .clean(folder: "/Demo/Projects/harbor-api/target"), .cleanIdle,
            .keepFree(bytes: 25_000_000_000), .showFilter, .filterChanged(text: "cedar"),
            .hideFilter, .pointerEnteredList, .pointerLeftList, .toggleGlobalMute,
            .toggleGlobalNotifications, .toggleUnitMute(environment: "web", unit: "web"),
            .toggleUnitNotifications(environment: "web", unit: "web"),
            .terminateEnvironment(pid: 4242), .answer(dialog: 1, yes: true),
            .folderGranted(folder: "/Demo"), .watchPrivateFolder(folder: "Desktop", watch: true),
        ]
        for action in undone {
            core.perform(action: action)
        }
        #expect(core.contents() == before)
        #expect(window.changes == 0)
    }

    @Test func rewindingReturnsTheListToHowATakeOpensIt() {
        core.perform(action: .toggleFold(line: Scenario.cedarWebLine))
        core.perform(action: .setSort(order: .name))
        core.perform(action: .toggleQuietProjects)
        core.rewind()
        #expect(core.contents() == ScriptedCore(window: CountingWindow()).contents())
    }

    @Test func aHeldTakeShowsItsMomentAndTheWallClockMovesItNoFurther() async throws {
        core.play()
        core.hold(at: Scenario.permissionAsked)
        #expect(core.moment == Scenario.permissionAsked)
        #expect(core.contents() == Arrangement().contents(at: Scenario.permissionAsked))
        // Past the next whole second of the wall clock, when a playing take would look again.
        try await Task.sleep(for: .milliseconds(1_100))
        #expect(core.moment == Scenario.permissionAsked)
        #expect(window.changes == 1)
    }
}
