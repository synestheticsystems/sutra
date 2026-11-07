import SutraCore
import SwiftUI

/// The line of the cards that share a parent, a repository's worktrees or a session's scratch
/// folders: a disclosure chevron, the parent's name, and the disk its cards take, inside the mark
/// of scratch on a session's line. Open, the cards follow the line in rows of their own, nested
/// under it (`NestedCardRow`). A click on the line folds the cards away, and another shows them
/// again. Folded, the line holds the robots of the agents in its cards at its end and the servers
/// running in them after its total, and throbs as their cards would while one of them waits on the
/// user. A right click on a session's line offers to clean the build folders in its scratch.
struct GroupLineRow: View {
    let line: GroupLine
    let fold: Fold
    let dashboard: DashboardModel

    var body: some View {
        switch fold {
        case .open:
            header(open: true)
                .padding(.horizontal, FoldingLine.inset)
                .padding(.vertical, FoldingLine.padding)
        case .folded(let sessions, let subagents, let waiting, let units, let moreUnits):
            HStack(spacing: 12) {
                header(open: false, units: units, moreUnits: moreUnits)
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

    /// The line itself, all of it one button: the chevron, pointing at the cards while they show,
    /// the name and the total, in a session's scratch mark; folded, the servers running in its
    /// cards after them, `units` drawn and `moreUnits` counted. A click on a server here opens the
    /// cards, where a server opens in the browser.
    private func header(
        open: Bool, units: [UnitLozenge] = [], moreUnits: MoreUnits? = nil
    ) -> some View {
        Button {
            dashboard.perform(.toggleFold(parent: line.parent))
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                Chevron(open: open)
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(line.name)
                        .fontWeight(.semibold)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if let total = line.total {
                        if let scratch = line.scratch {
                            // On a session's line the mark goes around the total, which is
                            // exactly what is throwaway.
                            Text(total).scratchMarked(scratch)
                        } else {
                            Text(total)
                        }
                    }
                    ForEach(Array(units.enumerated()), id: \.offset) { _, unit in
                        UnitCapsule(lozenge: unit)
                            .hoverTip(unit.tooltip)
                    }
                    if let moreUnits {
                        Text(moreUnits.label)
                            .hoverTip(moreUnits.tooltip)
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
        // A session's line offers to clean the build folders in its scratch; a repository's
        // offers nothing.
        .contextMenu { MenuEntries(entries: line.menu, dashboard: dashboard) }
        .accessibilityAddTraits(.isHeader)
        .accessibilityValue(open ? "Open" : "Folded")
    }
}
