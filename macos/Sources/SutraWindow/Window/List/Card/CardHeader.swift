import SutraCore
import SwiftUI

/// A card's identity: the name, the chip (a branch, or the scratch space's mark), and the age at
/// the right, whose place the buttons that open the folder take while the pointer is over the card.
struct CardHeader: View {
    let card: Card
    let hovered: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            // The chip moves under the name before it pushes the age out.
            NameAndChip { name; chip }
                .frame(maxWidth: .infinity, alignment: .leading)
            // The age and the buttons share one place as wide as the wider of them, so the name
            // and the chip never reflow when the pointer comes or goes, and a card never grows
            // under the pointer and pushes the cards below it away.
            ZStack(alignment: .trailing) {
                // With no age, a blank stands in its place: the row then still aligns on a line
                // of text rather than on the buttons' taller frame, which would lower the name and
                // stand the card taller than the rest.
                Text(card.age ?? " ")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .opacity(hovered ? 0 : 1)
                    .accessibilityHidden(card.age == nil)
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
        switch card.chip {
        case .branch(let text, let tooltip):
            CodeChip(text: text, tooltip: tooltip)
        case .scratch(let session, let mark):
            // The session's name is the mark's own words, so once the session has ended it steps
            // back with the outline, as a robot at rest does; the card's own words stay.
            Text(session)
                .font(.callout)
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
                .opacity(mark.sessionRuns ? 1 : Still.opacity)
                .scratchMarked(mark)
        case nil:
            EmptyView()
        }
    }
}

/// A chip of words written as code beside a card's name, as a branch reads on a checkout's card and
/// an image on a container's: monospaced on a quiet fill, cut in the middle where short of room.
struct CodeChip: View {
    let text: String
    let tooltip: String

    var body: some View {
        Text(text)
            .font(.callout.monospaced())
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .truncationMode(.middle)
            .padding(.horizontal, 6)
            .padding(.vertical, 1)
            .background(Color(nsColor: .quaternaryLabelColor), in: .rect(cornerRadius: 4))
            .hoverTip(tooltip)
    }
}

/// A card's name and its chip: side by side on their first baselines where both fit at their ideal
/// widths, otherwise the chip under the name, each cut to the width there is. Laid out by hand
/// rather than by `ViewThatFits`, which builds the views of whichever arrangement fits while it
/// measures. The stack around the name measures it at several widths in each pass of layout, so a
/// pass can tear one arrangement down and build the other, and that change sets off another pass.
/// A layout only measures and places.
struct NameAndChip: Layout {
    /// The room between the name and the chip beside it.
    private static let besideSpacing: CGFloat = 8
    /// The room between the name and the chip under it.
    private static let underSpacing: CGFloat = 4

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        arrangement(of: subviews, within: proposal.width).size
    }

    func placeSubviews(
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) {
        let arrangement = arrangement(of: subviews, within: proposal.width)
        for (subview, origin) in zip(subviews, arrangement.origins) {
            subview.place(
                at: CGPoint(x: bounds.minX + origin.x, y: bounds.minY + origin.y),
                proposal: arrangement.proposal
            )
        }
    }

    private struct Arrangement {
        /// Where each subview's top leading corner goes.
        var origins: [CGPoint] = []
        /// What each subview is offered: its ideal size side by side, the width there is one
        /// under another.
        var proposal: ProposedViewSize
        var size: CGSize = .zero
    }

    /// The subviews side by side if their ideal widths fit in `width`, any width when `nil`, or
    /// else one under another.
    private func arrangement(of subviews: Subviews, within width: CGFloat?) -> Arrangement {
        let ideal = subviews.map { $0.dimensions(in: .unspecified) }
        let besideWidth = ideal.reduce(0) { $0 + $1.width }
            + Self.besideSpacing * CGFloat(max(ideal.count - 1, 0))
        if besideWidth <= width ?? .infinity {
            let baselines = ideal.map { $0[VerticalAlignment.firstTextBaseline] }
            let baseline = baselines.max() ?? 0
            let descent = zip(ideal, baselines).map { $0.height - $1 }.max() ?? 0
            var beside = Arrangement(
                proposal: .unspecified, size: CGSize(width: besideWidth, height: baseline + descent)
            )
            var x: CGFloat = 0
            for (dimensions, own) in zip(ideal, baselines) {
                beside.origins.append(CGPoint(x: x, y: baseline - own))
                x += dimensions.width + Self.besideSpacing
            }
            return beside
        }
        var under = Arrangement(proposal: ProposedViewSize(width: width, height: nil))
        for subview in subviews {
            let size = subview.sizeThatFits(under.proposal)
            let y = under.origins.isEmpty ? 0 : under.size.height + Self.underSpacing
            under.origins.append(CGPoint(x: 0, y: y))
            under.size = CGSize(width: max(under.size.width, size.width), height: y + size.height)
        }
        return under
    }
}
