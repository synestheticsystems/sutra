@testable import SutraWindow
import Testing

/// The welcome screen across launches: each launch is a new `Welcome` over one memory, as each
/// launch of the app reads the same defaults.
@MainActor
struct WelcomeTests {
    /// The defaults of one Mac, which outlast every launch.
    private final class Defaults: WelcomeMemory {
        var hasSeenIntroduction = false
    }

    private let defaults = Defaults()

    @Test func aFirstLaunchOpensOnTheWelcomeScreen() {
        #expect(Welcome(memory: defaults).shown)
    }

    @Test func continuingRevealsTheDashboardNowAndAtEveryLaterLaunch() {
        let first = Welcome(memory: defaults)
        first.continueToDashboard()
        #expect(!first.shown)
        #expect(!Welcome(memory: defaults).shown)
    }

    @Test func theHelpMenuBringsTheWelcomeBackUntilTheUserContinuesAgain() {
        let welcome = Welcome(memory: defaults)
        welcome.continueToDashboard()
        welcome.replay()
        #expect(welcome.shown)
        #expect(!Welcome(memory: defaults).shown, "a launch during the replay opens on the dashboard")
        welcome.continueToDashboard()
        #expect(!welcome.shown)
    }
}
