import Darwin
import Foundation
@testable import SutraRecording
@testable import SutraWindow
import Testing

/// The recording's window as the recording build makes it: over the script, never the live core,
/// reaching no Mac service, and opening every take alike.
@MainActor
struct RecordingTests {
    @Test func theRecordingNeverStartsTheLiveCore() async throws {
        // The live core names every thread it starts `sutra-`, its dashboard's first. A thread of
        // the test's own shows the names can be read at all.
        let named = Thread { Thread.sleep(forTimeInterval: 2) }
        named.name = "recording-test"
        named.start()
        try await waitUntil { threadNames().contains("recording-test") }
        #expect(threadNames().contains("recording-test"))

        let recording = Recording()
        recording.welcome.continueToDashboard()
        try await Task.sleep(for: .milliseconds(1_200))
        #expect(recording.script.moment > 1, "the take played")
        #expect(!threadNames().contains { $0.hasPrefix("sutra-") })
        recording.script.stop()
    }

    @Test func theRecordingsWindowReachesOnlyTheStandInForMacServices() {
        #expect(Recording().dashboard.native is NativeStandIn)
    }

    @Test func everyLaunchOfTheRecordingOpensOnTheWelcomeScreen() {
        let first = Recording()
        #expect(first.welcome.shown)
        first.welcome.continueToDashboard()
        #expect(Recording().welcome.shown)
    }

    @Test func continuingPlaysTheTakeAndBringingTheWelcomeBackRewindsIt() async throws {
        let recording = Recording()
        #expect(recording.script.moment == 0, "the script waits behind the welcome screen")
        recording.welcome.continueToDashboard()
        try await Task.sleep(for: .milliseconds(1_200))
        #expect(recording.script.moment > 1)

        recording.script.perform(action: .setSort(order: .name))
        recording.welcome.replay()
        try await waitUntil { recording.script.moment == 0 }
        #expect(recording.script.moment == 0)
        #expect(shown(recording.script.contents()) == takeOpening)
        recording.script.stop()
    }

    /// Waits until `condition` holds, for as long as two seconds, giving the main actor up
    /// meanwhile to the work it waits on.
    private func waitUntil(_ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(2)
        while !condition(), ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
    }

    /// The names of this process's threads, as macOS keeps them.
    private func threadNames() -> [String] {
        let pid = getpid()
        var handles = [UInt64](repeating: 0, count: 1_024)
        let listed = proc_pidinfo(
            pid, PROC_PIDLISTTHREADS, 0, &handles, Int32(handles.count * MemoryLayout<UInt64>.size)
        )
        return handles.prefix(Int(listed) / MemoryLayout<UInt64>.size).compactMap { handle in
            var info = proc_threadinfo()
            let size = Int32(MemoryLayout<proc_threadinfo>.size)
            // A thread that ended since it was listed tells nothing.
            guard proc_pidinfo(pid, PROC_PIDTHREADINFO, handle, &info, size) == size else {
                return nil
            }
            return withUnsafeBytes(of: info.pth_name) { name in
                String(decoding: name.prefix { $0 != 0 }, as: UTF8.self)
            }
        }
    }
}
