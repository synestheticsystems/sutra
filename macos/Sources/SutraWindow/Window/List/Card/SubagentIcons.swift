import SutraCore
import SwiftUI

/// The subagents at work in a folder, a small sign each, side by side in the core's order: at the
/// end of a card's tray, or of a line of the agents working elsewhere.
struct SubagentIcons: View {
    let icons: [SubagentIcon]

    var body: some View {
        HStack(spacing: 6) {
            ForEach(Array(icons.enumerated()), id: \.offset) { _, icon in
                SubagentIconView(icon: icon)
            }
        }
    }
}

/// One subagent at work: its harness's symbol in its look's colour, held still, as the tray it
/// stands on is quiet. Its tooltip names it; a click opens the session it works for.
private struct SubagentIconView: View {
    let icon: SubagentIcon

    var body: some View {
        Image(systemName: icon.harness.symbol)
            // The lozenges' weight, so the code brackets' strokes read as firmly as the filled
            // sparkle beside them.
            .font(.callout.weight(.medium))
            .foregroundStyle(icon.look.ink)
            .accessibilityLabel(icon.tooltip)
            .opensLink(icon.link)
            .help(icon.tooltip)
    }
}

extension AgentHarness {
    /// An SF Symbol for the harness, never its maker's logo, which is not Sutra's to draw.
    fileprivate var symbol: String {
        switch self {
        case .claudeCode: "sparkle"
        case .codex: "chevron.left.forwardslash.chevron.right"
        }
    }
}
