import AppKit
import SutraCore
import SwiftUI

/// The window's contents: the toolbar, then the scope bar, the banner, the line saying updates
/// stopped, the list and the footer, or the help panel in the list's place; and the core's dialogs
/// over them.
public struct DashboardView: View {
    private let dashboard: DashboardModel

    @FocusState private var filterFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var seen = NSApp.occlusionState.contains(.visible)

    public init(dashboard: DashboardModel) {
        self.dashboard = dashboard
    }

    public var body: some View {
        VStack(spacing: 0) {
            if dashboard.helpShown {
                HelpPanel()
            } else {
                ScopeBar(dashboard: dashboard, filterFocus: $filterFocused)
                BannerView(dashboard: dashboard)
                StoppedLine(dashboard: dashboard)
                ListView(dashboard: dashboard)
                FooterView(dashboard: dashboard)
            }
        }
        // Narrower, and the toolbar's icons start to spill into its overflow menu.
        .frame(minWidth: 460, minHeight: 300)
        .navigationTitle(dashboard.title)
        .toolbar { WindowToolbar(dashboard: dashboard) }
        .dialogAlert(dashboard, in: .dashboard)
        .background { EscapeKey(close: dashboard.escape) }
        .focusedSceneValue(\.showFilter, ShowFilter {
            dashboard.showFilter()
            filterFocused = true
        })
        // Nothing moves while macOS is set to reduce motion, while nobody can see the window, or
        // once the map no longer updates, when a pulse would claim news Sutra no longer has.
        .environment(\.motion, reduceMotion || !seen || dashboard.stopped != nil ? .steady : .live)
        .onReceive(NotificationCenter.default.publisher(
            for: NSApplication.didChangeOcclusionStateNotification
        )) { _ in
            seen = NSApp.occlusionState.contains(.visible)
        }
    }
}
