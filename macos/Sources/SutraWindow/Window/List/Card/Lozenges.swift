import SutraCore
import SwiftUI

/// One dev-runner unit, as `(web :3000)` reads: the capsule's colour says its state. Beside it
/// stands what a unit under way or in trouble says of itself, and a sign for each announcement the
/// user switched off. A click opens the unit in the browser.
struct UnitLozengeView: View {
    let lozenge: UnitLozenge

    var body: some View {
        HStack(spacing: 6) {
            Lozenge(ink: lozenge.look.ink, fill: lozenge.look.fill) {
                HStack(spacing: 3) {
                    Text(lozenge.name)
                    if let port = lozenge.port {
                        Text(port).monospaced()
                    }
                }
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
        .hoverTip(lozenge.tooltip)
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

/// A lozenge's shape: `label` in `ink` on a capsule of `fill`.
private struct Lozenge<Label: View>: View {
    let ink: Color
    let fill: Color
    @ViewBuilder let label: () -> Label

    var body: some View {
        label()
            .font(.callout.weight(.medium))
            .foregroundStyle(ink)
            .lineLimit(1)
            .padding(.horizontal, 8)
            .padding(.vertical, 2)
            .background(Capsule().fill(fill))
    }
}

extension View {
    /// A click opens `link` when there is one, and the pointer shows that it will.
    func opensLink(_ link: String?) -> some View {
        modifier(OpensLink(link: link))
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

/// What `opensLink` makes of a view: a modifier, as only a view or a modifier reads the services
/// the window reaches from the environment.
private struct OpensLink: ViewModifier {
    let link: String?

    @Environment(\.native) private var native

    func body(content: Content) -> some View {
        if let link {
            Button { native.open(link: link) } label: { content }
                .buttonStyle(.plain)
                .linkPointer()
        } else {
            content
        }
    }
}
