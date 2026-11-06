import SwiftUI

/// The welcome illustration. A fixed time draws the same pose at any resolution for splash or listing artwork.
public struct PortDance: View {
    private let time: TimeInterval?
    private let seed: UInt64?
    private let showsCue: Bool
    @State private var sessionSeed = UInt64.random(in: .min ... .max)
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.motion) private var motion
    @State private var runningSince: Date? = Date()
    @State private var accumulatedTime: TimeInterval = 0

    public init(time: TimeInterval? = nil, seed: UInt64? = nil, showsCue: Bool = false) {
        self.time = time
        self.seed = seed
        self.showsCue = showsCue
    }

    public var body: some View {
        let isRunning = time == nil && !reduceMotion && motion == .live
        TimelineView(.animation(minimumInterval: 1.0 / 30, paused: !isRunning)) { timeline in
            let elapsed = time ?? (reduceMotion ? 0 : accumulatedTime + (runningSince.map { max(0, timeline.date.timeIntervalSince($0)) } ?? 0))
            let seed = seed ?? (time == nil ? sessionSeed : PortDanceChoreography.exportSeed)
            Canvas { context, size in
                context.withCGContext { cg in
                    PortDanceDrawing.draw(in: cg, size: size, time: elapsed, seed: seed, showsCue: showsCue)
                }
            }
        }
        .onChange(of: isRunning, initial: true) { _, running in
            if running {
                if runningSince == nil { runningSince = Date() }
            } else if let start = runningSince {
                accumulatedTime += max(0, Date().timeIntervalSince(start))
                runningSince = nil
            }
        }
        .accessibilityLabel("Three robots racing to a called dev server port; one wins and two pull back")
        .accessibilityAddTraits(.isImage)
        .allowsHitTesting(false)
    }
}
