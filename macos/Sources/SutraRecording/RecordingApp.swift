import AppKit
import SutraWindow
import SwiftUI

/// The recording build as an app: Sutra's window over invented work, for recording the Mac App
/// Store's preview without anyone's machine in it. It has no live core to start, as nothing in this
/// build makes one; packaging/macos/recording.sh bundles it with an identity of its own.
struct RecordingApp: App {
    @NSApplicationDelegateAdaptor private var delegate: RecordingDelegate

    var body: some Scene {
        SutraScenes(dashboard: delegate.recording.dashboard, welcome: delegate.recording.welcome)
    }
}

/// Holds the recording's window for as long as the app runs, and stops its script as the app
/// quits.
@MainActor
final class RecordingDelegate: NSObject, NSApplicationDelegate {
    let recording = Recording()

    /// The window is the app, as in Sutra: closing it quits.
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }

    func applicationWillTerminate(_ notification: Notification) {
        recording.script.stop()
    }
}
