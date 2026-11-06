import SutraCore
import SwiftUI

/// A right-click menu made of the core's lines. A card's menu starts with opening its folder in the
/// editor chosen in Settings, which the window offers itself, since it knows which editors this
/// Mac has.
struct MenuEntries: View {
    let entries: [MenuEntry]
    let dashboard: DashboardModel
    /// The folder a card's menu opens in the editor; `nil` for a menu of no folder.
    var folder: String?

    @AppStorage(Native.editorChoiceKey) private var chosenEditor = ""

    var body: some View {
        if let folder, let editor = Native.editor(chosen: chosenEditor) {
            Button("Open in \(editor.name)") { Native.open(folder, in: editor) }
        }
        ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
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
