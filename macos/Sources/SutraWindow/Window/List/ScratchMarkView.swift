import SutraCore
import SwiftUI

/// The mark of Claude Code's scratch space: a scribble, then on a scratch folder's card where the
/// folder lies in its session's folder, inside an outline of dashes. On a session's line, the
/// scribble alone marks the whole space. A branch's chip is filled, as a label on something kept;
/// scratch is thrown away, so its mark is drawn the way a placeholder is, and the two never read as
/// the same kind of thing.
struct ScratchMarkView: View {
    let mark: ScratchMark

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Image(systemName: "scribble")
                .accessibilityHidden(true)
            if let place = mark.place {
                Text(place)
                    .truncationMode(.middle)
            }
        }
        .font(.callout.monospaced())
        .foregroundStyle(.secondary)
        .lineLimit(1)
        .padding(.horizontal, 6)
        .padding(.vertical, 1)
        .overlay {
            RoundedRectangle(cornerRadius: 4)
                .strokeBorder(
                    Color(nsColor: .tertiaryLabelColor),
                    style: StrokeStyle(lineWidth: 1, dash: Scratch.dash)
                )
        }
        .hoverTip(mark.tooltip)
    }
}
