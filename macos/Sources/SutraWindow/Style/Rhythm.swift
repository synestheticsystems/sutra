import QuartzCore
import SwiftUI

/// What moves on the list: a waiting card throbs, and its waiting agents with it; an agent at work
/// pulses slowly. Core Animation runs every rhythm, so the window's views do no work while it
/// moves, and on the wall clock, so whatever throbs throbs in step, however late it appeared.
enum Rhythm {
    case throb
    case pulse

    /// One cycle, plain to its peak and back.
    private var period: TimeInterval {
        switch self {
        case .throb: 1.6
        // Slow enough to read as alive rather than as an alert.
        case .pulse: 3.2
        }
    }

    /// The level held while nothing moves, from 0 to 1: a waiting card stays at its peak, so it
    /// still stands out; a working agent rests halfway.
    var steadyLevel: Double {
        switch self {
        case .throb: 1
        case .pulse: 0.5
        }
    }

    /// An animation of a layer's opacity from `rest` up to `peak` and back, easing in and out as a
    /// cosine does, forever, at the point of its cycle the wall clock gives.
    func animation(from rest: Double, to peak: Double) -> CABasicAnimation {
        let animation = CABasicAnimation(keyPath: "opacity")
        animation.fromValue = rest
        animation.toValue = peak
        animation.duration = period / 2
        animation.autoreverses = true
        animation.repeatCount = .infinity
        // The sine-shaped ease, which makes each half of a cycle half a cosine.
        animation.timingFunction = CAMediaTimingFunction(controlPoints: 0.37, 0, 0.63, 1)
        animation.timeOffset = Date.timeIntervalSinceReferenceDate
            .truncatingRemainder(dividingBy: period)
        return animation
    }
}

/// Whether the rhythms run, or hold their steady levels: while macOS is set to reduce motion, and
/// while nobody can see the window.
public enum Motion: Sendable {
    case live
    case steady
}

extension EnvironmentValues {
    @Entry public var motion: Motion = .live
}
