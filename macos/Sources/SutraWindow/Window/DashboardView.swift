import AppKit
import SutraCore
import SwiftUI

/// The window's contents: the toolbar, then the scope bar, the banner, the line saying updates
/// stopped, the list and the footer, or the help panel in the list's place; and the core's dialogs
/// over them. On the first launch, and whenever the user asks for it again, the welcome screen
/// stands in front of all of it, the toolbar too, until the user continues.
public struct DashboardView: View {
    private let dashboard: DashboardModel
    private let welcome: Welcome

    @FocusState private var filterFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var seen = NSApp.occlusionState.contains(.visible)

    public init(dashboard: DashboardModel, welcome: Welcome) {
        self.dashboard = dashboard
        self.welcome = welcome
    }

    public var body: some View {
        VStack(spacing: 0) {
            if welcome.shown {
                IntroView { welcome.continueToDashboard() }
                    // The dance tells no news, so only Reduce Motion and a window nobody can see
                    // hold it still.
                    .environment(\.motion, reduceMotion || !seen ? .steady : .live)
            } else if dashboard.helpShown {
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
        .navigationTitle(welcome.shown ? "Welcome to Robo Sutra" : dashboard.title)
        .toolbar {
            if !welcome.shown {
                WindowToolbar(dashboard: dashboard)
            }
        }
        .dialogAlert(dashboard, in: .dashboard)
        // The help panel and the filter field wait behind the welcome screen, out of reach.
        .background { EscapeKey(close: { !welcome.shown && dashboard.escape() }) }
        .focusedSceneValue(\.showFilter, welcome.shown ? nil : ShowFilter {
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
