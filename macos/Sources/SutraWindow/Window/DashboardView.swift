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
    /// How the window's host asks it to move: live in the app; frozen at each moment in turn while
    /// a recording draws the window frame by frame.
    @Environment(\.motion) private var hostMotion

    public init(dashboard: DashboardModel, welcome: Welcome) {
        self.dashboard = dashboard
        self.welcome = welcome
    }

    public var body: some View {
        VStack(spacing: 0) {
            if welcome.shown {
                IntroView(danceSeed: welcome.danceSeed) { welcome.continueToDashboard() }
                    // The dance tells no news, so only Reduce Motion and a window nobody can see
                    // hold it still.
                    .environment(\.motion, motion(heldStill: reduceMotion || !seen))
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
        // Nothing moves once the map no longer updates, when a pulse would claim news Sutra no
        // longer has, nor while macOS is set to reduce motion or nobody can see the window.
        .environment(
            \.motion, dashboard.stopped != nil ? .steady : motion(heldStill: reduceMotion || !seen)
        )
        // The views open, copy and reveal through the services the model holds, links in text
        // too.
        .environment(\.native, dashboard.native)
        .environment(\.openURL, OpenURLAction { url in
            dashboard.native.open(link: url.absoluteString)
            return .handled
        })
        .onReceive(NotificationCenter.default.publisher(
            for: NSApplication.didChangeOcclusionStateNotification
        )) { _ in
            seen = NSApp.occlusionState.contains(.visible)
        }
    }

    /// The window's motion when it holds still or moves. A recording that draws the window frame
    /// by frame freezes each moment itself, whatever this Mac is set to and though nobody sees the
    /// window here, as its viewers watch it elsewhere.
    private func motion(heldStill: Bool) -> Motion {
        if hostMotion.frozenMoment != nil {
            hostMotion
        } else if heldStill {
            .steady
        } else {
            .live
        }
    }
}
