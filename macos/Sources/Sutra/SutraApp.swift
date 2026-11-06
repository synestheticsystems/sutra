import AppKit
import SutraCore
import SutraWindow
import SwiftUI

/// The app: one window of projects, and Settings. The core runs for as long as the app does.
@main
struct SutraApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate

    var body: some Scene {
        Window("Sutra", id: "dashboard") {
            DashboardView(dashboard: delegate.dashboard)
                .onChange(of: delegate.dashboard.dockBadge, initial: true) { _, badge in
                    NSApp.dockTile.badgeLabel = badge
                }
        }
        .defaultSize(width: 520, height: 640)
        .commands { DashboardCommands() }

        Settings {
            SettingsView(dashboard: delegate.dashboard)
        }
    }
}

/// Starts the core as the app launches and stops it as the app quits.
@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let dashboard: DashboardModel
    private let core: Dashboard

    override init() {
        let relay = WindowRelay(banners: NotificationBanners())
        core = Dashboard.start(window: relay)
        dashboard = DashboardModel(core: core)
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
