import AppKit
import ImageIO
import UniformTypeIdentifiers

/// The store preview's film: the recording's window drawn offscreen into PNG files, frame by
/// frame, as the cut directs. Nothing shows on the screen and no clock moves between frames: the
/// take, the welcome dance and the throbs all stand at each frame's moment, so every film of the
/// cut draws the same frames.
@MainActor
final class Film {
    private let recording = Recording()
    private let window: FilmWindow
    /// Where the frames go, as `frame-0000.png` and on.
    private let folder: URL

    init(into folder: URL) {
        self.folder = folder
        window = FilmWindow(showing: recording)
    }

    /// Draw every frame of the cut.
    func draw() throws(FilmError) {
        do {
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        } catch {
            throw .cannotWrite(folder)
        }
        window.settle(for: 0.5)
        for frame in 0..<Cut.frames {
            try act(as: frame)
            let moment = Cut.moment(of: frame)
            if !recording.welcome.shown {
                recording.script.hold(at: moment - Cut.continues)
            }
            window.show(at: moment)
            try write(window.image(), as: frame)
        }
    }

    /// Do what the cut does as `frame` comes.
    private func act(as frame: Int) throws(FilmError) {
        switch frame {
        case Cut.frame(at: Cut.continues):
            window.pressReturn()
            // The recording plays its take from the main actor's next turn once the welcome screen
            // has gone, and the film holds the take from then on.
            window.settle()
            guard !recording.welcome.shown else {
                throw .welcomeStayed
            }
        case Cut.frame(at: Cut.folds), Cut.frame(at: Cut.unfolds):
            // What a click on Cedar Web's line asks of the core.
            recording.script.perform(action: .toggleFold(line: Scenario.cedarWebLine))
        default:
            break
        }
    }

    private func write(_ image: CGImage, as frame: Int) throws(FilmError) {
        let file = folder.appending(path: String(format: "frame-%04d.png", frame))
        guard let destination = CGImageDestinationCreateWithURL(
            file as CFURL, UTType.png.identifier as CFString, 1, nil
        ) else {
            throw .cannotWrite(file)
        }
        CGImageDestinationAddImage(destination, image, nil)
        guard CGImageDestinationFinalize(destination) else {
            throw .cannotWrite(file)
        }
    }
}

/// Why the film stopped.
enum FilmError: Error, CustomStringConvertible {
    /// Return left the welcome screen up: its button no longer answers as the window's default
    /// action.
    case welcomeStayed
    case cannotDraw
    case cannotWrite(URL)

    var description: String {
        switch self {
        case .welcomeStayed:
            "Return left the welcome screen up, so the film cannot press Let's untangle this"
        case .cannotDraw:
            "The window could not be drawn"
        case .cannotWrite(let file):
            "Could not write \(file.path(percentEncoded: false))"
        }
    }
}
