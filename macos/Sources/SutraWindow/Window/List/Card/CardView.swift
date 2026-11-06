import SutraCore
import SwiftUI

/// One project's card: its identity, then what is happening in it (why its folder cannot be read,
/// its units), and its tray along the bottom, with its disk use and a robot for each agent in it.
/// It throbs amber while an agent in it waits on the user. A right click offers what
/// can be done to the project.
public struct CardView: View {
    private let model: CardModel
    private let dashboard: DashboardModel
    private let forcedHover: Bool?

    @State private var pointerOver = false

    /// `forcedHover` draws the card as if the pointer were over it or not, whatever the pointer
    /// does, for pictures of a card drawn away from the window.
    public init(model: CardModel, dashboard: DashboardModel, forcedHover: Bool? = nil) {
        self.model = model
        self.dashboard = dashboard
        self.forcedHover = forcedHover
    }

    public var body: some View {
        let card = model.card
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 8) {
                CardHeader(card: card, hovered: forcedHover ?? pointerOver)
                Happening(card: card)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            if let tray = card.tray {
                TrayView(tray: tray, location: card.location, hovered: forcedHover ?? pointerOver)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .clipShape(.rect(cornerRadius: Surface.cornerRadius - 1))
        // Inside the edge, so the tray and the seam bar never cover it.
        .padding(1)
        .background { CardSurface(waiting: card.waiting) }
        .contextMenu { MenuEntries(entries: card.menu, dashboard: dashboard, folder: card.path) }
        .onHover { over in
            withAnimation(.easeOut(duration: 0.15)) { pointerOver = over }
        }
    }
}

/// What is happening in the project, a line each: why its folder cannot be read, when it cannot,
/// above the units that sources outside the folder still show; its units. Its agents stand on its
/// tray.
private struct Happening: View {
    let card: Card

    var body: some View {
        if card.unreadable != nil || !card.units.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                if let unreadable = card.unreadable {
                    UnreadableLine(unreadable: unreadable)
                }
                if !card.units.isEmpty {
                    WrappingLine(spacing: 6) {
                        ForEach(Array(card.units.enumerated()), id: \.offset) { _, unit in
                            UnitLozengeView(lozenge: unit)
                        }
                    }
                }
            }
        }
    }
}

/// Why the folder cannot be read. A folder kept out by the user's own choice, or until macOS asks
/// them, gets a quiet sign, so the warning sign stays for a folder that is missing or locked.
private struct UnreadableLine: View {
    let unreadable: Unreadable

    var body: some View {
        switch unreadable.look {
        case .keptOut:
            Label {
                Text(unreadable.reason)
            } icon: {
                Image(systemName: "eye.slash")
            }
            .foregroundStyle(.secondary)
        case .failed:
            Label {
                Text(unreadable.reason)
            } icon: {
                Image(systemName: "exclamationmark.triangle.fill")
                    .symbolRenderingMode(.multicolor)
            }
        }
    }
}
