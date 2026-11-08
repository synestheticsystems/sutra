import SutraCore
import SwiftUI

/// The Settings window. The window keeps three settings itself: whether Sutra opens at login, which
/// macOS keeps for it, and the editor and the terminal a card's folder opens in. The core keeps the
/// rest: how much free space Sutra keeps by cleaning by itself, and which of the folders macOS
/// guards it watches. Each of the core's controls sends the user's choice to the core and shows
/// what the core holds, so a choice the core asks about first shows only once the user agrees; the
/// question shows here, over this window.
public struct SettingsView: View {
    private let dashboard: DashboardModel

    @Environment(\.controlActiveState) private var activeState
    @AppStorage(Native.editorChoiceKey) private var chosenEditor = ""
    @AppStorage(Native.terminalChoiceKey) private var chosenTerminal = ""
    @State private var opensAtLogin: Bool
    /// Flipped with each choice sent, which draws the controls again from what the core holds. A
    /// pop-up or a switch shows the user's choice as soon as it is made, and would go on showing it
    /// while the core holds another, as it does while it asks first.
    @State private var redrawn = false

    public init(dashboard: DashboardModel) {
        self.dashboard = dashboard
        _opensAtLogin = State(initialValue: dashboard.native.opensAtLogin)
    }

    public var body: some View {
        let settings = dashboard.settings
        Form {
            Toggle("Open at login", isOn: Binding(
                get: { opensAtLogin },
                set: { opensAtLogin = dashboard.native.setOpensAtLogin($0) }
            ))
            if !dashboard.native.editors.isEmpty {
                Picker("Open folders in", selection: Binding(
                    get: { dashboard.native.editor(chosen: chosenEditor)?.bundleIdentifier ?? "" },
                    set: { chosenEditor = $0 }
                )) {
                    ForEach(dashboard.native.editors, id: \.bundleIdentifier) { editor in
                        Text(editor.name).tag(editor.bundleIdentifier)
                    }
                }
            }
            if !dashboard.native.terminals.isEmpty {
                Picker("Terminal", selection: Binding(
                    get: { dashboard.native.terminal(chosen: chosenTerminal)?.bundleIdentifier ?? "" },
                    set: { chosenTerminal = $0 }
                )) {
                    ForEach(dashboard.native.terminals, id: \.bundleIdentifier) { terminal in
                        Text(terminal.name).tag(terminal.bundleIdentifier)
                    }
                }
            }
            Picker("Cleaning", selection: Binding(
                get: { settings.keepFree },
                set: { send(.keepFree(bytes: $0)) }
            )) {
                ForEach(settings.keepFreeChoices, id: \.bytes) { choice in
                    Text(choice.label).tag(choice.bytes)
                }
            }
            Section("Also watch") {
                ForEach(settings.privateFolders, id: \.folder) { privateFolder in
                    Toggle(privateFolder.folder, isOn: Binding(
                        get: { privateFolder.watched },
                        set: { send(.watchPrivateFolder(folder: privateFolder.folder, watch: $0)) }
                    ))
                }
            }
        }
        .id(redrawn)
        .formStyle(.grouped)
        .frame(width: 440)
        .fixedSize()
        .dialogAlert(dashboard, in: .settings)
        .onChange(of: activeState, initial: true) {
            dashboard.settingsInFront = activeState == .key
            // The user may have changed it in System Settings meanwhile.
            opensAtLogin = dashboard.native.opensAtLogin
        }
        .onDisappear { dashboard.settingsClosed() }
    }

    private func send(_ choice: Action) {
        redrawn.toggle()
        dashboard.perform(choice)
    }
}
