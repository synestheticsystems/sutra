@testable import SutraWindow
import Testing

/// The terminals Settings offers, the one a card's folder opens in, and the way each terminal is
/// handed the folder its new shell starts in.
@MainActor
struct FolderAppsTests {
    /// Services that offer the terminals they are given, and do nothing else.
    private struct Offering: NativeServices {
        let terminals: [Native.FolderApp]

        var finder: Native.FolderApp? { nil }
        var editors: [Native.FolderApp] { [] }
        var opensAtLogin: Bool { false }
        func revealInFinder(_ path: String) {}
        func copy(_ text: String) {}
        func chooseFolder() -> String? { nil }
        func open(link: String) {}
        func openInFinder(_ folder: String) {}
        func open(_ folder: String, in app: Native.FolderApp) {}
        func setOpensAtLogin(_ open: Bool) -> Bool { false }
    }

    @Test func settingsListsTerminalFirst() {
        #expect(Native().terminals.first?.bundleIdentifier == "com.apple.Terminal")
    }

    @Test func aFolderOpensInTheTerminalChosenWhileThisMacHasIt() throws {
        // Console stands in for a terminal besides Terminal: every Mac has it.
        let services = Offering(terminals: [
            try #require(Native.FolderApp(bundleIdentifier: "com.apple.Terminal")),
            try #require(Native.FolderApp(bundleIdentifier: "com.apple.Console")),
        ])
        #expect(services.terminal(chosen: "com.apple.Console")?.bundleIdentifier == "com.apple.Console")
    }

    @Test func aFolderOpensInTerminalUntilATerminalIsChosenAndOnceTheOneChosenIsGone() throws {
        let services = Offering(terminals: [
            try #require(Native.FolderApp(bundleIdentifier: "com.apple.Terminal")),
        ])
        #expect(services.terminal(chosen: "")?.bundleIdentifier == "com.apple.Terminal")
        #expect(
            services.terminal(chosen: "com.googlecode.iterm2")?.bundleIdentifier
                == "com.apple.Terminal",
            "iTerm2 was chosen, and this Mac no longer has it"
        )
    }

    @Test func terminalsThatStartAShellInAFolderHandedToThemAreHandedItAsADocument() {
        for terminal in [
            "com.apple.Terminal", "com.googlecode.iterm2", "com.mitchellh.ghostty",
            "dev.warp.Warp-Stable",
        ] {
            #expect(handoff(of: terminal) == .document, "\(terminal)")
        }
    }

    @Test func alacrittyKittyAndWezTermAreHandedTheFolderOnTheCommandLineOfANewInstance() {
        #expect(handoff(of: "org.alacritty") == .commandLine(before: ["--working-directory"]))
        #expect(
            handoff(of: "net.kovidgoyal.kitty")
                == .commandLine(before: ["-o", "macos_quit_when_last_window_closed=yes", "--directory"])
        )
        #expect(handoff(of: "com.github.wez.wezterm") == .commandLine(before: ["start", "--cwd"]))
    }

    @Test func insideTheSandboxSettingsOffersOnlyTheTerminalsHandedTheFolderAsADocument() {
        #expect(Native.offeredTerminals(sandboxed: true).map(\.bundleIdentifier) == [
            "com.apple.Terminal", "com.googlecode.iterm2", "com.mitchellh.ghostty",
            "dev.warp.Warp-Stable",
        ])
        #expect(
            Native.offeredTerminals(sandboxed: false).map(\.bundleIdentifier)
                == Native.knownTerminals.map(\.bundleIdentifier),
            "outside it, every terminal Sutra knows"
        )
    }

    private func handoff(of bundleIdentifier: String) -> Native.FolderApp.Handoff? {
        Native.knownTerminals.first { $0.bundleIdentifier == bundleIdentifier }?.handoff
    }
}
