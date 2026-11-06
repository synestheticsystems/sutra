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

    /// What wants the user: an agent waiting, and nothing else.
    static let amber = Hue(.systemOrange, deepened: 0.42, lifted: 0)
    /// On its way: a unit starting or building.
    static let blue = Hue(.systemBlue, deepened: 0.2, lifted: 0.15)
    /// At work, or up.
    static let green = Hue(.systemGreen, deepened: 0.45, lifted: 0)
    /// Failed.
    static let red = Hue(.systemRed, deepened: 0.25, lifted: 0.2)
}

/// Grey and still: what is at rest, or no longer news.
enum Still {
    static let ink = Color(nsColor: .secondaryLabelColor)
    static let fill = Color(nsColor: .quaternaryLabelColor)
    /// An outline at rest: a step darker than the fill, as a line covers less than a fill does.
    static let edge = Color(nsColor: .tertiaryLabelColor)
}

extension AgentLook {
    var ink: Color {
        switch self {
        case .waiting: Hue.amber.ink
        case .working: Hue.green.ink
        case .still: Still.ink
        }
    }

    /// How the capsule's fill swells: a waiting agent's with its card's throb, a working one's
    /// with its slow pulse. `nil` for an agent at rest, whose fill is `Still.fill`.
    var swell: Swell? {
        switch self {
        case .waiting: Swell(rhythm: .throb, color: Hue.amber.base, rest: 0.18, peak: 0.38)
        case .working: Swell(rhythm: .pulse, color: Hue.green.base, rest: 0.12, peak: 0.26)
        case .still: nil
        }
    }

    /// How an outlined capsule's edge swells, in the fill's rhythms but stronger, as a line covers
    /// less than a fill does. `nil` for an agent at rest, whose edge is `Still.edge`.
    var edgeSwell: Swell? {
        switch self {
        case .waiting: Swell(rhythm: .throb, color: Hue.amber.base, rest: 0.5, peak: 1)
        case .working: Swell(rhythm: .pulse, color: Hue.green.base, rest: 0.45, peak: 0.9)
        case .still: nil
        }
    }
}

extension UnitLook {
    var ink: Color {
        switch self {
        case .up: Hue.green.ink
        case .starting: Hue.blue.ink
        case .failed: Hue.red.ink
        case .still: Still.ink
        }
    }

    var fill: Color {
        switch self {
        case .up: Hue.green.fill(0.18)
        case .starting: Hue.blue.fill(0.18)
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

    /// The amber wash a waiting card's surface takes with its throb: faint, so what lies on the
    /// card still reads.
    static let wash = Swell(rhythm: .throb, color: Hue.amber.base, rest: 0, peak: 0.08)

    /// The amber a waiting card's edge turns with its throb.
    static let waitingEdge = Swell(rhythm: .throb, color: Hue.amber.base, rest: 0, peak: 1)

    /// The tray along a card's bottom edge: a shade off the card, so it reads as the card's
    /// footing rather than as more of its body.
    static let tray = Color.primary.opacity(0.04)

    /// A card's thin edge, and the track a bar fills along.
    static let edge = Color(nsColor: .separatorColor)

    static let cornerRadius: CGFloat = 10
}
