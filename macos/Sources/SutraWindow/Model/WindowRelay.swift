import os
import SutraCore

/// The window as the core sees it. The core calls in on its own thread, as often as several times
/// a second; the relay turns that into at most one pending refresh on the main actor, which reads
/// the contents as they are by the time it runs, and posts the core's banners.
public final class WindowRelay: DashboardWindow {
    private struct State: Sendable {
        /// Set once the model exists, which needs the core, which needs this relay first. Weak,
        /// because the running core holds this relay and the model holds the core. A strong
        /// reference would keep all three alive, so a core the app let go of without `stop()`
        /// would never be released, and its threads would run on.
        weak var model: DashboardModel?
        var refreshPending = false
    }

    private let state = OSAllocatedUnfairLock(initialState: State())
    private let banners: NotificationBanners

    public init(banners: NotificationBanners) {
        self.banners = banners
    }

    /// Send the core's changes to `model` from now on, for as long as the app keeps it: the relay
    /// does not. The model read the contents when it was made, so nothing the core said before
    /// this is lost.
    @MainActor
    public func deliver(to model: DashboardModel) {
        state.withLock { $0.model = model }
    }

    public func changed() {
        let first = state.withLock { state in
            defer { state.refreshPending = true }
            return !state.refreshPending
        }
        guard first else { return }
        Task { @MainActor in
            // Cleared before reading, so a change the core makes during the read asks again.
            let model = self.state.withLock { state in
                state.refreshPending = false
                return state.model
            }
            model?.refresh()
        }
    }

    public func notify(title: String, body: String) {
        Task { @MainActor in
            self.banners.post(title: title, body: body)
        }
    }
}
