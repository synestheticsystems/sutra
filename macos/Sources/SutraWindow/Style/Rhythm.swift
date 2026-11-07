import QuartzCore
import SwiftUI

/// What moves on the list: a waiting card throbs, and its waiting agents with it; an agent at work
/// pulses slowly. Core Animation runs every rhythm, so the window's views do no work while it
/// moves, and on the wall clock, so whatever throbs throbs in step, however late it appeared.
enum Rhythm {
    case throb
    case pulse

    /// One cycle, plain to its peak and back.
    var period: TimeInterval {
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

    /// The level at `moment` on the rhythm's clock, from 0 to 1, as its animation draws it then:
    /// at rest as each cycle starts, at the peak halfway through.
    func level(at moment: TimeInterval) -> Double {
        let half = period / 2
        let phase = moment - (moment / period).rounded(.down) * period
        return Self.ease.progress(at: phase < half ? phase / half : (period - phase) / half)
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
        animation.timingFunction = Self.ease.timingFunction
        animation.timeOffset = Date.timeIntervalSinceReferenceDate
            .truncatingRemainder(dividingBy: period)
        return animation
    }

    /// The sine-shaped ease, which makes each half of a cycle half a cosine.
    private static let ease = Ease(x1: 0.37, y1: 0, x2: 0.63, y2: 1)
}

/// An ease as Core Animation's timing functions draw one: a cubic Bézier curve from (0, 0) to
/// (1, 1) through two control points, which gives how far a change has gone at each point of its
/// time. A rhythm's animation hands it to Core Animation, and a recording drawing frame by frame
/// reads it here, so both draw one curve.
private struct Ease {
    let x1: Double
    let y1: Double
    let x2: Double
    let y2: Double

    var timingFunction: CAMediaTimingFunction {
        CAMediaTimingFunction(controlPoints: Float(x1), Float(y1), Float(x2), Float(y2))
    }

    /// How far the change has gone, from 0 to 1, once `time`, a fraction of its duration, has
    /// passed.
    func progress(at time: Double) -> Double {
        // Time only grows along the curve, so halving the stretch that holds `time` finds the
        // point of the curve at it.
        var low = 0.0
        var high = 1.0
        for _ in 0..<40 {
            let middle = (low + high) / 2
            if Self.coordinate(at: middle, x1, x2) < time {
                low = middle
            } else {
                high = middle
            }
        }
        return Self.coordinate(at: (low + high) / 2, y1, y2)
    }

    /// One coordinate of the point `along` the curve, from 0 to 1, given that coordinate of the
    /// two control points.
    private static func coordinate(at along: Double, _ first: Double, _ second: Double) -> Double {
        let back = 1 - along
        return 3 * back * back * along * first + 3 * back * along * along * second
            + along * along * along
    }
}

/// How the window's rhythms and its welcome dance move: live; held at their steady levels, while
/// macOS is set to reduce motion and while nobody can see the window; or frozen at one moment, as a
/// recording of the window draws it frame by frame.
public enum Motion: Sendable, Equatable {
    case live
    case steady
    /// Everything that moves stands as it does `moment` seconds into a recording that opens on the
    /// welcome screen and draws the window one frame at a time, so that nothing moves between
    /// frames: each rhythm at that point of its cycle, the welcome dance that far into its
    /// performance.
    case frozen(at: TimeInterval)

    /// The moment a recording draws, while one draws the window frame by frame.
    var frozenMoment: TimeInterval? {
        if case .frozen(let moment) = self { moment } else { nil }
    }
}

extension EnvironmentValues {
    @Entry public var motion: Motion = .live
}
