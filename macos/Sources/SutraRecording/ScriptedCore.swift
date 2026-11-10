import Foundation
import os
import SutraCore

/// The core a recording's window reads in place of the live one. It plays the scenario's invented
/// work as the live core would show it, so the window's own views draw a busy machine that belongs
/// to no one. It changes the list as the live core would for what a recording demonstrates:
/// folding a line, the sort, and the quiet projects' row. The rest the user asks of it, it
/// leaves undone.
///
/// It waits at the start of the script until `play`, and its clock runs from there, so every take
/// reads alike at each moment. A film that draws the take frame by frame holds it at each frame's
/// moment instead.
final class ScriptedCore: DashboardProtocol {
    private struct State {
        var arrangement = Arrangement()
        var clock = Clock.waiting
        /// What the window shows, as the script last worked it out.
        var shown: WindowContents
    }

    /// Where the take's clock stands.
    private enum Clock {
        /// At the start of the script, waiting for the take to play.
        case waiting
        /// Playing on the wall clock since `since`, with `ticks` working the contents out again at
        /// each whole second of the take.
        case playing(since: Date, ticks: Task<Void, Never>)
        /// Held at a moment of the take by a film, which moves the clock itself, frame by frame.
        case held(at: TimeInterval)

        /// The moment of the take it is at `now` on the wall clock.
        func moment(at now: Date) -> TimeInterval {
            switch self {
            case .waiting: 0
            case .playing(let since, _): now.timeIntervalSince(since)
            case .held(let moment): moment
            }
        }

        /// No more ticks, if it plays.
        func stopTicks() {
            if case .playing(_, let ticks) = self {
                ticks.cancel()
            }
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
            case .toggleFold(let line):
                state.arrangement.toggleFold(of: line)
            case .setSort(let order):
                state.arrangement.sort = order
            case .toggleQuietProjects:
                state.arrangement.quietShown.toggle()
            // A recording demonstrates none of these. The invented work has nothing real to
            // clean, mute, stop or answer, and no folder to grant or watch, and the filter field
            // is left out of the take. No card moves while a take plays, so the pointer has
            // nothing to hold still.
            case .clean, .cleanBuildFolders, .cleanOffered, .keepFree, .cleanEveryGroupWhenIdle,
                 .cleanWhenIdle, .showFilter,
                 .filterChanged, .hideFilter, .pointerEnteredList, .pointerLeftList,
                 .toggleGlobalMute, .toggleGlobalNotifications, .toggleUnitMute,
                 .toggleUnitNotifications, .terminateEnvironment, .answer, .folderGranted,
                 .watchPrivateFolder:
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
        state.withLock { $0.clock.stopTicks() }
    }

    /// Play the script from its start, from now.
    func play() {
        state.withLock { state in
            state.clock.stopTicks()
            let ticks = Task { [weak self] in
                while let wait = self?.untilNextSecond() {
                    do {
                        try await Task.sleep(for: .seconds(wait))
                    } catch {
                        return // Cancelled: rewound, held, or stopped.
                    }
                    self?.publish()
                }
            }
            state.clock = .playing(since: Date(), ticks: ticks)
        }
        publish()
    }

    /// Back to the start of the script, the list as a take opens it, to wait for the next take.
    func rewind() {
        state.withLock { state in
            state.clock.stopTicks()
            state.clock = .waiting
            state.arrangement = Arrangement()
        }
        publish()
    }

    /// Show the take at `moment` and hold it there, out of the wall clock's reach. A film that
    /// draws the take frame by frame moves the clock itself, a frame at a time, so every film of
    /// the take draws the same frames.
    func hold(at moment: TimeInterval) {
        state.withLock { state in
            state.clock.stopTicks()
            state.clock = .held(at: moment)
        }
        publish()
    }

    /// The moment of the take it is now: 0 while the script waits at its start, and the moment a
    /// film holds it at.
    var moment: TimeInterval {
        state.withLock { $0.clock.moment(at: Date()) }
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
            let contents = state.arrangement.contents(at: state.clock.moment(at: Date()))
            guard contents != state.shown else { return false }
            state.shown = contents
            return true
        }
        if changed {
            window.changed()
        }
    }
}
