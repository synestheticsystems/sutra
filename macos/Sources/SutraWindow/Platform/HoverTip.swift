import AppKit
import SwiftUI

extension View {
    /// `text` as the view's tooltip, shown while the pointer rests on the view whether or not
    /// Sutra is the active app. `nil` shows none.
    ///
    /// macOS shows a `.help` tooltip only in the active app, and Sutra's window is mostly read
    /// from another app, beside the work it reports on: there no `.help` tooltip ever showed. The
    /// window's own controls, the toolbar's buttons and the sort's segments, keep `.help`, the
    /// tooltip macOS draws for its controls: they are there to be clicked, and a click brings
    /// Sutra forward, where those tooltips show.
    func hoverTip(_ text: String?) -> some View {
        modifier(HoverTipModifier(text: text))
    }
}

private struct HoverTipModifier: ViewModifier {
    let text: String?

    /// Tells this view apart from the others with a tip, for as long as it is shown.
    @State private var anchor = HoverTipAnchor()

    func body(content: Content) -> some View {
        content
            .accessibilityHint(text ?? "")
            .onHover { over in
                if over {
                    HoverTips.shared.pointerEntered(anchor, text: text)
                } else {
                    HoverTips.shared.pointerLeft(anchor)
                }
            }
            .onChange(of: text) { _, text in
                HoverTips.shared.retell(anchor, text: text)
            }
            .onDisappear {
                HoverTips.shared.pointerLeft(anchor)
            }
    }
}

/// One view with a tip.
private final class HoverTipAnchor {}

/// The tip on screen, and the views with a tip that the pointer is over. As macOS does with its
/// tooltips: a tip shows once the pointer has rested a moment, the next shows at once as the
/// pointer passes on from one, and a click, a scroll or a key press takes it away until the
/// pointer leaves that view.
@MainActor
private final class HoverTips {
    static let shared = HoverTips()

    /// How long the pointer rests on a view before its tip shows.
    private static let delay = Duration.milliseconds(800)
    /// For how long after a tip goes the next one shows at once.
    private static let passingOn: TimeInterval = 0.5

    /// The views with a tip that the pointer is over, the innermost last: its tip is the one
    /// that shows.
    private var under: [(anchor: HoverTipAnchor, text: String?)] = []
    /// The view whose tip a click, a scroll or a key press took away.
    private var dismissed: HoverTipAnchor?
    private var waiting: Task<Void, Never>?
    private var hiddenAt = Date.distantPast
    private lazy var panel = HoverTipPanel()
    private var watchingInput = false

    func pointerEntered(_ anchor: HoverTipAnchor, text: String?) {
        watchInput()
        under.removeAll { $0.anchor === anchor }
        under.append((anchor, text))
        hide()
        showSoon()
    }

    func pointerLeft(_ anchor: HoverTipAnchor) {
        if dismissed === anchor {
            dismissed = nil
        }
        guard let place = under.firstIndex(where: { $0.anchor === anchor }) else { return }
        under.remove(at: place)
        if place == under.count {
            hide()
            showSoon()
        }
    }

    /// The text of `anchor`'s tip changed, as a status does while the pointer rests on it.
    func retell(_ anchor: HoverTipAnchor, text: String?) {
        guard let place = under.firstIndex(where: { $0.anchor === anchor }) else { return }
        under[place].text = text
        guard place == under.count - 1 else { return }
        if panel.isVisible {
            if let text {
                panel.retell(text)
            } else {
                hide()
            }
        } else if waiting == nil {
            // A tip on its way reads the newest text when it shows, so its wait goes on.
            showSoon()
        }
    }

    /// Show the innermost view's tip once the pointer has rested on it, or at once as the pointer
    /// passes on from another tip.
    private func showSoon() {
        stopWaiting()
        guard let shown = under.last, shown.text != nil, shown.anchor !== dismissed else { return }
        let atOnce = Date.now.timeIntervalSince(hiddenAt) < Self.passingOn
        waiting = Task { [self] in
            if !atOnce {
                try? await Task.sleep(for: Self.delay)
            }
            guard !Task.isCancelled else { return }
            waiting = nil
            guard let shown = under.last, let text = shown.text, shown.anchor !== dismissed
            else { return }
            panel.show(text, at: NSEvent.mouseLocation)
        }
    }

    private func stopWaiting() {
        waiting?.cancel()
        waiting = nil
    }

    private func hide() {
        stopWaiting()
        if panel.isVisible {
            panel.orderOut(nil)
            hiddenAt = .now
        }
    }

    private func dismiss() {
        stopWaiting()
        dismissed = under.last?.anchor
        if panel.isVisible {
            panel.orderOut(nil)
        }
    }

    /// Take the tip away on a click, a scroll or a key press, which reach Sutra whether or not it
    /// is active. Watched from the first tip on, for as long as the app runs.
    private func watchInput() {
        guard !watchingInput else { return }
        watchingInput = true
        let input: NSEvent.EventTypeMask = [
            .leftMouseDown, .rightMouseDown, .otherMouseDown, .scrollWheel, .keyDown,
        ]
        _ = NSEvent.addLocalMonitorForEvents(matching: input) { event in
            MainActor.assumeIsolated {
                HoverTips.shared.dismiss()
            }
            return event
        }
    }
}

/// The panel a tip shows in: above every window, never key, letting the pointer through, and
/// shown while Sutra is not the active app, as a panel by default is not.
private final class HoverTipPanel: NSPanel {
    /// How far below the pointer a tip's top edge stands, clear of the arrow.
    private static let belowPointer: CGFloat = 20
    /// How far above the pointer a tip stands where the screen ends below it.
    private static let abovePointer: CGFloat = 6

    private let bubble = NSHostingView(rootView: TipBubble(text: ""))

    init() {
        super.init(
            contentRect: .zero,
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: true
        )
        isOpaque = false
        backgroundColor = .clear
        hasShadow = true
        level = .popUpMenu
        ignoresMouseEvents = true
        hidesOnDeactivate = false
        isReleasedWhenClosed = false
        animationBehavior = .none
        collectionBehavior = [.canJoinAllSpaces, .transient, .ignoresCycle, .fullScreenAuxiliary]
        contentView = bubble
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }

    /// Show `text` below the pointer at `pointer`, as a tooltip shows, or above it where the
    /// screen ends, and never past the screen's sides.
    func show(_ text: String, at pointer: NSPoint) {
        let size = draw(text)
        var origin = NSPoint(x: pointer.x, y: pointer.y - Self.belowPointer - size.height)
        let screen = NSScreen.screens.first { NSMouseInRect(pointer, $0.frame, false) }
        if let visible = (screen ?? NSScreen.main)?.visibleFrame {
            if origin.y < visible.minY {
                origin.y = pointer.y + Self.abovePointer
            }
            origin.x = max(visible.minX, min(origin.x, visible.maxX - size.width))
        }
        setFrame(NSRect(origin: origin, size: size), display: true)
        orderFrontRegardless()
    }

    /// Redraw the tip with `text`, its top left corner where it stands.
    func retell(_ text: String) {
        let size = draw(text)
        let top = NSPoint(x: frame.minX, y: frame.maxY)
        setFrame(NSRect(x: top.x, y: top.y - size.height, width: size.width, height: size.height), display: true)
        invalidateShadow()
    }

    /// Lay `text` out in the bubble, and its size.
    private func draw(_ text: String) -> NSSize {
        bubble.rootView = TipBubble(text: text)
        return bubble.fittingSize
    }
}

/// A tip as macOS draws a tooltip: small text on the window's colour, in a thin rounded edge.
private struct TipBubble: View {
    let text: String

    var body: some View {
        WrapAt(width: 320) {
            Text(text)
                .font(Font(NSFont.toolTipsFont(ofSize: 0)))
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 3)
        .background(Color(nsColor: .windowBackgroundColor), in: .rect(cornerRadius: 5))
        .overlay {
            RoundedRectangle(cornerRadius: 5)
                .strokeBorder(Color(nsColor: .separatorColor), lineWidth: 0.5)
        }
    }
}

/// Its one subview at the width it would take on one line, or at `width` and wrapped where it
/// is wider, so a short tip stays short and a long one wraps.
private struct WrapAt: Layout {
    let width: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        guard let text = subviews.first else { return .zero }
        let oneLine = text.sizeThatFits(.unspecified)
        return text.sizeThatFits(ProposedViewSize(width: min(oneLine.width, width), height: nil))
    }

    func placeSubviews(
        in bounds: CGRect,
        proposal: ProposedViewSize,
        subviews: Subviews,
        cache: inout ()
    ) {
        subviews.first?.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
    }
}
