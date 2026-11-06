import SutraCore
import SwiftUI

/// The strip over the list once the map no longer updates: the list below stays, as how things
/// last stood, and this says so, and what mends it.
struct StoppedLine: View {
    let dashboard: DashboardModel

    var body: some View {
        if let stopped = dashboard.stopped {
            VStack(spacing: 0) {
                Label(stopped, systemImage: "exclamationmark.octagon.fill")
                    .foregroundStyle(Hue.red.ink)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(Hue.red.fill(0.08))
                Divider()
            }
        }
    }
}
