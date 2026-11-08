import AppKit
import os

/// The apps a card's folder opens in, as macOS finds them: Finder, and the editors and the
/// terminals this Mac has.
extension Native {
    /// An app a card's folder opens in, with its name and icon for the button and the menu, and the
    /// way it is handed the folder.
    public struct FolderApp {
        let bundleIdentifier: String
        let url: URL
        let name: String
        let icon: NSImage
        let handoff: Handoff

        init?(bundleIdentifier: String, handoff: Handoff = .document) {
            guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: bundleIdentifier)
            else { return nil }
            self.bundleIdentifier = bundleIdentifier
            self.url = url
            self.handoff = handoff
            // The name carries `.app` when the user has Finder show every extension; the app's
            // name reads without it.
            let shown = FileManager.default.displayName(atPath: url.path(percentEncoded: false))
            name = shown.hasSuffix(".app") ? String(shown.dropLast(4)) : shown
            icon = NSWorkspace.shared.icon(forFile: url.path(percentEncoded: false))
        }

        /// How an app is handed the folder it opens.
        enum Handoff: Equatable {
            /// As a document, as Finder hands an app the folder dropped on its icon.
            case document
            /// On the command line of a new instance of the app, after the arguments `before`.
            case commandLine(before: [String])
        }
    }

    public var finder: FolderApp? {
        Self.foundFinder
    }

    public var editors: [FolderApp] {
        Self.installedEditors
    }

    public var terminals: [FolderApp] {
        Self.installedTerminals
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
    ].compactMap { FolderApp(bundleIdentifier: $0) }

    /// The terminals Sutra knows, in the order Settings lists them, each with the way it is handed
    /// the folder its new shell starts in. Terminal comes first: every Mac has it, so a card's
    /// folder opens there until the user chooses another, and again once the one chosen is gone.
    static let knownTerminals: [(bundleIdentifier: String, handoff: FolderApp.Handoff)] = [
        // Each opens a folder handed to it in a new window or tab whose shell starts there.
        ("com.apple.Terminal", .document),
        ("com.googlecode.iterm2", .document),
        ("com.mitchellh.ghostty", .document),
        ("dev.warp.Warp-Stable", .document),
        // These start in the folder their command lines name, as Alacritty opens no folder handed
        // to it, kitty opens one as the user's launch actions say, and WezTerm types one into a
        // new shell as a command. By default kitty keeps running once its last window closes, so
        // its new instance is told to quit with its window.
        ("org.alacritty", .commandLine(before: ["--working-directory"])),
        (
            "net.kovidgoyal.kitty",
            .commandLine(before: ["-o", "macos_quit_when_last_window_closed=yes", "--directory"])
        ),
        ("com.github.wez.wezterm", .commandLine(before: ["start", "--cwd"])),
    ]

    /// The terminals Settings offers: the known terminals that start in a folder Sutra hands them.
    /// The App Sandbox drops the arguments of every app Sutra launches, so inside it only those
    /// handed the folder as a document do; the others would open in a folder of their own choosing.
    static func offeredTerminals(
        sandboxed: Bool
    ) -> [(bundleIdentifier: String, handoff: FolderApp.Handoff)] {
        sandboxed ? knownTerminals.filter { $0.handoff == .document } : knownTerminals
    }

    /// Looked for once, as the editors are.
    private static let installedTerminals = offeredTerminals(sandboxed: sandboxed).compactMap {
        FolderApp(bundleIdentifier: $0.bundleIdentifier, handoff: $0.handoff)
    }

    /// Whether Sutra runs inside the App Sandbox, as the store edition does. The sandbox puts this
    /// variable in the environment of every process it confines, the sign the core reads too
    /// (`src/map/grant.rs`).
    private static let sandboxed =
        ProcessInfo.processInfo.environment["APP_SANDBOX_CONTAINER_ID"] != nil

    /// Where the editor chosen in Settings is kept, by its bundle identifier.
    static let editorChoiceKey = "openFoldersIn"

    /// Where the terminal chosen in Settings is kept, by its bundle identifier.
    static let terminalChoiceKey = "terminal"

    public func openInFinder(_ folder: String) {
        if !NSWorkspace.shared.open(URL(filePath: folder, directoryHint: .isDirectory)) {
            Self.log.error("Finder did not open \(folder, privacy: .public)")
        }
    }

    public func open(_ folder: String, in app: FolderApp) {
        let configuration = NSWorkspace.OpenConfiguration()
        let logFailure: @Sendable (NSRunningApplication?, (any Error)?) -> Void = { _, error in
            if let error {
                Self.log.error("Opening \(folder, privacy: .public) failed: \(error, privacy: .public)")
            }
        }
        switch app.handoff {
        case .document:
            NSWorkspace.shared.open(
                [URL(filePath: folder, directoryHint: .isDirectory)],
                withApplicationAt: app.url,
                configuration: configuration,
                completionHandler: logFailure
            )
        case .commandLine(let before):
            // An app already running ignores the arguments of a launch that only brings it
            // forward, so the folder goes to a new instance.
            configuration.createsNewApplicationInstance = true
            configuration.arguments = before + [folder]
            NSWorkspace.shared.openApplication(
                at: app.url,
                configuration: configuration,
                completionHandler: logFailure
            )
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

    /// The terminal a card's folder opens in: the one chosen in Settings while it is installed,
    /// otherwise the first, Terminal. `nil` only where macOS cannot find Terminal itself.
    func terminal(chosen: String) -> Native.FolderApp? {
        terminals.first { $0.bundleIdentifier == chosen } ?? terminals.first
    }
}
