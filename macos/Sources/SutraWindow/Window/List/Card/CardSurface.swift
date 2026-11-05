import SwiftUI

/// The surface a card is raised on, with its thin edge: a project's, or the one of the agents
/// working elsewhere. While an agent on it waits, the edge turns amber and the surface takes a
/// faint amber wash, swelling and fading with the throb.
struct CardSurface: View {
    let waiting: Bool

    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: Surface.cornerRadius)
        ZStack {
            // The shadow stays with the still fill, so a throb redraws no shadow.
            shape
                .fill(Surface.card)
                .shadow(
                    color: .black.opacity(colorScheme == .dark ? 0.4 : 0.08),
                    radius: 2,
                    y: 1
                )
            shape.strokeBorder(Surface.edge)
            if waiting {
                Rhythmic(rhythm: .throb) { level in
                    ZStack {
                        shape.fill(Surface.wash(at: level))
                        shape.strokeBorder(Hue.amber.fill(level))
                    }
                }
            }
        }
    }
}

/// The card's surface as it shows at this moment, wash and all, under what is drawn over the
/// card's contents and must look like part of the card.
struct CardFill: View {
    let waiting: Bool

    var body: some View {
        ZStack {
            Surface.card
            if waiting {
                Rhythmic(rhythm: .throb) { level in
                    Surface.wash(at: level)
                }
            }
        }
    }
}
