import SwiftUI

/// A card of an open group, nested under the group's line: set in from the list's edge, well past
/// the line's chevron, and hung from its stretch of the guide that runs down from the chevron, so
/// the group's cards read as the line's at a glance.
struct NestedCardRow: View {
    let model: CardModel
    let guide: GuideStretch
    let dashboard: DashboardModel

    /// How far the card stands in from the list's edge, well past the chevron.
    private static let indent: CGFloat = 24
    /// Where the guide runs, from the list's edge: down the middle of the chevron.
    private static let guideOffset = FoldingLine.inset + FoldingLine.chevronWidth / 2

    var body: some View {
        CardView(model: model, dashboard: dashboard)
            .padding(.leading, Self.indent)
            .background(alignment: .topLeading) { stretch }
    }

    /// The card's stretch of the guide: from the foot of the card up through the room above it, to
    /// the card above, or for the first card on up into the line, to just below the chevron, which
    /// sits above the foot of the line's text.
    private var stretch: some View {
        hairline
            .frame(width: 1)
            .padding(.top, -(ListView.rowSpacing + (guide.first ? FoldingLine.padding : 0)))
            .offset(x: Self.guideOffset - 0.5)
            .accessibilityHidden(true)
    }

    /// Unbroken under a repository, whose worktrees are kept; in dots under a session's scratch,
    /// broken as its mark is, so cards whose line has scrolled away still read as scratch.
    @ViewBuilder
    private var hairline: some View {
        if guide.dotted {
            DotsDownTheMiddle()
                .fill(Scratch.guideInk)
        } else {
            Rectangle()
                .fill(Surface.guide)
        }
    }
}

/// Round dots down the middle of the frame, a stretch of a dotted guide.
struct DotsDownTheMiddle: Shape {
    func path(in rect: CGRect) -> Path {
        let radius = Scratch.guideDot / 2
        var path = Path()
        for depth in Self.depths(down: rect.height) {
            path.addEllipse(in: CGRect(
                x: rect.midX - radius, y: rect.minY + depth - radius, width: 2 * radius,
                height: 2 * radius
            ))
        }
        return path
    }

    /// How far down a stretch `height` tall each of its dots stands: the first at its top, the rest
    /// spread evenly, about `Scratch.guideDotSpacing` apart, so the stretches of a guide, each
    /// drawn by its own row, keep one rhythm down the whole guide. The dot at the foot is the next
    /// stretch's first, so it is left to that stretch.
    static func depths(down height: CGFloat) -> [CGFloat] {
        let gaps = max(1, (height / Scratch.guideDotSpacing).rounded())
        return (0..<Int(gaps)).map { CGFloat($0) * height / gaps }
    }
}
