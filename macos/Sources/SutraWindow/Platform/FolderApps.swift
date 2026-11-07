import AppKit
import os

/// The apps a card's folder opens in, as macOS finds them: Finder, and the editors this Mac has.
extension Native {
    /// An app a card's folder opens in, with its name and icon for the button and the menu.
    public struct FolderApp {
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

    public var finder: FolderApp? {
        Self.foundFinder
    }

    public var editors: [FolderApp] {
        Self.installedEditors
    }

    /// Finder, looked for once, the first time a card is hovered.
    private static let foundFinder = FolderApp(bundleIdentifier: "com.apple.finder")

    /// Looked for once, the first time they are needed, so an editor installed while Sutra runs
    /// shows from its next launch.
    private static let installedEditors: [FolderApp] = [
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

    public func openInFinder(_ folder: String) {
        if !NSWorkspace.shared.open(URL(filePath: folder, directoryHint: .isDirectory)) {
            Self.log.error("Finder did not open \(folder, privacy: .public)")
        }
    }

    public func open(_ folder: String, in app: FolderApp) {
        NSWorkspace.shared.open(
            [URL(filePath: folder, directoryHint: .isDirectory)],
            withApplicationAt: app.url,
            configuration: NSWorkspace.OpenConfiguration()
        ) { _, error in
            if let error {
                Self.log.error("Opening \(folder, privacy: .public) failed: \(error, privacy: .public)")
            }
        }
    }
}

extension NativeServices {
    /// The editor a card's folder opens in: the one chosen in Settings while it is installed,
    /// otherwise the first installed, which is Visual Studio Code where it is. `nil` on a Mac with
    /// none.
    func editor(chosen: String) -> Native.FolderApp? {
        editors.first { $0.bundleIdentifier == chosen } ?? editors.first
    }
}
