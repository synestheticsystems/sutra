import SutraCore
import SwiftUI

/// A right-click menu made of the core's lines, less any this Mac cannot carry out.
struct MenuEntries: View {
    let entries: [MenuEntry]
    let dashboard: DashboardModel

    var body: some View {
        ForEach(Array(entries.filter(\.isAvailable).enumerated()), id: \.offset) { _, entry in
            switch entry {
            case .item(let title, let checked, let action):
                if checked {
                    // A menu draws a check mark beside a toggle that is on.
                    Toggle(title, isOn: Binding(
                        get: { true },
                        set: { _ in dashboard.choose(action) }
                    ))
                } else {
                    Button(title) { dashboard.choose(action) }
                }
            case .separator:
                Divider()
            }
        }
    }
}

extension MenuEntry {
    /// Whether this Mac can carry the entry out: opening in Visual Studio Code needs it installed.
    @MainActor
    fileprivate var isAvailable: Bool {
        if case .item(_, _, .openInVisualStudioCode) = self {
            return Native.vsCode != nil
        }
        return true
    }
}
