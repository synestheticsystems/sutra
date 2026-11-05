import AppKit
import SwiftUI

/// Escape in the window closes what is open, whichever view has the keyboard, the filter field
/// included, and when none does. SwiftUI's own Escape handling reaches only a view that has the
/// keyboard, so this listens to the window's key presses instead. A press that closes nothing, and
/// every press while a sheet or panel is up, goes on as if the window had never seen it.
struct EscapeKey: NSViewRepresentable {
    /// Closes what is open, and says whether there was anything.
    let close: @MainActor () -> Bool

    func makeNSView(context: Context) -> Listener {
        Listener(close: close)
    }

    func updateNSView(_ listener: Listener, context: Context) {
        listener.close = close
    }

    /// An empty view in the window, through which the listener knows which window is its own. It
    /// listens while it is in one.
    final class Listener: NSView {
        var close: @MainActor () -> Bool
        private var monitor: Any?

        init(close: @escaping @MainActor () -> Bool) {
            self.close = close
            super.init(frame: .zero)
        }

        @available(*, unavailable)
        required init?(coder: NSCoder) {
            fatalError("made in code only")
        }

        /// It lies behind the whole window, and every click is for what lies in front of it.
        override func hitTest(_ point: NSPoint) -> NSView? {
            nil
        }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let monitor {
                NSEvent.removeMonitor(monitor)
                self.monitor = nil
            }
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
                let escape = event.keyCode == Self.escapeKeyCode
                let sentTo = event.window.map(ObjectIdentifier.init)
                // AppKit hands key presses to local monitors on the main thread.
                let closed = MainActor.assumeIsolated {
                    escape && self?.closeIfOpen(inWindow: sentTo) == true
                }
                return closed ? nil : event
            }
        }

        /// Close what is open, for an Escape pressed in the window `sentTo`, while nothing else in
        /// the app waits on the user.
        private func closeIfOpen(inWindow sentTo: ObjectIdentifier?) -> Bool {
            guard let window, sentTo == ObjectIdentifier(window), window.attachedSheet == nil,
                  NSApp.modalWindow == nil
            else { return false }
            return close()
        }

        /// The Escape key's code, the same on every keyboard layout.
        private nonisolated static let escapeKeyCode: UInt16 = 53
    }
}
