import SutraCore
import SwiftUI

/// The agents working outside every project, on a card of their own under its heading, a line for
/// each: a session's robot, then the folder it works in, with the subagents at work in that same
/// folder at the line's end, as on a card's tray; or the robots of subagents working in a folder no
/// session there works in, then that folder. A folder is cut in the middle when long, and whole in
/// its tooltip. The card throbs as a project's does while one of them waits on the user.
struct ElsewhereCard: View {
    let heading: String
    let lines: [ElsewhereLine]
    let waiting: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(heading)
                .font(.title3.weight(.semibold))
            // Room between the lines for robots drawn taller than the text.
            VStack(alignment: .leading, spacing: 8) {
                ForEach(Array(lines.enumerated()), id: \.offset) { _, line in
                    // The folder gives way to the robots, which it never cuts short.
                    HStack(spacing: 8) {
                        switch line {
                        case .session(let badge, let folder, let subagents):
                            AgentBadgeView(badge: badge, moves: true)
                                .fixedSize()
                            Folder(path: folder)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            if !subagents.isEmpty {
                                AgentBadges(badges: subagents, moves: false)
                                    .fixedSize()
                            }
                        case .subagents(let subagents, let folder):
                            AgentBadges(badges: subagents, moves: false)
                                .fixedSize()
                            Folder(path: folder)
                        }
                    }
                }
            }
        }
        .padding(.horizontal, Surface.contentInset)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(Surface.edgeWidth)
        .background { CardSurface(waiting: waiting) }
    }
}

/// The folder an agent works in, cut in the middle when long and whole in its tooltip.
private struct Folder: View {
    let path: String

    var body: some View {
        Text(path)
            .font(.callout)
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .truncationMode(.middle)
            .hoverTip(path)
    }
}
