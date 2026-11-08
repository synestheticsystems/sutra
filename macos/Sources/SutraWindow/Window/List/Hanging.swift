import SutraCore
import SwiftUI

extension View {
    /// This row at its place in its group's tree (see `Hanging`).
    func hanging(at place: Place) -> some View {
        modifier(Hanging(place: place))
    }
}

/// A row hanging at its place in its group's tree: set in a step from the list's edge for each line
/// it hangs under, and crossed by the guide that runs down from each of those lines' chevrons past
/// every row the line holds, so the rows read as their lines' at a glance. Each row draws its own
/// stretch of each guide, down its height and up through the room above it, as a lazy list lays
/// out each row alone; the first row under a line reaches on up into the line, to just below its
/// chevron, so the stretches join into one line from each chevron to the foot of the last row it
/// holds.
struct Hanging: ViewModifier {
    let place: Place

    /// How far each step sets a row in: well past the chevron of the line above it.
    static let step: CGFloat = 24

    /// Where the guide of the line `depth` steps in runs, from the list's edge: down the middle of
    /// that line's chevron.
    static func guide(at depth: Int) -> CGFloat {
        CGFloat(depth) * step + FoldingLine.inset + FoldingLine.chevronWidth / 2
    }

    func body(content: Content) -> some View {
        content
            .padding(.leading, Self.step * CGFloat(place.depth))
            .background(alignment: .topLeading) { guides }
    }

    /// A stretch of the guide of each line the row hangs under.
    private var guides: some View {
        let depth = Int(place.depth)
        return ZStack(alignment: .topLeading) {
            ForEach(0..<depth, id: \.self) { line in
                // Only the guide of the line the row hangs from first reaches into that line.
                let intoTheLine = place.first && line == depth - 1
                Rectangle()
                    .fill(Surface.guide)
                    .frame(width: 1)
                    .padding(.top, -(ListView.rowSpacing + (intoTheLine ? FoldingLine.padding : 0)))
                    .offset(x: Self.guide(at: line) - 0.5)
            }
        }
        .accessibilityHidden(true)
    }
}
