import SutraCore
import SwiftUI

/// A card's identity: the name, the branch chip, and the age at the right. While the pointer is
/// over the card, the folder's full path and the buttons that open it take the age's place.
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
            if let age = card.age {
                Text(age)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .opacity(hovered ? 0 : 1)
            }
        }
        // Drawn over the line rather than in it, so a hovered card keeps its size: a card that
        // grew under the pointer would push the cards below it away as the pointer moved down the
        // list. The path covers what it needs of the name or branch instead of reflowing them.
        .overlay(alignment: Alignment(horizontal: .trailing, vertical: .firstTextBaseline)) {
            if hovered {
                HoveredLocation(card: card)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
    }

    private var name: some View {
        Text(card.name)
            .font(.title3.weight(.semibold))
            .lineLimit(1)
            .truncationMode(.middle)
            .help(card.nameTooltip)
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
                .help(branch.tooltip)
        }
    }
}

/// Where the folder is, as its full path, and a button for each app that opens it: Visual Studio
/// Code when it is installed, and Finder. It lies on the card's own surface, which fades in over
/// whatever of the name or branch it covers.
private struct HoveredLocation: View {
    let card: Card

    var body: some View {
        HStack(spacing: 6) {
            Text(card.location)
                .font(.callout)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .help(card.location)
            if let vsCode = Native.vsCode {
                OpenButton(app: vsCode) { Native.open(card.path, in: vsCode) }
            }
            if let finder = Native.finder {
                OpenButton(app: finder) { Native.openInFinder(card.path) }
            }
        }
        .padding(.leading, Self.fade + 4)
        // Tall enough to hide the name's ascenders and descenders where it covers them.
        .padding(.vertical, 4)
        .background {
            CardFill(waiting: card.waiting)
                .mask {
                    HStack(spacing: 0) {
                        LinearGradient(colors: [.clear, .black], startPoint: .leading, endPoint: .trailing)
                            .frame(width: Self.fade)
                        Rectangle()
                    }
                }
        }
    }

    /// How far the surface takes to fade in over what it covers.
    private static let fade: CGFloat = 20
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
        }
        .buttonStyle(.plain)
        .help("Open in \(app.name)")
        .accessibilityLabel("Open in \(app.name)")
    }
}
