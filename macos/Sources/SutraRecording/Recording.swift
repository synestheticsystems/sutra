import Observation
import SutraCore
import SutraWindow

/// The recording build's window, made as the app makes its own but over the script in place of the
/// live core, with a stand-in for macOS's services, and opening on the welcome screen at every
/// launch. Nothing it shows comes from the Mac it runs on, and no click in it reaches that Mac.
///
/// The script waits at its start behind the welcome screen and plays from the moment the user
/// continues, so every take opens alike: the first at launch, and each later one once Help >
/// Welcome to Robo Sutra has brought the screen back and rewound the script.
@MainActor
final class Recording {
    let dashboard: DashboardModel
    let welcome = Welcome(memory: LaunchMemory(), danceSeed: Scenario.danceSeed)
    let script: ScriptedCore

    init() {
        // The relay would post the core's banners; the script asks for none.
        let relay = WindowRelay(banners: NotificationBanners())
        script = ScriptedCore(window: relay)
        dashboard = DashboardModel(core: script, native: NativeStandIn())
        relay.deliver(to: dashboard)
        followWelcome()
    }

    /// Rewind the script each time the welcome screen comes back, and play it each time the user
    /// continues past it.
    private func followWelcome() {
        withObservationTracking {
            _ = welcome.shown
        } onChange: { [weak self] in
            // Told as the screen is about to come or go, so it looks once it has.
            Task { @MainActor [weak self] in
                guard let self else { return }
                if self.welcome.shown {
                    self.script.rewind()
                } else {
                    self.script.play()
                }
                self.followWelcome()
            }
        }
    }
}

/// The recording's memory of the user continuing past the welcome screen, which lasts as long as
/// the launch, so every launch opens on the screen and nothing is kept.
private final class LaunchMemory: WelcomeMemory {
    var hasSeenIntroduction = false
}
