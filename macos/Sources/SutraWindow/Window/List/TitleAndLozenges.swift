import SwiftUI

/// A line's header, on one line: its title (the chevron, name, total and mark, which fold or open
/// the line), then, while the line is folded, the lozenges of the servers under it, then the count
/// of those it does not draw. Short of room, the lozenges give way to their count, the last first,
/// and only once the count stands alone is the title narrowed, its name cut in the middle.
///
/// Every piece stays a subview whatever the room, those not drawn placed out of sight, so a change
/// of width moves pieces and never changes which views a row holds: a row of a lazy list that
/// changes them while it is laid out asks the list for another pass, and a row offered two widths
/// in turn can keep the list from settling. For the same reason the line stands as tall as its
/// tallest piece, drawn or not, so its height never changes with the room either.
struct TitleAndLozenges: Layout {
    /// The room between the pieces.
    static let spacing: CGFloat = 8
    /// How far before the line the pieces not drawn stand: past the window's edge, where nothing
    /// draws them and the pointer never reaches them.
    private static let outOfSight: CGFloat = 100_000

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
        for (subview, place) in zip(subviews, arrangement.places) {
            guard let place else {
                subview.place(
                    at: CGPoint(x: bounds.minX - Self.outOfSight, y: bounds.minY),
                    proposal: .unspecified
                )
                continue
            }
            subview.place(
                at: CGPoint(x: bounds.minX + place.origin.x, y: bounds.minY + place.origin.y),
                proposal: place.proposal
            )
        }
    }

    private struct Place {
        /// Where the piece's top leading corner goes.
        var origin: CGPoint
        /// What the piece is offered: its ideal size, or for a narrowed title the width it has.
        var proposal: ProposedViewSize
    }

    private struct Arrangement {
        /// Each subview's place, `nil` for those not drawn.
        var places: [Place?]
        var size: CGSize
    }

    /// The pieces drawn within `width`, any width when `nil`, and where each goes.
    private func arrangement(of subviews: Subviews, within width: CGFloat?) -> Arrangement {
        let pieces = subviews.map { $0[LinePiece.Key.self] }
        let ideal = subviews.map { $0.dimensions(in: .unspecified) }
        let lozenges = pieces.filter(\.isLozenge).count
        // The subviews drawn with the first `drawn` lozenges: the title, those lozenges, and the
        // count after them if there is one.
        func drawing(_ drawn: Int) -> [Int] {
            let wanted = [LinePiece.title] + (0..<drawn).map(LinePiece.lozenge)
                + [.count(after: drawn)]
            return wanted.compactMap { pieces.firstIndex(of: $0) }
        }
        func widthOf(_ drawn: [Int]) -> CGFloat {
            drawn.reduce(0) { $0 + ideal[$1].width }
                + Self.spacing * CGFloat(max(drawn.count - 1, 0))
        }
        let room = width ?? .infinity
        let drawn = drawing((0...lozenges).reversed().first { widthOf(drawing($0)) <= room } ?? 0)
        // Still short of room with the count alone: the title gives up what is missing.
        let missing = max(widthOf(drawn) - room, 0)
        let baselines = ideal.map { $0[VerticalAlignment.firstTextBaseline] }
        let baseline = baselines.max() ?? 0
        let descent = zip(ideal, baselines).map { $0.height - $1 }.max() ?? 0
        var places = [Place?](repeating: nil, count: subviews.count)
        var x: CGFloat = 0
        for index in drawn {
            var width = ideal[index].width
            var proposal = ProposedViewSize.unspecified
            if pieces[index] == .title, missing > 0 {
                width = max(width - missing, 0)
                proposal = ProposedViewSize(width: width, height: nil)
            }
            let origin = CGPoint(x: x, y: baseline - baselines[index])
            places[index] = Place(origin: origin, proposal: proposal)
            x += width + Self.spacing
        }
        let size = CGSize(width: max(x - Self.spacing, 0), height: baseline + descent)
        return Arrangement(places: places, size: size)
    }
}

/// What a subview of `TitleAndLozenges` is, which decides where it goes.
enum LinePiece: Hashable {
    /// The chevron, name, total and mark, which fold or open the line.
    case title
    /// The lozenge at this place among those the line draws while there is room.
    case lozenge(Int)
    /// The count standing after the first this-many lozenges, while only they are drawn.
    case count(after: Int)

    var isLozenge: Bool {
        if case .lozenge = self { true } else { false }
    }

    fileprivate struct Key: LayoutValueKey {
        static let defaultValue = LinePiece.title
    }
}

extension View {
    /// Marks this view as `piece` of a line's header, laid out by `TitleAndLozenges`.
    func linePiece(_ piece: LinePiece) -> some View {
        layoutValue(key: LinePiece.Key.self, value: piece)
    }
}
