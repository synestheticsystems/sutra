import AppKit
import SwiftUI

/// A colour that swells and fades with a rhythm: so much of `color` shows at the rhythm's low, so
/// much at its peak.
struct Swell: Equatable {
    let rhythm: Rhythm
    let color: NSColor
    let rest: Double
    let peak: Double

    /// How much of the colour shows while nothing moves.
    var steady: Double {
        rest + (peak - rest) * rhythm.steadyLevel
    }
}

/// A swell filling its frame, or edging it, in a layer of its own that Core Animation moves, so
/// the window's views do no work while it moves. A swell SwiftUI redrew frame by frame would lay
/// the list out several times a second, and each layout restarts the wait before AppKit shows a
/// tooltip, so no tooltip on the list would ever show.
struct SwellLayer: NSViewRepresentable {
    let swell: Swell
    let shape: SwellShape

    func makeNSView(context: Context) -> SwellView {
        SwellView()
    }

    func updateNSView(_ view: SwellView, context: Context) {
        view.show(swell, as: shape, motion: context.environment.motion)
    }
}

/// The shapes a swell takes.
enum SwellShape: Equatable {
    /// A lozenge's capsule, its ends as round as its height allows.
    case capsule
    case rectangle(cornerRadius: CGFloat)
    /// A line `width` wide just inside the frame's edge, as a card's edge is drawn.
    case edge(cornerRadius: CGFloat, width: CGFloat)
}

/// The view under a `SwellLayer`: an empty view whose one sublayer carries the swell.
final class SwellView: NSView {
    private let swellLayer = CALayer()
    private var swell: Swell?
    private var shape = SwellShape.capsule
    private var motion = Motion.steady

    private static let animationKey = "swell"

    init() {
        super.init(frame: .zero)
        wantsLayer = true
        layer?.addSublayer(swellLayer)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("made in code only")
    }

    /// It lies under what the swell colours, and every click and tooltip is for that.
    override func hitTest(_ point: NSPoint) -> NSView? {
        nil
    }

    func show(_ swell: Swell, as shape: SwellShape, motion: Motion) {
        let looksChanged = swell != self.swell || shape != self.shape
        let moves = looksChanged || motion != self.motion
        self.swell = swell
        self.shape = shape
        self.motion = motion
        if looksChanged {
            paint()
        }
        if moves {
            move()
        }
    }

    override func setFrameSize(_ newSize: NSSize) {
        super.setFrameSize(newSize)
        needsLayout = true
    }

    override func layout() {
        super.layout()
        withoutImplicitAnimation {
            swellLayer.frame = bounds
            swellLayer.cornerRadius = cornerRadius
        }
    }

    /// The system colours resolve differently in light and dark mode.
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        paint()
    }

    /// A layer leaving its window loses its animations, and one in no window has none to run.
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        move()
    }

    private var cornerRadius: CGFloat {
        switch shape {
        case .capsule: min(bounds.width, bounds.height) / 2
        case .rectangle(let radius), .edge(let radius, _): radius
        }
    }

    private func paint() {
        guard let swell else { return }
        var color: CGColor?
        effectiveAppearance.performAsCurrentDrawingAppearance {
            color = swell.color.cgColor
        }
        withoutImplicitAnimation {
            switch shape {
            case .capsule, .rectangle:
                swellLayer.backgroundColor = color
                swellLayer.borderWidth = 0
            case .edge(_, let width):
                swellLayer.backgroundColor = nil
                swellLayer.borderColor = color
                swellLayer.borderWidth = width
            }
            swellLayer.cornerRadius = cornerRadius
        }
        needsLayout = true
    }

    /// Hold the steady level, and run the rhythm over it while motion is live and a window shows
    /// the swell.
    private func move() {
        guard let swell else { return }
        withoutImplicitAnimation {
            swellLayer.opacity = Float(swell.steady)
        }
        swellLayer.removeAnimation(forKey: Self.animationKey)
        if motion == .live, window != nil {
            swellLayer.add(swell.rhythm.animation(from: swell.rest, to: swell.peak), forKey: Self.animationKey)
        }
    }

    private func withoutImplicitAnimation(_ change: () -> Void) {
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        change()
        CATransaction.commit()
    }
}
