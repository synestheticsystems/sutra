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
            shape
                .fill(Surface.card)
                // Faint in both modes: in dark mode the surface's step up from the window and its
                // edge already lift the card, and a dark halo only muddies the gap between cards.
                .shadow(
                    color: .black.opacity(colorScheme == .dark ? 0.15 : 0.08),
                    radius: 2,
                    y: 1
                )
            shape.strokeBorder(Surface.edge)
            if waiting {
                Throb(cornerRadius: Surface.cornerRadius)
            }
        }
    }
}

/// The throb of something an agent in it waits on: an amber edge and a faint amber wash, swelling
/// and fading together. A card throbs over its surface; a folded line, which has no
/// surface of its own, throbs alone.
struct Throb: View {
    let cornerRadius: CGFloat

    var body: some View {
        SwellLayer(swell: Surface.wash, shape: .rectangle(cornerRadius: cornerRadius))
        SwellLayer(swell: Surface.waitingEdge, shape: .edge(cornerRadius: cornerRadius, width: 1))
    }
}
