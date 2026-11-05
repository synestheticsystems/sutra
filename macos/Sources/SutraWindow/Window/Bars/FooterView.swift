import SutraCore
import SwiftUI

/// What shows under the list while there is something to say: an alert sign whose tooltip names
/// the sources whose failure loses only some folders or some changes, the warning that the scanner
/// stopped, and a thin bar along the bottom while the first sizing runs.
struct FooterView: View {
    let dashboard: DashboardModel

    var body: some View {
        if let footer = dashboard.footer {
            VStack(spacing: 0) {
                if !footer.problems.isEmpty || footer.stopped != nil {
                    Divider()
                    HStack(spacing: 8) {
                        if !footer.problems.isEmpty {
                            let problems = footer.problems.joined(separator: "\n")
                            Image(systemName: "exclamationmark.triangle.fill")
                                .symbolRenderingMode(.multicolor)
                                .help(problems)
                                .accessibilityLabel(problems)
                        }
                        if let stopped = footer.stopped {
                            Text(stopped)
                                .font(.callout)
                                .foregroundStyle(Hue.red.ink)
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 6)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                if let sizing = footer.sizing {
                    // How far the first sizing has come, along the window's bottom edge.
                    FillBar(progress: sizing, fill: .secondary, thickness: 2)
                        .accessibilityHidden(true)
                }
            }
        }
    }
}
