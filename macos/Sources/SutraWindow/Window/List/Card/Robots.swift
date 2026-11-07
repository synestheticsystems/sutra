import AppKit
import SutraCore
import SwiftUI

/// One agent, as one of Sutra's robots: which AI it is shows in the robot, and what it is doing in
/// the light of its face, behind the visor. Green is at work, amber is waiting on the user, and a
/// robot at rest dims, its face unlit. A session's face pulses with its rhythm, while a subagent's
/// holds still, as the tray it stands on is quiet.
struct RobotView: View {
    let robot: RobotFigure
    let look: AgentLook
    /// The face moves with the look's rhythm: a session's does, a subagent's does not.
    let moves: Bool

    var body: some View {
        let size = CGSize(width: RobotFigure.height * RobotFigure.aspect, height: RobotFigure.height)
        let face = robot.face(in: CGRect(origin: .zero, size: size))
        ZStack {
            Canvas { context, size in
                robot.drawBody(in: &context, size: size)
            }
            if moves, let swell = look.faceSwell {
                SwellLayer(swell: swell, shape: .figure(face))
            } else {
                Path(face).fill(look.faceLight)
            }
        }
        .frame(width: size.width, height: size.height)
        // At rest, the robot steps back.
        .opacity(look == .still ? Still.opacity : 1)
        // Drawn taller than the line it stands on, into the tray's margins, so a tray with robots
        // is as tall as one without and no card moves when an agent comes or goes.
        .frame(height: RobotFigure.lineHeight)
    }
}

/// The robots of Sutra's art: a pale helmet with ear muffs and a thick dark outline, and a black
/// visor with the face of the AI on it.
struct RobotFigure: Equatable {
    /// The robot's height as drawn: nearly the height of a card's tray, so it reads at a glance.
    static let height: CGFloat = 21
    /// The height it takes on its line, the height of the tray's text.
    static let lineHeight: CGFloat = 15
    static let aspect: CGFloat = 1.2

    let head: Color
    let ears: Color
    let face: Face

    enum Face: Equatable {
        /// Two asterisks for eyes.
        case asterisks
        /// The prompt, `>_`.
        case prompt
        /// Two four-point sparkles.
        case sparkles
        /// Two round eyes, for an AI with no robot of its own.
        case eyes
    }

    static let claude = RobotFigure(head: .rgb(0xFBEEE4), ears: .rgb(0xFBEEE4), face: .asterisks)
    static let codex = RobotFigure(head: .rgb(0xEAE4FA), ears: .rgb(0xB7A3F2), face: .prompt)
    static let gemini = RobotFigure(head: .rgb(0xDCEAFB), ears: .rgb(0x62AEF5), face: .sparkles)
    static let plain = RobotFigure(head: .rgb(0xEBEBED), ears: .rgb(0xB5B5BA), face: .eyes)

    private static let outline = Color.rgb(0x2B2B2E)
    private static let visor = Color.rgb(0x1C1C1E)

    /// The ears, the head and the visor, each outlined, in a frame `size` big. The drawing is in
    /// units of the frame's height, after the proportions of the art, simplified to read when
    /// small: a thick outline, a visor across most of the face, short ear muffs.
    func drawBody(in context: inout GraphicsContext, size: CGSize) {
        let unit = size.height
        let line = 0.085 * unit
        let place = Placement(unit: unit)
        let ears = place.roundedRect(x: 0.045, y: 0.36, width: 1.11, height: 0.34, radius: 0.12)
        context.fill(ears, with: .color(self.ears))
        context.stroke(ears, with: .color(Self.outline), lineWidth: line)
        let head = place.roundedRect(x: 0.12, y: 0.045, width: 0.96, height: 0.91, radius: 0.27)
        context.fill(head, with: .color(self.head))
        context.stroke(head, with: .color(Self.outline), lineWidth: line)
        let visor = place.roundedRect(x: 0.21, y: 0.29, width: 0.78, height: 0.46, radius: 0.17)
        context.fill(visor, with: .color(Self.visor))
    }

    /// The face on the visor, as one filled figure in a frame `bounds` big.
    func face(in bounds: CGRect) -> CGPath {
        let place = Placement(unit: bounds.height)
        let path = CGMutablePath()
        switch face {
        case .asterisks:
            for x in [0.415, 0.785] {
                path.addPath(place.asterisk(x: x, y: 0.52, radius: 0.135, width: 0.065))
            }
        case .prompt:
            path.addPath(place.chevron(x: 0.33, y: 0.52, size: 0.14, width: 0.085))
            path.addPath(place.roundedRect(x: 0.56, y: 0.6, width: 0.3, height: 0.07, radius: 0.035).cgPath)
        case .sparkles:
            path.addPath(place.sparkle(x: 0.43, y: 0.55, radius: 0.16))
            path.addPath(place.sparkle(x: 0.78, y: 0.45, radius: 0.12))
        case .eyes:
            for x in [0.415, 0.785] {
                path.addPath(place.circle(x: x, y: 0.52, radius: 0.085))
            }
        }
        return path
    }
}

/// Lays the robot out in units of its height.
private struct Placement {
    let unit: CGFloat

    func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint {
        CGPoint(x: x * unit, y: y * unit)
    }

    func roundedRect(x: CGFloat, y: CGFloat, width: CGFloat, height: CGFloat, radius: CGFloat) -> Path {
        Path(
            roundedRect: CGRect(origin: point(x, y), size: CGSize(width: width * unit, height: height * unit)),
            cornerRadius: radius * unit,
            style: .continuous
        )
    }

    func circle(x: CGFloat, y: CGFloat, radius: CGFloat) -> CGPath {
        CGPath(ellipseIn: CGRect(origin: point(x - radius, y - radius), size: CGSize(width: 2 * radius * unit, height: 2 * radius * unit)), transform: nil)
    }

    /// Eight rays from a centre, stroked round and given as their outline, so the figure fills.
    func asterisk(x: CGFloat, y: CGFloat, radius: CGFloat, width: CGFloat) -> CGPath {
        let rays = CGMutablePath()
        for ray in 0..<4 {
            let angle = CGFloat(ray) * .pi / 4
            let reach = CGPoint(x: cos(angle) * radius, y: sin(angle) * radius)
            rays.move(to: point(x - reach.x, y - reach.y))
            rays.addLine(to: point(x + reach.x, y + reach.y))
        }
        return rays.copy(strokingWithWidth: width * unit, lineCap: .round, lineJoin: .round, miterLimit: 1)
    }

    /// `>`, its point at `x + size`, given as its outline.
    func chevron(x: CGFloat, y: CGFloat, size: CGFloat, width: CGFloat) -> CGPath {
        let stroke = CGMutablePath()
        stroke.move(to: point(x, y - size * 0.85))
        stroke.addLine(to: point(x + size, y))
        stroke.addLine(to: point(x, y + size * 0.85))
        return stroke.copy(strokingWithWidth: width * unit, lineCap: .round, lineJoin: .round, miterLimit: 1)
    }

    /// A four-point star with curved sides.
    func sparkle(x: CGFloat, y: CGFloat, radius: CGFloat) -> CGPath {
        let star = CGMutablePath()
        let pinch = radius * 0.16
        star.move(to: point(x, y - radius))
        star.addQuadCurve(to: point(x + radius, y), control: point(x + pinch, y - pinch))
        star.addQuadCurve(to: point(x, y + radius), control: point(x + pinch, y + pinch))
        star.addQuadCurve(to: point(x - radius, y), control: point(x - pinch, y + pinch))
        star.addQuadCurve(to: point(x, y - radius), control: point(x - pinch, y - pinch))
        star.closeSubpath()
        return star
    }
}

extension Robot {
    /// The drawing of the robot the core picked.
    var figure: RobotFigure {
        switch self {
        case .claude: .claude
        case .codex: .codex
        case .gemini: .gemini
        case .plain: .plain
        }
    }
}

extension Color {
    /// A colour of the robots, as `0xFBEEE4` writes it: the same in light and dark mode, as an
    /// illustration's colours are.
    fileprivate static func rgb(_ hex: UInt32) -> Color {
        Color(
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}
