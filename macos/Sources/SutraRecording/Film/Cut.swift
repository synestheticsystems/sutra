import Foundation

/// The store preview's cut: what the film shows, second by second. It opens on the welcome screen,
/// as the app's first launch does, and presses Let's untangle this once the robots have raced and
/// one has landed. The take then plays over the dashboard: the layout fix stops to ask permission
/// and its card throbs, Cedar Web's worktrees fold under its line, which throbs for them, and open
/// again, the fix works on, and the search page finishes with its subagents.
enum Cut {
    static let framesPerSecond = 30
    /// Inside the store's thirty seconds.
    static let length: TimeInterval = 29.5
    /// The film presses Let's untangle this, and the take begins. The layout fix asks permission
    /// six seconds later, at 9.6 seconds, six whole throbs into the film, so its card's throb
    /// swells in from rest.
    static let continues: TimeInterval = 3.6
    /// Cedar Web's worktrees fold under its line while the layout fix still waits in them.
    static let folds: TimeInterval = 18
    /// They open again.
    static let unfolds: TimeInterval = 22

    /// How many frames the film draws.
    static var frames: Int {
        frame(at: length)
    }

    /// The frame that first shows `moment` of the film.
    static func frame(at moment: TimeInterval) -> Int {
        Int((moment * Double(framesPerSecond)).rounded())
    }

    /// The moment of the film that `frame` shows.
    static func moment(of frame: Int) -> TimeInterval {
        Double(frame) / Double(framesPerSecond)
    }
}
