import SutraCore
import SwiftUI

/// The strip over the list while a source that alone tells of a kind of thing cannot be read, so
/// that its absence does not pass for nothing running: what Sutra cannot see, why, source by
/// source, and in the store edition the remedy, a larger folder.
struct BannerView: View {
    let dashboard: DashboardModel

    var body: some View {
        if let banner = dashboard.banner {
            VStack(spacing: 0) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .symbolRenderingMode(.multicolor)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(banner.heading)
                        ForEach(Array(banner.reasons.enumerated()), id: \.offset) { _, reason in
                            Text(reason)
                                .font(.callout)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if banner.offersFolder {
                        Button("Change\u{2026}") { dashboard.chooseFolder() }
                            .controlSize(.small)
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
                .background(Hue.amber.fill(0.08))
                Divider()
            }
        }
    }
}
