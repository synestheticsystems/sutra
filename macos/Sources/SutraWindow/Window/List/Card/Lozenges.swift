import SutraCore
import SwiftUI

/// One agent session, as `(Claude) 4m` reads: the capsule's colour says what it is doing, and a
/// waiting or working one moves with its rhythm. Its tooltip says the rest in words; a click opens
/// the session where it runs.
struct AgentLozengeView: View {
    let lozenge: AgentLozenge

    var body: some View {
        HStack(spacing: 6) {
            Lozenge(ink: lozenge.look.ink) {
                Text(lozenge.harness)
            } fill: {
                if let rhythm = lozenge.look.rhythm {
                    Rhythmic(rhythm: rhythm) { level in
                        Capsule().fill(lozenge.look.fill(at: level))
                    }
                } else {
                    Capsule().fill(lozenge.look.fill(at: 0))
                }
            }
            if let beside = lozenge.beside {
                Text(beside)
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
        .opensLink(lozenge.link)
        .accessibilityValue(lozenge.tooltip)
        .help(lozenge.tooltip)
    }
}

/// One dev-runner unit, as `(web :3000)` reads: the capsule's colour says its state. Beside it
/// stands what a unit under way or in trouble says of itself, and a sign for each announcement the
/// user switched off. A click opens the unit in the browser.
struct UnitLozengeView: View {
    let lozenge: UnitLozenge

    var body: some View {
        HStack(spacing: 6) {
            Lozenge(ink: lozenge.look.ink) {
                HStack(spacing: 3) {
                    Text(lozenge.name)
                    if let port = lozenge.port {
                        Text(port).monospaced()
                    }
                }
            } fill: {
                Capsule().fill(lozenge.look.fill)
            }
            if let beside = lozenge.beside {
                Text(beside)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            if lozenge.muted {
                SwitchedOff(symbol: "speaker.slash", meaning: "Muted")
            }
            if lozenge.notificationsOff {
                SwitchedOff(symbol: "bell.slash", meaning: "Notifications off")
            }
        }
        .accessibilityElement(children: .combine)
        .opensLink(lozenge.link)
        .accessibilityValue(lozenge.tooltip)
        .help(lozenge.tooltip)
    }
}

/// A small sign beside a unit for an announcement the user switched off for it.
private struct SwitchedOff: View {
    let symbol: String
    /// What the sign says, for VoiceOver.
    let meaning: String

    var body: some View {
        Image(systemName: symbol)
            .font(.caption)
            .foregroundStyle(.secondary)
            .accessibilityLabel(meaning)
    }
}

/// A lozenge's shape: `label` on a capsule painted by `fill`.
private struct Lozenge<Label: View, Fill: View>: View {
    let ink: Color
    @ViewBuilder let label: () -> Label
    @ViewBuilder let fill: () -> Fill

    var body: some View {
        label()
            .font(.callout.weight(.medium))
            .foregroundStyle(ink)
            .lineLimit(1)
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .background(content: fill)
    }
}

extension View {
    /// A click opens `link` when there is one, and the pointer shows that it will.
    @ViewBuilder
    func opensLink(_ link: String?) -> some View {
        if let link {
            Button { Native.open(link: link) } label: { self }
                .buttonStyle(.plain)
                .linkPointer()
        } else {
            self
        }
    }

    /// The pointing hand over what opens something, on the macOS versions that let SwiftUI set
    /// the pointer.
    @ViewBuilder
    func linkPointer() -> some View {
        if #available(macOS 15, *) {
            pointerStyle(.link)
        } else {
            self
        }
    }
}
