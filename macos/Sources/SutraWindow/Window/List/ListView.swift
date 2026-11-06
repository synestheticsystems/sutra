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
                    case .repository(_, let name, let tooltip, let total, let cards):
                        RepositoryGroup(
                            name: name, tooltip: tooltip, total: total, cards: cards,
                            dashboard: dashboard
                        )
                    case .elsewhere(let heading, let agents, let waiting):
                        ElsewhereCard(heading: heading, agents: agents, waiting: waiting)
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

/// One repository's worktrees: a light line with the repository's name and the disk its worktrees
/// take, then their cards, set in a little so they read as the repository's at a glance.
private struct RepositoryGroup: View {
    let name: String
    /// Where the repository is.
    let tooltip: String
    let total: String?
    let cards: [CardModel]
    let dashboard: DashboardModel

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(name)
                    .fontWeight(.semibold)
                    .lineLimit(1)
                    .truncationMode(.middle)
                if let total {
                    Text(total)
                }
            }
            .font(.callout)
            .foregroundStyle(.secondary)
            .help(tooltip)
            .padding(.horizontal, 4)
            // A little more room above the line than below, so it reads as its cards' heading.
            .padding(.top, 4)
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
            ForEach(cards, id: \.path) { card in
                CardView(model: card, dashboard: dashboard)
                    .padding(.leading, 12)
            }
        }
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
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .rotationEffect(.degrees(open ? 90 : 0))
                    .accessibilityHidden(true)
                Text(label)
                    .font(.callout)
            }
            .foregroundStyle(.secondary)
            .contentShape(.rect)
        }
        .buttonStyle(.plain)
        .help(tooltip)
        .accessibilityValue(tooltip)
        .padding(.horizontal, 4)
        .padding(.vertical, 2)
    }
}
