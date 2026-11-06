import SutraCore
import SwiftUI

/// The list, or what stands in its place: the store edition's request for a folder, the first
/// scan under way, or why there is nothing to list. The core holds the cards in place while the
/// pointer is over the list, so the list tells it when the pointer comes and goes.
struct ListView: View {
    let dashboard: DashboardModel

    @State private var pointerInside = false

    var body: some View {
        // One view whatever it shows, so the pointer's coming and going is tracked across a
        // change from rows to a message and back.
        ZStack {
            switch dashboard.listing {
            case .noFolder(let message, let lost):
                NoFolder(message: message, lost: lost, dashboard: dashboard)
            case .scanning:
                ProgressView()
                    .controlSize(.small)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            case .empty(let message):
                Text(message)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
                    .padding(40)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            case .rows(let rows):
                Rows(rows: rows, dashboard: dashboard)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .onHover { inside in
            pointerInside = inside
            dashboard.perform(inside ? .pointerEnteredList : .pointerLeftList)
        }
        // The help panel takes the list's place under a still pointer, which then never leaves.
        .onDisappear {
            if pointerInside {
                pointerInside = false
                dashboard.perform(.pointerLeftList)
            }
        }
    }
}

private struct Rows: View {
    let rows: [Row]
    let dashboard: DashboardModel

    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 8) {
                ForEach(rows) { row in
                    switch row {
                    case .card(let card):
                        CardView(model: card, dashboard: dashboard)
                    case .repository(let commonDir, let name, let tooltip, let total, let fold):
                        RepositoryGroup(
                            commonDir: commonDir, name: name, tooltip: tooltip, total: total,
                            fold: fold, dashboard: dashboard
                        )
                    case .elsewhere(let heading, let lines, let waiting):
                        ElsewhereCard(heading: heading, lines: lines, waiting: waiting)
                    case .quietProjects(let open, let label, let tooltip):
                        QuietProjectsRow(open: open, label: label, tooltip: tooltip, dashboard: dashboard)
                    }
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 8)
            .padding(.bottom, 12)
        }
    }
}

/// The lines of the list that fold something away, a repository's and the one standing for the
/// quiet projects, all laid out alike, so their chevrons stand in one column.
private enum FoldingLine {
    /// How far a line's chevron stands in from the list's edge.
    static let inset: CGFloat = 8
    /// The room above and below a line's text, which a folded repository's throb fills.
    static let padding: CGFloat = 7
    /// The width of the chevron's slot, fixed so the guide under an open repository's chevron runs
    /// down its middle.
    static let chevronWidth: CGFloat = 10
}

/// A folding line's chevron, pointing at what the line holds while it shows.
private struct Chevron: View {
    let open: Bool

    var body: some View {
        Image(systemName: "chevron.right")
            .font(.caption.weight(.semibold))
            .rotationEffect(.degrees(open ? 90 : 0))
            .frame(width: FoldingLine.chevronWidth)
            .accessibilityHidden(true)
    }
}

/// One repository's worktrees: its line, with a disclosure chevron, the repository's name and the
/// disk its worktrees take, then their cards, set in from the list's edge and hung from a guide
/// that runs down from the chevron, so they read as the repository's at a glance. A click on the
/// line folds the cards away, and another shows them again. Folded, the line holds the robots of
/// the agents in its worktrees at its end, and throbs as their cards would while one of them waits
/// on the user.
private struct RepositoryGroup: View {
    /// The git folder the worktrees share, which the click names to the core.
    let commonDir: String
    let name: String
    /// Where the repository is.
    let tooltip: String
    let total: String?
    let fold: Fold
    let dashboard: DashboardModel

    /// The room between the line and the cards, and between cards, as between the list's rows.
    private static let spacing: CGFloat = 8
    /// How far the cards stand in from the list's edge, well past the chevron.
    private static let cardIndent: CGFloat = 24
    /// Where the guide runs, from the list's edge: down the middle of the chevron.
    private static let guideOffset = FoldingLine.inset + FoldingLine.chevronWidth / 2

    var body: some View {
        switch fold {
        case .open(let cards):
            VStack(alignment: .leading, spacing: Self.spacing) {
                header(open: true)
                    .padding(.horizontal, FoldingLine.inset)
                    .padding(.vertical, FoldingLine.padding)
                VStack(alignment: .leading, spacing: Self.spacing) {
                    ForEach(cards, id: \.path) { card in
                        CardView(model: card, dashboard: dashboard)
                    }
                }
                .padding(.leading, Self.cardIndent)
                .background(alignment: .topLeading) { guide }
            }
        case .folded(let sessions, let subagents, let waiting):
            HStack(spacing: 12) {
                header(open: false)
                if !sessions.isEmpty || !subagents.isEmpty {
                    // Laid out first and never cut, as on a card's tray.
                    TrayAgents(sessions: sessions, subagents: subagents)
                        .fixedSize()
                        .layoutPriority(1)
                }
            }
            .padding(.horizontal, FoldingLine.inset)
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

    /// The hairline the cards hang from, from just below the chevron to the foot of the last card.
    private var guide: some View {
        Rectangle()
            .fill(Surface.guide)
            .frame(width: 1)
            // Up through the room between the line and the first card, to just below the chevron,
            // which sits above the foot of the line's text.
            .padding(.top, -(Self.spacing + FoldingLine.padding))
            .offset(x: Self.guideOffset - 0.5)
            .accessibilityHidden(true)
    }

    /// The line itself, all of it one button: the chevron, pointing at the cards while they show,
    /// the name and the total.
    private func header(open: Bool) -> some View {
        Button {
            dashboard.perform(.toggleRepositoryFold(commonDir: commonDir))
        } label: {
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                Chevron(open: open)
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(name)
                        .fontWeight(.semibold)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    if let total {
                        Text(total)
                    }
                }
            }
            .font(.callout)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .hoverTip(tooltip)
        .accessibilityAddTraits(.isHeader)
        .accessibilityValue(open ? "Open" : "Folded")
    }
}

/// The store edition with no folder to read yet: what Sutra needs and why, and the button that
/// asks for the folder.
private struct NoFolder: View {
    let message: String
    let lost: String?
    let dashboard: DashboardModel

    var body: some View {
        VStack(spacing: 12) {
            Text(message)
                .multilineTextAlignment(.center)
            Button("Choose Folder\u{2026}") { dashboard.chooseFolder() }
                .controlSize(.large)
                .keyboardShortcut(.defaultAction)
            if let lost {
                Text(lost)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
        }
        .frame(maxWidth: 400)
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// The row standing for the quiet projects: a chevron and how many, which opens to list them below
/// it and closes again.
private struct QuietProjectsRow: View {
    let open: Bool
    let label: String
    let tooltip: String
    let dashboard: DashboardModel

    var body: some View {
        Button {
            dashboard.perform(.toggleQuietProjects)
        } label: {
            HStack(spacing: 4) {
                Chevron(open: open)
                Text(label)
                    .font(.callout)
            }
            .foregroundStyle(.secondary)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .hoverTip(tooltip)
        .accessibilityValue(tooltip)
        .padding(.horizontal, FoldingLine.inset)
        .padding(.vertical, FoldingLine.padding)
    }
}
