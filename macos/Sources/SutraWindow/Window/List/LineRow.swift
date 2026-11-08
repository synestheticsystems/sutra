import SutraCore
import SwiftUI

/// A line of the list: a group's, which names the upstream its cards came from, or within a group a
/// checkout's, which holds its own card and the copies made from it. A disclosure chevron, the
/// line's name, and the disk all it holds takes, then a quiet mark on a group's line whose build
/// folders Sutra cleans by itself once idle. Open, its rows follow it, each hanging under it
/// (`Hanging`). A click on the line folds them away, and another shows them again. Folded, the line
/// holds the robots of the agents in its rows at its end and the servers running in them after its
/// total, each opening its port on a click as on its card, and throbs as their cards would while
/// one of them waits on the user. A right click offers to clean the build folders in what it
/// holds, and on a group's line, how long they may sit idle before Sutra cleans them.
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

    /// The line itself: the chevron, pointing at the rows while they show, the name and the total,
    /// a button that folds or opens the line; folded, the servers running in its cards after them,
    /// `lozenges` drawn as their cards draw them, each opening its port, and `moreLozenges`
    /// counted. A click anywhere else on the line folds or opens it too.
    private func header(
        open: Bool, lozenges: [Lozenge] = [], moreLozenges: MoreLozenges? = nil
    ) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Button(action: toggle) {
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
                        if let cleansWhenIdle = line.cleansWhenIdle {
                            // A mark rather than words, quieter than the total beside it: its tip
                            // says how long the folders may sit idle.
                            Image(systemName: "clock.arrow.circlepath")
                                .imageScale(.small)
                                .foregroundStyle(.tertiary)
                                .hoverTip(cleansWhenIdle)
                                .accessibilityLabel(cleansWhenIdle)
                        }
                    }
                }
                .contentShape(.rect)
            }
            .buttonStyle(.plain)
            .accessibilityAddTraits(.isHeader)
            .accessibilityValue(open ? "Open" : "Folded")
            ForEach(Array(lozenges.enumerated()), id: \.offset) { _, lozenge in
                LozengeView(lozenge: lozenge)
            }
            if let moreLozenges {
                Text(moreLozenges.label)
                    .hoverTip(moreLozenges.tooltip)
            }
        }
        .font(.callout)
        .foregroundStyle(.secondary)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(.rect)
        // A server's own button takes a click on it before the line does.
        .onTapGesture(perform: toggle)
        .hoverTip(line.tooltip)
        .contextMenu { MenuEntries(entries: line.menu, dashboard: dashboard) }
    }

    private func toggle() {
        dashboard.perform(.toggleFold(line: line.key))
    }
}
