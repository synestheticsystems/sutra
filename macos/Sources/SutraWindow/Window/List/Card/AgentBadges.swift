import SutraCore
import SwiftUI

/// Agents as a row of robots, side by side in the core's order: a card's sessions or its subagents
/// on its tray, or the subagents of a line of the agents working elsewhere.
struct AgentBadges: View {
    let badges: [AgentBadge]
    /// Whether the robots' faces move with their rhythm: a session's do, a subagent's hold still.
    let moves: Bool

    var body: some View {
        HStack(spacing: 4) {
            ForEach(Array(badges.enumerated()), id: \.offset) { _, badge in
                AgentBadgeView(badge: badge, moves: moves)
            }
        }
    }
}

/// One agent's robot: its tip says in words what the robot shows, and who the agent is, and a click
/// opens its session.
struct AgentBadgeView: View {
    let badge: AgentBadge
    let moves: Bool

    var body: some View {
        RobotView(robot: badge.robot.figure, look: badge.look, moves: moves)
            .accessibilityElement()
            .accessibilityLabel(badge.tooltip)
            .opensLink(badge.link)
            .hoverTip(badge.tooltip)
    }
}
