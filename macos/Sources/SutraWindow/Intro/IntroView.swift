import SwiftUI

/// A first-launch welcome, also reusable by the standalone artwork preview.
public struct IntroView: View {
    private let continueAction: () -> Void

    public init(continueAction: @escaping () -> Void) {
        self.continueAction = continueAction
    }

    public var body: some View {
        GeometryReader { geometry in
            ScrollView {
                VStack(spacing: 0) {
                    Text("Robo Sutra")
                        .font(.system(size: 36, weight: .semibold, design: .rounded))
                        .tracking(-1.4)
                        .padding(.top, 18)
                        .accessibilityAddTraits(.isHeader)
                    Text("A little coordination.")
                        .font(.title3)
                        .foregroundStyle(.secondary)
                        .padding(.top, 3)
                    PortDance(showsCue: true)
                        .frame(height: max(280, geometry.size.height - 190))
                        .padding(.top, 8)
                    Text("Your agents, dev servers and worktrees.\nAll in one place.")
                        .font(.body)
                        .foregroundStyle(.secondary)
                        .lineSpacing(3)
                        .padding(.horizontal, 24)
                        .padding(.top, 12)
                    Button("Let’s untangle this", action: continueAction)
                        .buttonStyle(.borderedProminent)
                        .controlSize(.large)
                        .keyboardShortcut(.defaultAction)
                        .padding(.top, 12)
                        .padding(.bottom, 18)
                }
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
            }
        }
        .frame(minWidth: 460, minHeight: 300)
    }
}
