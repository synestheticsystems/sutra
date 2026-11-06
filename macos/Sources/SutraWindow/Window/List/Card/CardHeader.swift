import SutraCore
import SwiftUI

/// A card's identity: the name, the branch chip, and the age at the right, whose place the buttons
/// that open the folder take while the pointer is over the card.
struct CardHeader: View {
    let card: Card
    let hovered: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            // The branch moves under the name before it pushes the age out.
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .firstTextBaseline, spacing: 8) { name; chip }
                VStack(alignment: .leading, spacing: 4) { name; chip }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            // The age and the buttons share one place as wide as the wider of them, so the name
            // and the branch never reflow when the pointer comes or goes, and a card never grows
            // under the pointer and pushes the cards below it away.
            ZStack(alignment: .trailing) {
                if let age = card.age {
                    Text(age)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .opacity(hovered ? 0 : 1)
                }
                OpenButtons(path: card.path)
                    .opacity(hovered ? 1 : 0)
                    .allowsHitTesting(hovered)
                    .accessibilityHidden(!hovered)
            }
        }
    }

    private var name: some View {
        Text(card.name)
            .font(.title3.weight(.semibold))
            .lineLimit(1)
            .truncationMode(.middle)
            .hoverTip(card.nameTooltip)
    }

    @ViewBuilder
    private var chip: some View {
        if let branch = card.branch {
            Text(branch.text)
                .font(.callout.monospaced())
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .padding(.horizontal, 6)
                .padding(.vertical, 1)
                .background(Color(nsColor: .quaternaryLabelColor), in: .rect(cornerRadius: 4))
                .hoverTip(branch.tooltip)
        }
    }
}

/// A button for each app that opens the folder: the editor chosen in Settings, when this Mac has
/// one, and Finder.
private struct OpenButtons: View {
    let path: String

    @Environment(\.native) private var native
    @AppStorage(Native.editorChoiceKey) private var chosenEditor = ""

    var body: some View {
        HStack(spacing: 4) {
            if let editor = native.editor(chosen: chosenEditor) {
                OpenButton(app: editor) { native.open(path, in: editor) }
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
