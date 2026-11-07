import SutraCore
import SwiftUI

/// Docker, on a card of its own, drawn as a project's card is: its name, and at the right how long
/// ago its newest container was made; the lozenges of the containers it runs for no project; and
/// along its bottom the disk Docker takes, on a tray like a project's, its socket showing there
/// while the pointer is over the card. It has neither the buttons that open a folder nor a menu,
/// as Sutra only reads Docker.
struct DockerCardView: View {
    let card: DockerCard

    @State private var pointerOver = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            VStack(alignment: .leading, spacing: 8) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(card.name)
                        .font(.title3.weight(.semibold))
                        .lineLimit(1)
                        .hoverTip(card.nameTooltip)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    // With no age, a blank keeps the card as tall as a project's.
                    Text(card.age ?? " ")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .accessibilityHidden(card.age == nil)
                }
                if !card.lozenges.isEmpty {
                    WrappingLine(spacing: 6) {
                        ForEach(Array(card.lozenges.enumerated()), id: \.offset) { _, lozenge in
                            LozengeView(lozenge: lozenge)
                        }
                    }
                }
            }
            .padding(.horizontal, Surface.contentInset)
            .padding(.vertical, 12)
            if let tray = card.tray {
                TrayView(tray: tray, location: card.location, hovered: pointerOver)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .clipShape(.rect(cornerRadius: Surface.cornerRadius - 1))
        // Inside the edge, so the tray never covers it.
        .padding(Surface.edgeWidth)
        .background { CardSurface(waiting: false) }
        .onHover { over in
            withAnimation(.easeOut(duration: 0.15)) { pointerOver = over }
        }
    }
}
