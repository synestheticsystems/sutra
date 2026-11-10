import SutraCore
import SwiftUI

/// The list, or what stands in its place: the store edition's request for a folder, the first
/// scan under way, or why there is nothing to list. The core holds the cards in place while the
/// pointer is over the list, so the list tells it when the pointer comes and goes.
struct ListView: View {
    let dashboard: DashboardModel

    /// The room between the list's rows: between cards, and between a line and its first row.
    static let rowSpacing: CGFloat = 8

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
            LazyVStack(alignment: .leading, spacing: ListView.rowSpacing) {
                ForEach(rows) { row in
                    switch row {
                    case .line(let line, let place):
                        LineRow(line: line, dashboard: dashboard)
                            .hanging(at: place)
                    case .card(let card, let place):
                        CardView(model: card, dashboard: dashboard)
                            .hanging(at: place)
                    case .container(let card, let place):
                        ContainerCardView(card: card)
                            .hanging(at: place)
                    case .dockerDisk(let tray, let location, let place):
                        DockerDiskRow(tray: tray, location: location)
                            .hanging(at: place)
                    case .elsewhere(let heading, let lines, let waiting, let place):
                        ElsewhereCard(heading: heading, lines: lines, waiting: waiting)
                            .hanging(at: place)
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

/// The lines of the list that fold something away, a group's, a checkout's and the one standing for
/// the quiet groups, all laid out alike, so the chevrons of lines at one depth stand in one column.
enum FoldingLine {
    /// How far a line's chevron stands in from where its row begins.
    static let inset: CGFloat = 8
    /// The room above and below a line's text, which a folded line's throb fills.
    static let padding: CGFloat = 7
    /// The width of the chevron's slot, fixed so the guide under an open line's chevron runs down
    /// its middle.
    static let chevronWidth: CGFloat = 10
}

/// A folding line's chevron, pointing at what the line holds while it shows.
struct Chevron: View {
    let open: Bool

    var body: some View {
        Image(systemName: "chevron.right")
            .font(.caption.weight(.semibold))
            .rotationEffect(.degrees(open ? 90 : 0))
            .frame(width: FoldingLine.chevronWidth)
            .accessibilityHidden(true)
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
