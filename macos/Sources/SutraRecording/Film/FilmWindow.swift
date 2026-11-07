import AppKit
import SutraWindow
import SwiftUI

/// The film's window: the recording's window as the app's Window scene makes it, its title and
/// toolbar in the title bar, at 960 by 540 points. It is never ordered onto the screen. It draws
/// itself as the app's window looks in use, the key window of the active app, at 2x, whatever this
/// Mac's screens are and whichever app is in front.
@MainActor
final class FilmWindow: NSWindow {
    /// The window's size, which drawn at 2x fills a frame of the store's 1920 by 1080 pixels.
    private static let size = NSSize(width: 960, height: 540)

    private let recording: Recording
    private let host: NSHostingController<FilmedDashboard>

    init(showing recording: Recording) {
        self.recording = recording
        host = NSHostingController(rootView: FilmedDashboard(recording: recording, moment: 0))
        // The title and the toolbar go to the window's title bar, as the Window scene puts them.
        host.sceneBridgingOptions = [.title, .toolbars]
        // The film sizes the window, not what it shows.
        host.sizingOptions = []
        super.init(
            contentRect: NSRect(origin: .zero, size: Self.size),
            // As the app's Window scene makes its window, so the toolbar overlays the contents
            // and the window keeps its size as the toolbar comes.
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        tabbingMode = .disallowed
        // Every film looks alike, whether this Mac is set to light or dark.
        appearance = NSAppearance(named: .aqua)
        contentViewController = host
        setFrame(NSRect(origin: .zero, size: Self.size), display: false)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("made in code only")
    }

    /// The frames are 1920 by 1080 pixels, so the window draws at 2x on any Mac, and its views
    /// keep their bitmaps at 2x.
    override var backingScaleFactor: CGFloat {
        2
    }

    // Offscreen in an app that is never in front, the window would draw itself as a window behind
    // the active app's does: grey window buttons, a grey title, a grey default button. AppKit
    // chooses by asking the window these two private questions, and the answers below draw the
    // window as the app looks in use. They belong to the recording build alone, never to the app:
    // the Mac App Store refuses private API.

    /// Asked for SwiftUI's controls: Let's untangle this and the sort's choice take the accent.
    @objc(hasKeyAppearance)
    func drawsAsKey() -> Bool {
        true
    }

    /// Asked for the title bar: the window buttons take their colours, the title its ink.
    @objc(_hasActiveAppearanceIgnoringKeyFocus)
    func drawsAsActive() -> Bool {
        true
    }

    /// Press Return, which the welcome screen's button answers as the window's default action.
    /// Whether the screen went, the film looks for itself.
    func pressReturn() {
        let returnKey = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [], timestamp: 0,
            windowNumber: windowNumber, context: nil, characters: "\r",
            charactersIgnoringModifiers: "\r", isARepeat: false, keyCode: 36
        )
        if let returnKey {
            _ = performKeyEquivalent(with: returnKey)
        }
    }

    /// Show the window as it stands at `moment` of the film: everything that moves frozen there,
    /// laid out and drawn.
    func show(at moment: TimeInterval) {
        host.rootView = FilmedDashboard(recording: recording, moment: moment)
        settle()
    }

    /// Let AppKit and SwiftUI carry out what the film last changed: the main actor's waiting work,
    /// such as the recording's turn to its take once the welcome screen goes, SwiftUI's update of
    /// the views, and the window's layout and drawing. The window's first layout and drawing take
    /// AppKit longer than the change from one frame to the next.
    func settle(for duration: TimeInterval = 0.02) {
        RunLoop.main.run(until: Date(timeIntervalSinceNow: duration))
        layoutIfNeeded()
        displayIfNeeded()
    }

    /// The window as it stands, title bar and all, at 2x.
    func image() throws(FilmError) -> CGImage {
        guard let frameView = contentView?.superview, let layer = frameView.layer else {
            throw .cannotDraw
        }
        let size = frameView.bounds.size
        guard let context = CGContext(
            data: nil, width: Int(size.width * backingScaleFactor),
            height: Int(size.height * backingScaleFactor), bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
        ) else {
            throw .cannotDraw
        }
        context.scaleBy(x: backingScaleFactor, y: backingScaleFactor)
        // On the screen, the window server lays the window's background under its views.
        effectiveAppearance.performAsCurrentDrawingAppearance {
            context.setFillColor(backgroundColor.cgColor)
        }
        context.fill(CGRect(origin: .zero, size: size))
        // A portal shows what the window server mirrors into it, which a drawing made offscreen
        // cannot reach, so here it draws nothing. The scroll edge's portal under the title bar
        // composites as "destination in", and drawing nothing that way erases everything under
        // it, so the window is drawn with its portals left out.
        let portals = Self.shownPortals(under: layer)
        portals.forEach { $0.isHidden = true }
        layer.render(in: context)
        portals.forEach { $0.isHidden = false }
        guard let image = context.makeImage() else {
            throw .cannotDraw
        }
        return image
    }

    /// The portal layers under `layer`, itself included, that would be drawn.
    private static func shownPortals(under layer: CALayer) -> [CALayer] {
        let portal = !layer.isHidden && String(describing: type(of: layer)).hasSuffix("PortalLayer")
        return (portal ? [layer] : []) + (layer.sublayers ?? []).flatMap(shownPortals(under:))
    }
}

/// The recording's window at one moment of the film, everything that moves frozen there.
private struct FilmedDashboard: View {
    let recording: Recording
    let moment: TimeInterval

    var body: some View {
        DashboardView(dashboard: recording.dashboard, welcome: recording.welcome)
            .environment(\.motion, .frozen(at: moment))
    }
}
