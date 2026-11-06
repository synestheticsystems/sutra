import SutraCore
import SwiftUI

/// The window's toolbar, which holds what acts on Sutra as a whole: the clean of idle build
/// folders, the help panel, the sound and notification switches, and the store edition's folder.
/// What acts on the list sits over the list, in the scope bar.
struct WindowToolbar: ToolbarContent {
    let dashboard: DashboardModel

    var body: some ToolbarContent {
        ToolbarItemGroup(placement: .primaryAction) {
            CleanButton(dashboard: dashboard)
            HelpToggle(dashboard: dashboard)
            IconButton(
                symbol: dashboard.toolbar.muted ? "speaker.slash" : "speaker.wave.2",
                tooltip: dashboard.toolbar.muteTooltip
            ) {
                dashboard.perform(.toggleGlobalMute)
            }
            IconButton(
                symbol: dashboard.toolbar.notificationsOff ? "bell.slash" : "bell",
                tooltip: dashboard.toolbar.notificationsTooltip
            ) {
                dashboard.perform(.toggleGlobalNotifications)
            }
            if let folder = dashboard.toolbar.folder {
                IconButton(symbol: "folder", tooltip: folder) {
                    dashboard.chooseFolder()
                }
            }
        }
    }
}

/// The clean of every idle build folder. A click asks the core to clean them, while it offers
/// anything to clean; the icon fades while it offers nothing, and takes the accent and pulses while
/// the core works out what a click would clean. A right click sets how much space Sutra keeps free,
/// offered or not.
private struct CleanButton: View {
    let dashboard: DashboardModel

    var body: some View {
        let clean = dashboard.toolbar.clean
        Button {
            if clean.offered {
                dashboard.perform(.cleanIdle)
            }
        } label: {
            // At rest the icon keeps the toolbar's own colour, as its neighbours do.
            if clean.busy {
                Image(systemName: "trash")
                    .foregroundStyle(.tint)
                    .symbolEffect(.pulse)
            } else {
                Image(systemName: "trash")
                    .opacity(clean.offered ? 1 : 0.4)
            }
        }
        .help(clean.tooltip)
        .accessibilityLabel(clean.tooltip)
        .contextMenu { MenuEntries(entries: clean.menu, dashboard: dashboard) }
    }
}

/// Opens the help panel in the list's place, and closes it; drawn pressed while it is open.
private struct HelpToggle: View {
    let dashboard: DashboardModel

    var body: some View {
        let tooltip = dashboard.helpShown ? "Hide integration help" : "Integration help"
        Toggle(isOn: Binding(
            get: { dashboard.helpShown },
            set: { _ in dashboard.toggleHelp() }
        )) {
            Image(systemName: "questionmark.circle")
        }
        .toggleStyle(.button)
        .help(tooltip)
        .accessibilityLabel(tooltip)
    }
}

/// A toolbar icon that does one thing, with what it does in its tooltip.
private struct IconButton: View {
    let symbol: String
    let tooltip: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(systemName: symbol)
        }
        .help(tooltip)
        .accessibilityLabel(tooltip)
    }
}
