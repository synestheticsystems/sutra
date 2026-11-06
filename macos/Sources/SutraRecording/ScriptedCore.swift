import Foundation
import os
import SutraCore

/// The core a recording's window reads in place of the live one. It plays the scenario's invented
/// work as the live core would show it, so the window's own views draw a busy machine that belongs
/// to no one. It changes the list as the live core would for what a recording demonstrates:
/// folding a repository, the sort, and the quiet projects' row. The rest the user asks of it, it
/// leaves undone.
///
/// It waits at the start of the script until `play`, and its clock runs from there, so every take
/// reads alike at each moment.
final class ScriptedCore: DashboardProtocol {
    private struct State {
        var arrangement = Arrangement()
        /// When the take began on the wall clock; `nil` while the script waits at its start.
        var playingSince: Date?
        /// What the window shows, as the script last worked it out.
        var shown: WindowContents
        /// Works the contents out again at each whole second of the take.
        var clock: Task<Void, Never>?

        /// The moment of the take it is at `now` on the wall clock.
        func moment(at now: Date) -> TimeInterval {
            playingSince.map { now.timeIntervalSince($0) } ?? 0
        }
    }

    private let state: OSAllocatedUnfairLock<State>
    /// Told each time what the window shows has changed, as the live core tells it.
    private let window: any DashboardWindow

    init(window: any DashboardWindow) {
        self.window = window
        state = OSAllocatedUnfairLock(initialState: State(shown: Arrangement().contents(at: 0)))
    }

    func contents() -> WindowContents {
        state.withLock { $0.shown }
    }

    func perform(action: Action) {
        let arranged = state.withLock { state in
            switch action {
            case .toggleRepositoryFold(let commonDir):
                state.arrangement.toggleFold(of: commonDir)
            case .setSort(let order):
                state.arrangement.sort = order
            case .toggleQuietProjects:
                state.arrangement.quietShown.toggle()
            // A recording demonstrates none of these. The invented work has nothing real to
            // clean, mute, stop or answer, and no folder to grant or watch, and the filter field
            // is left out of the take. No card moves while a take plays, so the pointer has
            // nothing to hold still.
            case .clean, .cleanIdle, .keepFree, .showFilter, .filterChanged, .hideFilter,
                 .pointerEnteredList, .pointerLeftList, .toggleGlobalMute,
                 .toggleGlobalNotifications, .toggleUnitMute, .toggleUnitNotifications,
                 .terminateEnvironment, .answer, .folderGranted, .watchPrivateFolder:
                return false
            }
            return true
        }
        if arranged {
            publish()
        }
    }

    /// Stop the clock, as the app quits.
    func stop() {
        state.withLock { state in
            state.clock?.cancel()
            state.clock = nil
        }
    }

    /// Play the script from its start, from now.
    func play() {
        state.withLock { state in
            state.clock?.cancel()
            state.playingSince = Date()
            state.clock = Task { [weak self] in
                while let wait = self?.untilNextSecond() {
                    do {
                        try await Task.sleep(for: .seconds(wait))
                    } catch {
                        return // Cancelled: rewound, or stopped.
                    }
                    self?.publish()
                }
            }
        }
        publish()
    }

    /// Back to the start of the script, the list as a take opens it, to wait for the next take.
    func rewind() {
        state.withLock { state in
            state.clock?.cancel()
            state.clock = nil
            state.playingSince = nil
            state.arrangement = Arrangement()
        }
        publish()
    }

    /// The moment of the take it is now: 0 while the script waits at its start.
    var moment: TimeInterval {
        state.withLock { $0.moment(at: Date()) }
    }

    /// How long until the take's next whole second, when the clock looks again.
    private func untilNextSecond() -> TimeInterval {
        let moment = moment
        return moment.rounded(.down) + 1 - moment
    }

    /// Work out what the window shows now, and tell the window when that differs from what it
    /// shows, as the live core publishes its contents.
    private func publish() {
        let changed = state.withLock { state in
            let contents = state.arrangement.contents(at: state.moment(at: Date()))
            guard contents != state.shown else { return false }
            state.shown = contents
            return true
        }
        if changed {
            window.changed()
        }
    }
}
