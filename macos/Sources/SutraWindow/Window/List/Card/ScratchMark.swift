import SutraCore
import SwiftUI

extension View {
    /// This view inside the mark of Claude Code's scratch space: an outline of dashes, drawn the
    /// way a placeholder is. A branch's chip is filled, as a label on something kept; scratch is
    /// thrown away, so its mark is drawn as a placeholder, and the two never read as the same kind
    /// of thing. Once the mark's session has ended, the outline steps back as a robot at rest does;
    /// what it holds stays as it is, for the chip that wears it to fade its words with it.
    func scratchMarked(_ mark: ScratchMark) -> some View {
        padding(.horizontal, 6)
            .padding(.vertical, 1)
            .overlay {
                RoundedRectangle(cornerRadius: 4)
                    .strokeBorder(
                        Color(nsColor: .tertiaryLabelColor),
                        style: StrokeStyle(lineWidth: 1, dash: Scratch.dash)
                    )
                    .opacity(mark.sessionRuns ? 1 : Still.opacity)
            }
            .hoverTip(mark.tooltip)
    }
}
