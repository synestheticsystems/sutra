import SwiftUI

/// Pieces laid out on one line, left to right, wrapping onto the next line where the width runs
/// out: a card's lozenges, the tray's entries. Each line centres its pieces on one another. A piece
/// wider than the whole line gets a line to itself, at the line's width.
struct WrappingLine: Layout {
    var spacing: CGFloat
    var lineSpacing: CGFloat = 4

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let lines = arrange(subviews, within: proposal.width ?? .infinity)
        let width = lines.map(\.width).max() ?? 0
        let height = lines.map(\.height).reduce(0, +) + lineSpacing * CGFloat(max(lines.count - 1, 0))
        return CGSize(width: width, height: height)
    }

    func placeSubviews(
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) {
        var y = bounds.minY
        for line in arrange(subviews, within: bounds.width) {
            var x = bounds.minX
            for piece in line.pieces {
                let top = y + (line.height - piece.size.height) / 2
                // Placed with the proposal it was measured with: proposed its own measured size,
                // text can come out a fraction short and truncate.
                subviews[piece.index].place(at: CGPoint(x: x, y: top), proposal: piece.proposal)
                x += piece.size.width + spacing
            }
            y += line.height + lineSpacing
        }
    }

    private struct Piece {
        let index: Int
        let proposal: ProposedViewSize
        let size: CGSize
    }

    private struct Line {
        var pieces: [Piece] = []
        var width: CGFloat = 0
        var height: CGFloat = 0
    }

    private func arrange(_ subviews: Subviews, within width: CGFloat) -> [Line] {
        var lines: [Line] = []
        var line = Line()
        for (index, subview) in subviews.enumerated() {
            var proposal = ProposedViewSize.unspecified
            var size = subview.sizeThatFits(proposal)
            if size.width > width {
                proposal = ProposedViewSize(width: width, height: nil)
                size = subview.sizeThatFits(proposal)
            }
            let needed = line.pieces.isEmpty ? size.width : line.width + spacing + size.width
            if needed > width, !line.pieces.isEmpty {
                lines.append(line)
                line = Line()
            }
            line.width = line.pieces.isEmpty ? size.width : line.width + spacing + size.width
            line.height = max(line.height, size.height)
            line.pieces.append(Piece(index: index, proposal: proposal, size: size))
        }
        if !line.pieces.isEmpty {
            lines.append(line)
        }
        return lines
    }
}
