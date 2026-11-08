import AppKit
import os
import ServiceManagement
import SwiftUI

/// The macOS services the window reaches itself, rather than through the core: Finder, the
/// pasteboard, the open panel, links, the apps a folder opens in, and the login item. The window's
/// model holds the services it reaches, macOS's own (`Native`) in the app, and the window hands
/// them to its views through the environment.
@MainActor
public protocol NativeServices {
    /// A Finder window with the item at `path` selected.
    func revealInFinder(_ path: String)

    func copy(_ text: String)

    /// The store edition's open panel, starting at the home folder. Returns the folder chosen, or
    /// `nil` when the user cancels.
    func chooseFolder() -> String?

    /// Open a lozenge's link in the app registered for it: an agent's session, a unit in the
    /// browser.
    func open(link: String)

    /// The folder in a window of the app that opens folders, Finder unless the user chose another.
    func openInFinder(_ folder: String)

    /// Open `folder` in `app`.
    func open(_ folder: String, in app: Native.FolderApp)

    /// Finder, which a card's folder opens in.
    var finder: Native.FolderApp? { get }

    /// The editors on this Mac that open a folder as a project, in the order Settings lists them.
    var editors: [Native.FolderApp] { get }

    /// The terminals on this Mac that open a window or tab whose shell starts in a folder, in the
    /// order Settings lists them: Terminal first.
    var terminals: [Native.FolderApp] { get }

    /// Whether Sutra opens when the user logs in.
    var opensAtLogin: Bool { get }

    /// Ask macOS to open Sutra at login, or to stop, and say whether it now will.
    func setOpensAtLogin(_ open: Bool) -> Bool
}

extension EnvironmentValues {
    /// The services the window's views reach: their model's, which the window's roots hand down.
    /// A view drawn away from the window, as a tool draws a card from fixtures, reaches none.
    @Entry var native: any NativeServices = NativeStandIn()
}

/// macOS's own services. All are public API that works inside the App Sandbox.
public struct Native: NativeServices {
    nonisolated static let log = Logger(
        subsystem: "systems.synesthetic.sutra",
        category: "native"
    )

    public init() {}

    public func revealInFinder(_ path: String) {
        NSWorkspace.shared.activateFileViewerSelecting([URL(filePath: path)])
    }

    public func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    /// The panel's own grant lets Sutra read the folder until it quits; the core keeps it for
    /// later launches.
    public func chooseFolder() -> String? {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.directoryURL = Self.home
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

    public func open(link: String) {
        guard let url = URL(string: link) else {
            Self.log.error("Not a link: \(link, privacy: .public)")
            return
        }
        if !NSWorkspace.shared.open(url) {
            Self.log.error("No app opened \(link, privacy: .public)")
        }
    }

    /// macOS keeps this, and the user can change it in System Settings too.
    public var opensAtLogin: Bool {
        SMAppService.mainApp.status == .enabled
    }

    /// macOS may hold a new login item until the user approves it, so System Settings then opens
    /// at Login Items.
    public func setOpensAtLogin(_ open: Bool) -> Bool {
        do {
            if open {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            Self.log.error("Changing the login item failed: \(error, privacy: .public)")
        }
        if open, SMAppService.mainApp.status == .requiresApproval {
            SMAppService.openSystemSettingsLoginItems()
        }
        return opensAtLogin
    }
}
