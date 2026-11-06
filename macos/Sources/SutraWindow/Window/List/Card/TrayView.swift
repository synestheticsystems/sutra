import SutraCore
import SwiftUI

/// The card's disk tray along its bottom edge, all on one line, as `32 GB  target/  node_modules/`
/// reads: an alert sign when the folder could not be sized, its total, then its build folders.
/// While a clean is at work, the seam bar along the tray's top edge fills as space comes back.
/// While the pointer is over the card, the folder's full path shows at the tray's far end.
struct TrayView: View {
    let tray: Tray
    /// The folder in full, with `~` for the home folder.
    let location: String
    let hovered: Bool

    var body: some View {
        VStack(spacing: 0) {
            if let progress = tray.progress {
                // The seam bar. VoiceOver reads what is left in the folders' tooltips instead.
                FillBar(progress: progress, fill: Color.accentColor, thickness: 3)
                    .accessibilityHidden(true)
            }
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                contents
                    .layoutPriority(1)
                // Only in the room the tray leaves, cut in the middle where that is short, so it
                // neither moves nor covers what the tray shows.
                Text(location)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .frame(maxWidth: .infinity, alignment: .trailing)
                    .opacity(hovered ? 1 : 0)
                    .accessibilityHidden(!hovered)
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Surface.tray)
        }
    }

    private var contents: some View {
        WrappingLine(spacing: 12) {
            if let warning = tray.warning {
                Image(systemName: "exclamationmark.triangle.fill")
                    .symbolRenderingMode(.multicolor)
                    .help(warning)
                    .accessibilityLabel(warning)
            }
            if let total = tray.total {
                Text(total)
                    .font(.callout.weight(.medium))
                    .tooltip(tray.totalTooltip)
            }
            ForEach(Array(tray.folders.enumerated()), id: \.offset) { _, folder in
                Text(folder.label)
                    .font(.callout)
                    // A folder a clean is at work on fades: what it holds is on its way out.
                    .foregroundStyle(folder.cleaning ? .tertiary : .secondary)
                    .help(folder.tooltip)
            }
        }
    }
}

extension View {
    /// `text` as the tooltip, when there is one.
    @ViewBuilder
    fileprivate func tooltip(_ text: String?) -> some View {
        if let text {
            help(text)
        } else {
            self
        }
    }
}
