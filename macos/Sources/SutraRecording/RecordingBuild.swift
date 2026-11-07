import AppKit

/// The recording build runs two ways. Launched as an app, it opens its window over invented work,
/// for a person to watch and try. Given `--film` and a folder, it draws the store preview's frames
/// into the folder offscreen and quits, showing nothing on the screen.
@main
enum RecordingBuild {
    static func main() {
        let arguments = Array(CommandLine.arguments.dropFirst())
        guard arguments.first == "--film" else {
            RecordingApp.main()
            return
        }
        guard arguments.count == 2 else {
            fputs("usage: SutraRecording [--film <folder>]\n", stderr)
            exit(2)
        }
        let folder = URL(filePath: arguments[1], directoryHint: .isDirectory)
        MainActor.assumeIsolated {
            // No Dock icon, no menu bar, never in front: the film shows nothing.
            NSApplication.shared.setActivationPolicy(.prohibited)
            do {
                try Film(into: folder).draw()
            } catch {
                fputs("error: \(error)\n", stderr)
                exit(1)
            }
        }
    }
}
