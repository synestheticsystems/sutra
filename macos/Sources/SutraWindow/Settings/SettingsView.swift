import SutraCore
import SwiftUI

/// The Settings window: how much free space Sutra keeps by cleaning by itself, and whether it looks
/// in the folders macOS guards. Each control sends the user's choice to the core and shows what the
/// core holds, so a choice the core asks about first shows only once the user agrees; the question
/// shows here, over this window.
public struct SettingsView: View {
    private let dashboard: DashboardModel

    @Environment(\.controlActiveState) private var activeState
    /// Flipped with each choice sent, which draws the controls again from what the core holds. A
    /// pop-up or a switch shows the user's choice as soon as it is made, and would go on showing it
    /// while the core holds another, as it does while it asks first.
    @State private var redrawn = false

    public init(dashboard: DashboardModel) {
        self.dashboard = dashboard
    }

    public var body: some View {
        let settings = dashboard.settings
        Form {
            Picker("Cleaning", selection: Binding(
                get: { settings.keepFree },
                set: { send(.keepFree(bytes: $0)) }
            )) {
                ForEach(settings.keepFreeChoices, id: \.bytes) { choice in
                    Text(choice.label).tag(choice.bytes)
                }
            }
            Toggle("Look in Desktop, Documents and Downloads too", isOn: Binding(
                get: { settings.includePrivateFolders },
                set: { send(.includePrivateFolders(include: $0)) }
            ))
        }
        .id(redrawn)
        .formStyle(.grouped)
        .frame(width: 440)
        .fixedSize()
        .dialogAlert(dashboard, in: .settings)
        .onChange(of: activeState, initial: true) {
            dashboard.settingsInFront = activeState == .key
        }
        .onDisappear { dashboard.settingsClosed() }
    }

    private func send(_ choice: Action) {
        redrawn.toggle()
        dashboard.perform(choice)
    }
}
