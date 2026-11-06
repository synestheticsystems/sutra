import Foundation
import Observation

/// Whether the welcome screen stands in front of the dashboard. It does on the first launch, until
/// the user continues, which its memory keeps, so later launches open on the dashboard; the Help
/// menu brings it back.
@MainActor
@Observable
public final class Welcome {
    /// The welcome screen stands in front of everything the window shows.
    public private(set) var shown: Bool

    /// The seed the welcome dance draws its calls from: `nil` for a new dance each time the screen
    /// shows, or a recording's own, so that every take dances alike.
    let danceSeed: UInt64?

    @ObservationIgnored private let memory: any WelcomeMemory

    public init(memory: any WelcomeMemory, danceSeed: UInt64? = nil) {
        self.memory = memory
        self.danceSeed = danceSeed
        shown = !memory.hasSeenIntroduction
    }

    /// The user continued: the dashboard takes the window, for as long as the memory keeps it.
    func continueToDashboard() {
        shown = false
        memory.hasSeenIntroduction = true
    }

    /// The welcome screen again, in front of the dashboard until the user continues.
    func replay() {
        shown = true
    }
}

/// Where the welcome screen keeps whether the user has continued past it.
public protocol WelcomeMemory: AnyObject {
    var hasSeenIntroduction: Bool { get set }
}

/// The app's defaults keep the user's continuing, so every later launch opens on the dashboard.
extension UserDefaults: WelcomeMemory {
    public var hasSeenIntroduction: Bool {
        get { bool(forKey: "hasSeenIntroduction") }
        set { set(newValue, forKey: "hasSeenIntroduction") }
    }
}
