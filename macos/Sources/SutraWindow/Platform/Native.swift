import AppKit
import os

/// The macOS services the window reaches itself: Finder, the pasteboard, the open panel, links,
/// and the apps a folder opens in. All are public API that works inside the App Sandbox.
@MainActor
enum Native {
    private nonisolated static let log = Logger(
        subsystem: "systems.synesthetic.sutra",
        category: "native"
    )

    /// A Finder window with the item at `path` selected.
    static func revealInFinder(_ path: String) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(filePath: path)])
    }

    static func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    /// The store edition's open panel, starting at the home folder. Returns the folder chosen, or
    /// `nil` when the user cancels. The panel's own grant lets Sutra read the folder until it quits;
    /// the core keeps it for later launches.
    static func chooseFolder() -> String? {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.directoryURL = home
        // The contract leaves the panel's words to the window, whose panel it is.
        panel.message = "Sutra watches the worktrees, dev environments and build folders in the "
            + "folder you choose, apart from Desktop, Documents and Downloads. Choose your home "
            + "folder: your agents keep their sessions in ~/.claude and ~/.codex."
        panel.prompt = "Grant Access"
        guard panel.runModal() == .OK, let folder = panel.url else { return nil }
        return folder.path(percentEncoded: false)
    }

    /// The real home folder. Inside the App Sandbox the home folder Foundation reports is the
    /// app's container.
    private static var home: URL? {
        guard let user = getpwuid(getuid()), let directory = user.pointee.pw_dir else { return nil }
        return URL(filePath: String(cString: directory), directoryHint: .isDirectory)
    }

    /// Open a lozenge's link in the app registered for it: an agent's session, a unit in the
    /// browser.
    static func open(link: String) {
        guard let url = URL(string: link) else {
            log.error("Not a link: \(link, privacy: .public)")
            return
        }
        if !NSWorkspace.shared.open(url) {
            log.error("No app opened \(link, privacy: .public)")
        }
    }

    /// The folder in a window of the app that opens folders, Finder unless the user chose another.
    static func openInFinder(_ folder: String) {
        if !NSWorkspace.shared.open(URL(filePath: folder, directoryHint: .isDirectory)) {
            log.error("Finder did not open \(folder, privacy: .public)")
        }
    }

    /// Open `folder` in `app`.
    static func open(_ folder: String, in app: FolderApp) {
        NSWorkspace.shared.open(
            [URL(filePath: folder, directoryHint: .isDirectory)],
            withApplicationAt: app.url,
            configuration: NSWorkspace.OpenConfiguration()
        ) { _, error in
            if let error {
                log.error("Opening \(folder, privacy: .public) failed: \(error, privacy: .public)")
            }
        }
    }

    /// An app a card's folder opens in, with its name and icon for the button.
    struct FolderApp {
        let url: URL
        let name: String
        let icon: NSImage

        init?(bundleIdentifier: String) {
            guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleIdentifier)
            else { return nil }
            self.url = url
            name = FileManager.default.displayName(atPath: url.path(percentEncoded: false))
            icon = NSWorkspace.shared.icon(forFile: url.path(percentEncoded: false))
        }
    }

    /// Each looked for once, the first time a card is hovered; Visual Studio Code only when it is
    /// installed.
    static let vsCode = FolderApp(bundleIdentifier: "com.microsoft.VSCode")
    static let finder = FolderApp(bundleIdentifier: "com.apple.finder")
}
