import AppKit
import SwiftUI

/// The app's scenes: the window of projects, with its menu commands and its Dock badge, and
/// Settings. The app and its recording build share them, so a recording shows the window the app
/// ships.
public struct SutraScenes: Scene {
    private let dashboard: DashboardModel
    private let welcome: Welcome

    public init(dashboard: DashboardModel, welcome: Welcome) {
        self.dashboard = dashboard
        self.welcome = welcome
    }

    public var body: some Scene {
        Window("Sutra", id: "dashboard") {
            DashboardView(dashboard: dashboard, welcome: welcome)
                .onChange(of: dashboard.dockBadge, initial: true) { _, badge in
                    NSApp.dockTile.badgeLabel = badge
                }
        }
        .defaultSize(width: 520, height: 640)
        .commands { DashboardCommands(welcome: welcome) }

        Settings {
            SettingsView(dashboard: dashboard)
        }
    }
}
