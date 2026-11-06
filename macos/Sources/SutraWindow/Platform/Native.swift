import AppKit
import os
import ServiceManagement

/// The macOS services the window reaches itself: Finder, the pasteboard, the open panel, links,
/// the apps a folder opens in, and the login item. All are public API that works inside the App
/// Sandbox.
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
            + "folder you choose. Choose your home folder: your agents keep their sessions in "
            + "~/.claude and ~/.codex."
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

    /// An app a card's folder opens in, with its name and icon for the button and the menu.
    struct FolderApp {
        let bundleIdentifier: String
        let url: URL
        let name: String
        let icon: NSImage

        init?(bundleIdentifier: String) {
            guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleIdentifier)
            else { return nil }
            self.bundleIdentifier = bundleIdentifier
            self.url = url
            // The name carries `.app` when the user has Finder show every extension; the app's
            // name reads without it.
            let shown = FileManager.default.displayName(atPath: url.path(percentEncoded: false))
            name = shown.hasSuffix(".app") ? String(shown.dropLast(4)) : shown
            icon = NSWorkspace.shared.icon(forFile: url.path(percentEncoded: false))
        }
    }

    /// Finder, looked for once, the first time a card is hovered.
    static let finder = FolderApp(bundleIdentifier: "com.apple.finder")

    /// The editors on this Mac that open a folder as a project, in the order Settings lists them.
    /// Looked for once, the first time they are needed, so an editor installed while Sutra runs
    /// shows from its next launch.
    static let editors: [FolderApp] = [
        "com.microsoft.VSCode",
        "com.todesktop.230313mzl4w4u92", // Cursor
        "com.exafunction.windsurf",
        "dev.zed.Zed",
        "com.apple.dt.Xcode",
        "com.jetbrains.rustrover",
        "com.jetbrains.intellij",
        "com.jetbrains.intellij.ce",
        "com.jetbrains.WebStorm",
        "com.jetbrains.goland",
        "com.jetbrains.pycharm",
        "com.jetbrains.pycharm.ce",
        "com.jetbrains.CLion",
        "com.google.android.studio",
        "com.sublimetext.4",
        "com.panic.Nova",
        "com.barebones.bbedit",
        "com.macromates.TextMate",
        "org.vim.MacVim",
        "com.microsoft.VSCodeInsiders",
        "dev.zed.Zed-Preview",
    ].compactMap(FolderApp.init(bundleIdentifier:))

    /// Where the editor chosen in Settings is kept, by its bundle identifier.
    static let editorChoiceKey = "openFoldersIn"

    /// The editor a card's folder opens in: the one chosen in Settings while it is installed,
    /// otherwise the first installed, which is Visual Studio Code where it is. `nil` on a Mac with
    /// none.
    static func editor(chosen: String) -> FolderApp? {
        editors.first { $0.bundleIdentifier == chosen } ?? editors.first
    }

    /// Whether Sutra opens when the user logs in. macOS keeps this, and the user can change it in
    /// System Settings too.
    static var opensAtLogin: Bool {
        SMAppService.mainApp.status == .enabled
    }

    /// Ask macOS to open Sutra at login, or to stop, and say whether it now will. macOS may hold a
    /// new login item until the user approves it, so System Settings then opens at Login Items.
    static func setOpensAtLogin(_ open: Bool) -> Bool {
        do {
            if open {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            log.error("Changing the login item failed: \(error, privacy: .public)")
        }
        if open, SMAppService.mainApp.status == .requiresApproval {
            SMAppService.openSystemSettingsLoginItems()
        }
        return opensAtLogin
    }
}
