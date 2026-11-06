import AppKit
import SutraCore
import SutraWindow
import SwiftUI

/// The app: one window of projects, and Settings. The core runs for as long as the app does.
@main
struct SutraApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate

    var body: some Scene {
        SutraScenes(dashboard: delegate.dashboard, welcome: delegate.welcome)
    }
}

/// Starts the core as the app launches and stops it as the app quits, and holds what the window
/// shows: the core's dashboard, and the welcome screen in front of it.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let dashboard: DashboardModel
    let welcome = Welcome(memory: UserDefaults.standard)
    private let core: Dashboard

    override init() {
        let relay = WindowRelay(banners: NotificationBanners())
        core = Dashboard.start(window: relay)
        dashboard = DashboardModel(core: core, native: Native())
        relay.deliver(to: dashboard)
        super.init()
    }

    /// Sutra is its window: closing it quits, as there is nothing to do without it.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        core.stop()
    }
}
