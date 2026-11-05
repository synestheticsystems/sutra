import AppKit
import SutraCore
import SwiftUI

/// A state's colour: the system colour, and its ink, the shade text takes on a pale fill of it.
/// The system colours are made to fill and to mark; as small text on a pale fill they read faintly
/// in light mode, so the ink is deepened there, and in dark mode lifted a little.
struct Hue {
    let base: NSColor
    let ink: Color

    init(_ base: NSColor, deepened: CGFloat, lifted: CGFloat) {
        self.base = base
        ink = Color(nsColor: NSColor(name: nil) { appearance in
            var shade = base
            appearance.performAsCurrentDrawingAppearance {
                shade = base.usingColorSpace(.sRGB) ?? base
            }
            let dark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            return (dark
                ? shade.blended(withFraction: lifted, of: .white)
                : shade.blended(withFraction: deepened, of: .black)) ?? shade
        })
    }

    /// `amount` of the colour over what lies behind it.
    func fill(_ amount: Double) -> Color {
        Color(nsColor: base).opacity(amount)
    }

    /// What wants the user: an agent waiting, a unit on its way.
    static let amber = Hue(.systemOrange, deepened: 0.42, lifted: 0)
    /// At work, or up.
    static let green = Hue(.systemGreen, deepened: 0.45, lifted: 0)
    /// Failed.
    static let red = Hue(.systemRed, deepened: 0.25, lifted: 0.2)
}

/// Grey and still: what is at rest, or no longer news.
private enum Still {
    static let ink = Color(nsColor: .secondaryLabelColor)
    static let fill = Color(nsColor: .quaternaryLabelColor)
}

extension AgentLook {
    var ink: Color {
        switch self {
        case .waiting: Hue.amber.ink
        case .working: Hue.green.ink
        case .still: Still.ink
        }
    }

    /// The capsule's fill at `level` of the lozenge's rhythm, from 0 to 1: a waiting agent's
    /// swells with its card's throb, a working one's with its slow pulse.
    func fill(at level: Double) -> Color {
        switch self {
        case .waiting: Hue.amber.fill(0.18 + 0.20 * level)
        case .working: Hue.green.fill(0.12 + 0.14 * level)
        case .still: Still.fill
        }
    }

    /// What moves the lozenge, if anything does.
    var rhythm: Rhythm? {
        switch self {
        case .waiting: .throb
        case .working: .pulse
        case .still: nil
        }
    }
}

extension UnitLook {
    var ink: Color {
        switch self {
        case .up: Hue.green.ink
        case .starting: Hue.amber.ink
        case .failed: Hue.red.ink
        case .still: Still.ink
        }
    }

    var fill: Color {
        switch self {
        case .up: Hue.green.fill(0.18)
        case .starting: Hue.amber.fill(0.22)
        case .failed: Hue.red.fill(0.18)
        case .still: Still.fill
        }
    }
}

/// The surfaces a card is made of.
enum Surface {
    /// Raised over the window: white in light mode; in dark mode a step lighter than the window,
    /// as raised things are.
    static let card = Color(nsColor: NSColor(name: nil) { appearance in
        var surface = NSColor.controlBackgroundColor
        appearance.performAsCurrentDrawingAppearance {
            if appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua {
                let window = NSColor.windowBackgroundColor.usingColorSpace(.sRGB)
                surface = window?.blended(withFraction: 0.06, of: .white) ?? surface
            }
        }
        return surface
    })

    /// The amber a waiting card's surface takes at `level` of its throb, from 0 to 1: faint, so
    /// what lies on the card still reads.
    static func wash(at level: Double) -> Color {
        Hue.amber.fill(0.08 * level)
    }

    /// The tray along a card's bottom edge: a shade off the card, so it reads as the card's
    /// footing rather than as more of its body.
    static let tray = Color.primary.opacity(0.04)

    /// A card's thin edge, and the track a bar fills along.
    static let edge = Color(nsColor: .separatorColor)

    static let cornerRadius: CGFloat = 10
}
