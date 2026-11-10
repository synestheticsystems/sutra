import SutraCore
import SwiftUI

/// A Docker container's card, under Docker's line, drawn as a project's card is: its name, the
/// image it runs beside it where a checkout's card carries its branch, and at the right how long it
/// has run; then its ports, and at the far end of their line, quietly, the projects that account
/// for it. It has neither the buttons that open a folder nor a menu, as Sutra only reads Docker.
struct ContainerCardView: View {
    let card: ContainerCard

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                // The image moves under the name before it pushes the age out.
                NameAndChip {
                    Text(card.name)
                        .font(.title3.weight(.semibold))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .hoverTip(card.nameTooltip)
                    CodeChip(text: card.image, tooltip: card.imageTooltip)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                // With no age, a blank keeps the card as tall as the rest.
                Text(card.age ?? " ")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .accessibilityHidden(card.age == nil)
            }
            if !card.lozenges.isEmpty || card.accountedFor != nil {
                HStack(alignment: .center, spacing: 12) {
                    WrappingLine(spacing: 6) {
                        ForEach(Array(card.lozenges.enumerated()), id: \.offset) { _, lozenge in
                            LozengeView(lozenge: lozenge)
                        }
                    }
                    .layoutPriority(1)
                    if let accountedFor = card.accountedFor {
                        AccountedForLabel(accountedFor: accountedFor)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    }
                }
            }
        }
        .padding(.horizontal, Surface.contentInset)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .clipShape(.rect(cornerRadius: Surface.cornerRadius - 1))
        .padding(Surface.edgeWidth)
        .background { CardSurface(waiting: false) }
    }
}

/// The projects that account for a container, by their folders' names behind a folder's sign, as
/// quiet as the card's age: their cards show the container too, and the tip names their folders in
/// full. Cut in the middle where the ports leave it short of room.
private struct AccountedForLabel: View {
    let accountedFor: AccountedFor

    var body: some View {
        Label {
            Text(accountedFor.names)
                .lineLimit(1)
                .truncationMode(.middle)
        } icon: {
            Image(systemName: "folder")
        }
        .font(.callout)
        .foregroundStyle(.secondary)
        .hoverTip(accountedFor.tooltip)
    }
}
