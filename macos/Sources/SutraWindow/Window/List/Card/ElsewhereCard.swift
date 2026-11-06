import SutraCore
import SwiftUI

/// The agents working outside every project, on a card of their own under its heading: a line
/// each, the agent's lozenge and then the folder it works in, cut in the middle when long and whole
/// in its tooltip. The card throbs as a project's does while one of them waits on the user.
struct ElsewhereCard: View {
    let heading: String
    let agents: [ElsewhereAgent]
    let waiting: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(heading)
                .font(.title3.weight(.semibold))
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(agents.enumerated()), id: \.offset) { _, agent in
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        // The folder gives way to the lozenge, which it never cuts short.
                        AgentLozengeView(lozenge: agent.lozenge)
                            .fixedSize()
                        Text(agent.folder)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .help(agent.folder)
                    }
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(1)
        .background { CardSurface(waiting: waiting) }
    }
}
