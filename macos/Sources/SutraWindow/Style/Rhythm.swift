import Foundation
import SwiftUI

/// What moves on the list: a waiting card throbs, and its waiting agents with it; an agent at work
/// pulses slowly. Every rhythm runs on the wall clock, so whatever throbs throbs in step, however
/// late it appeared.
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

    /// The least time between frames. A throb swells to a strong colour and shows its steps at
    /// fewer than ten frames a second; a pulse is a faint tint changing slowly, and can run all
    /// day, so it takes five.
    fileprivate var frameInterval: TimeInterval {
        switch self {
        case .throb: 0.1
        case .pulse: 0.2
        }
    }

    /// The level held while nothing moves: a waiting card stays at its peak, so it still stands
    /// out; a working agent rests halfway.
    fileprivate var steadyLevel: Double {
        switch self {
        case .throb: 1
        case .pulse: 0.5
        }
    }

    /// How far into its cycle the rhythm is at `date`, from 0 to 1 and back, easing in and out.
    fileprivate func level(at date: Date) -> Double {
        let phase = date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: period) / period
        return 0.5 - 0.5 * cos(2 * .pi * phase)
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

/// `content` drawn at the level of `rhythm`, from 0 to 1, frame by frame while motion is live.
struct Rhythmic<Content: View>: View {
    let rhythm: Rhythm
    @ViewBuilder let content: (Double) -> Content

    @Environment(\.motion) private var motion

    var body: some View {
        switch motion {
        case .steady:
            content(rhythm.steadyLevel)
        case .live:
            TimelineView(.animation(minimumInterval: rhythm.frameInterval)) { timeline in
                content(rhythm.level(at: timeline.date))
            }
        }
    }
}
