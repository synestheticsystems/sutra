import SwiftUI

/// A button for each app that opens the folder: the editor chosen in Settings, when this Mac has
/// one, the terminal chosen there, and Finder.
struct OpenButtons: View {
    let path: String

    @Environment(\.native) private var native
    @AppStorage(Native.editorChoiceKey) private var chosenEditor = ""
    @AppStorage(Native.terminalChoiceKey) private var chosenTerminal = ""

    var body: some View {
        HStack(spacing: 4) {
            if let editor = native.editor(chosen: chosenEditor) {
                OpenButton(app: editor) { native.open(path, in: editor) }
            }
            if let terminal = native.terminal(chosen: chosenTerminal) {
                OpenButton(app: terminal) { native.open(path, in: terminal) }
            }
            if let finder = native.finder {
                OpenButton(app: finder) { native.openInFinder(path) }
            }
        }
    }
}

/// A button showing an app's icon, which opens the card's folder in that app.
private struct OpenButton: View {
    let app: Native.FolderApp
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Image(nsImage: app.icon)
                .resizable()
                .frame(width: 16, height: 16)
                // A target a little larger than the icon, within the line's own height.
                .frame(width: 20, height: 20)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .hoverTip("Open in \(app.name)")
        .accessibilityLabel("Open in \(app.name)")
    }
}
