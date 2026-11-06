import SwiftUI

/// The integration help, in the list's place while it is open: the `~/.dev-runner/` contract a dev
/// script satisfies to show up in Sutra, and a prompt that hands that job to a coding agent. Its
/// words are the window's own. They are set verbatim, as Markdown would take `*_PORT` for markup.
struct HelpPanel: View {
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 10) {
                Text(verbatim: "Integration")
                    .font(.title3.weight(.semibold))
                Text(verbatim: "Sutra watches ~/.dev-runner/. Any dev script that writes the right files gets a live status row. No daemon, no IPC.")
                    .foregroundStyle(.secondary)
                // Each file's path comes first, as the literal it is, so the eye lands on where
                // before what and when.
                ContractPart(heading: "Meta file") {
                    Literal("~/.dev-runner/<id>")
                    Prose("KEY=VALUE per line. Required: DIR, PID. Optional: STARTED, *_PORT.")
                    Prose("Write once at startup. Delete on exit.")
                }
                ContractPart(heading: "Status file") {
                    Literal("~/.dev-runner/<id>.<unit>.status")
                    Literal("Single line: <state>[: <detail>]")
                    Prose("States: starting, building, running, ready, failed, stopped.")
                    Prose("Overwrite (don't append) on each transition. The detail is freeform.")
                }
                // The contract reads as facts; what follows is the call to action.
                Divider()
                Text(verbatim: "Hand it to an agent")
                    .font(.headline)
                Text(verbatim: "Paste this into Claude Code, Cursor, or any code agent. It'll fetch the full guide and wire your dev script up.")
                    .foregroundStyle(.secondary)
                AgentPrompt()
                // One paragraph, so the long link wraps as text does; the arrow says it leaves
                // the app.
                Text("\(Self.readOnGitHub)\u{00a0}\(Image(systemName: "arrow.up.right"))")
                    .font(.callout)
                    .foregroundStyle(Color.accentColor)
            }
            .padding(.horizontal, 16)
            .padding(.top, 8)
            .padding(.bottom, 16)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    /// The integration guide, for a person to read on GitHub.
    private static let guide = URL(
        string: "https://github.com/synestheticsystems/sutra/blob/main/docs/INTEGRATION.md"
    )!

    /// `Read on GitHub: ` and the guide's address, which opens it.
    private static var readOnGitHub: AttributedString {
        var lead = AttributedString("Read on GitHub: ")
        lead.foregroundColor = .secondary
        var link = AttributedString(guide.absoluteString)
        link.link = guide
        return lead + link
    }
}

/// The prompt to paste into a coding agent, selectable so the user can take part of it, with a
/// Copy button that answers Copied for a moment.
private struct AgentPrompt: View {
    @Environment(\.native) private var native
    @State private var copied = false

    /// The URL pins to `main` rather than a release tag because the guide is a living reference:
    /// a shipped app should point at the latest patterns, not a frozen snapshot.
    private static let prompt = """
        Make my project's dev script sutra-compatible by following:
        https://raw.githubusercontent.com/synestheticsystems/sutra/main/docs/INTEGRATION.md

        If a dev script already exists, update it. Otherwise create one.

        """

    var body: some View {
        VStack(alignment: .trailing, spacing: 8) {
            Text(verbatim: Self.prompt.trimmingCharacters(in: .newlines))
                .font(.callout)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button {
                native.copy(Self.prompt)
                copied = true
            } label: {
                Label {
                    Text(verbatim: copied ? "Copied" : "Copy")
                } icon: {
                    Image(systemName: copied ? "checkmark" : "doc.on.doc")
                }
                .foregroundStyle(copied ? Hue.green.ink : Color.accentColor)
            }
            .buttonStyle(.plain)
            .font(.callout)
        }
        .padding(10)
        .background(Color(nsColor: .textBackgroundColor), in: .rect(cornerRadius: 6))
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Surface.edge))
        .task(id: copied) {
            guard copied else { return }
            // Cancelled only as the panel closes, which takes the flash with it either way.
            try? await Task.sleep(for: .seconds(1.6))
            copied = false
        }
    }
}

/// A part of the contract: a heading, then its lines.
private struct ContractPart<Lines: View>: View {
    let heading: String
    @ViewBuilder let lines: () -> Lines

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(verbatim: heading)
                .font(.headline)
                .padding(.bottom, 2)
            lines()
        }
    }
}

/// A path or a format, set as the literal it is.
private struct Literal: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(verbatim: text)
            .font(.callout.monospaced())
            .textSelection(.enabled)
    }
}

/// A line of the contract in words.
private struct Prose: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        Text(verbatim: text)
            .font(.callout)
            .foregroundStyle(.secondary)
    }
}
