import SutraCore
import SwiftUI

/// The card's tray along its bottom edge, all on one line: at its start the folder's disk use, as
/// `32 GB  target/  node_modules/` reads, with an alert sign first when the folder could not be
/// sized; at its far end the subagents at work in the folder. While a clean is at work, the seam
/// bar along the tray's top edge fills as space comes back. While the pointer is over the card, the
/// folder's full path shows in the room between.
struct TrayView: View {
    let tray: Tray
    /// The folder in full, with `~` for the home folder.
    let location: String
    let hovered: Bool

    var body: some View {
        VStack(spacing: 0) {
            if let progress = tray.disk?.progress {
                // The seam bar. VoiceOver reads what is left in the folders' tooltips instead.
                FillBar(progress: progress, fill: Color.accentColor, thickness: 3)
                    .accessibilityHidden(true)
            }
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                if let disk = tray.disk {
                    DiskUseLine(disk: disk)
                        .layoutPriority(1)
                }
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
                if !tray.subagents.isEmpty {
                    // Laid out first and never cut, so the icons keep their place at the end.
                    SubagentIcons(icons: tray.subagents)
                        .fixedSize()
                        .layoutPriority(2)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Surface.tray)
        }
    }
}

/// The folder's disk use: an alert sign when it could not be sized, its total, then its build
/// folders, wrapping where the tray runs out of width.
private struct DiskUseLine: View {
    let disk: DiskUse

    var body: some View {
        WrappingLine(spacing: 12) {
            if let warning = disk.warning {
                Image(systemName: "exclamationmark.triangle.fill")
                    .symbolRenderingMode(.multicolor)
                    .hoverTip(warning)
                    .accessibilityLabel(warning)
            }
            if let total = disk.total {
                Text(total)
                    .font(.callout.weight(.medium))
                    .hoverTip(disk.totalTooltip)
            }
            ForEach(Array(disk.folders.enumerated()), id: \.offset) { _, folder in
                Text(folder.label)
                    .font(.callout)
                    // A folder a clean is at work on fades: what it holds is on its way out.
                    .foregroundStyle(folder.cleaning ? .tertiary : .secondary)
                    .hoverTip(folder.tooltip)
            }
        }
    }
}
