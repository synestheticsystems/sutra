import SutraCore
import SwiftUI

/// The subagents at work in a folder, a small round badge each, side by side in the core's order:
/// at the end of a card's tray, or of a line of the agents working elsewhere.
struct SubagentIcons: View {
    let icons: [SubagentIcon]

    var body: some View {
        HStack(spacing: 4) {
            ForEach(Array(icons.enumerated()), id: \.offset) { _, icon in
                SubagentIconView(icon: icon)
            }
        }
    }
}

/// One subagent at work: its harness's symbol on a circle in its look's colours, the colours of
/// its session's lozenge, held still, as the tray it stands on is quiet. Round where a session's
/// lozenge is long, it reads as the session's helper rather than as another session. Its tip
/// names it; a click opens the session it works for.
private struct SubagentIconView: View {
    let icon: SubagentIcon

    /// The circle's width. It stands a little taller than the line it sits on, spilling into the
    /// tray's margin, so a tray with subagents is no taller than one without.
    private static let diameter: CGFloat = 17
    private static let lineHeight: CGFloat = 14

    var body: some View {
        Image(systemName: icon.harness.symbol)
            .font(.system(size: 8.5, weight: .bold))
            .foregroundStyle(icon.look.ink)
            .frame(width: Self.diameter, height: Self.lineHeight)
            .background {
                Circle()
                    .fill(icon.look.stillFill)
                    .frame(width: Self.diameter, height: Self.diameter)
            }
            .accessibilityLabel(icon.tooltip)
            .opensLink(icon.link)
            .hoverTip(icon.tooltip)
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

extension AgentLook {
    /// The fill a lozenge of this look shows when nothing moves.
    fileprivate var stillFill: Color {
        if let swell {
            Color(nsColor: swell.color).opacity(swell.steady)
        } else {
            Still.fill
        }
    }
}
