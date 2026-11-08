import SutraCore
import SwiftUI

/// A line of the list: a group's, which names the upstream its cards came from, or within a group a
/// checkout's, which holds its own card and the copies made from it. A disclosure chevron, the
/// line's name, and the disk all it holds takes. Open, its rows follow it, each hanging under it
/// (`Hanging`). A click on the line folds them away, and another shows them again. Folded, the line
/// holds the robots of the agents in its rows at its end and the servers running in them after its
/// total, and throbs as their cards would while one of them waits on the user. A right click offers
/// to clean the build folders in what it holds.
struct LineRow: View {
    let line: Line
    let dashboard: DashboardModel

    var body: some View {
        switch line.fold {
        case .open:
            header(open: true)
                .padding(.horizontal, FoldingLine.inset)
                .padding(.vertical, FoldingLine.padding)
        case .folded(let sessions, let subagents, let waiting, let lozenges, let moreLozenges):
            HStack(spacing: 12) {
                header(open: false, lozenges: lozenges, moreLozenges: moreLozenges)
                if !sessions.isEmpty || !subagents.isEmpty {
                    // Laid out first and never cut, as on a card's tray.
                    TrayAgents(sessions: sessions, subagents: subagents)
                        .fixedSize()
                        .layoutPriority(1)
                }
            }
            .padding(.leading, FoldingLine.inset)
            // The robots end where a card's tray ends them, so every robot on the list stands in
            // one column.
            .padding(.trailing, Surface.edgeWidth + Surface.contentInset)
            .padding(.vertical, FoldingLine.padding)
            // Open, the cards of the agents that wait throb themselves; folded, the line throbs
            // for them.
            .background {
                if waiting {
                    Throb(cornerRadius: Surface.cornerRadius)
                }
            }
        }
    }

    /// The line itself, all of it one button: the chevron, pointing at the rows while they show,
    /// the name and the total; folded, the servers running in its cards after them, `lozenges`
    /// drawn and `moreLozenges` counted. A click on a server here opens the line, where a server
    /// opens in the browser.
    private func header(
        open: Bool, lozenges: [Lozenge] = [], moreLozenges: MoreLozenges? = nil
    ) -> some View {
        Button {
            dashboard.perform(.toggleFold(line: line.key))
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                Chevron(open: open)
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(line.name)
                        .fontWeight(.semibold)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if let total = line.total {
                        Text(total)
                    }
                    ForEach(Array(lozenges.enumerated()), id: \.offset) { _, lozenge in
                        LozengeCapsule(lozenge: lozenge)
                            .hoverTip(lozenge.tooltip)
                    }
                    if let moreLozenges {
                        Text(moreLozenges.label)
                            .hoverTip(moreLozenges.tooltip)
                    }
                }
            }
            .font(.callout)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .hoverTip(line.tooltip)
        .contextMenu { MenuEntries(entries: line.menu, dashboard: dashboard) }
        .accessibilityAddTraits(.isHeader)
        .accessibilityValue(open ? "Open" : "Folded")
    }
}
