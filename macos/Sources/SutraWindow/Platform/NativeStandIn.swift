import os

/// A stand-in for macOS's services that opens, copies and changes nothing, so no click in a window
/// drawn from invented contents reaches the Mac it runs on. Its Finder and Terminal are macOS's
/// own, so a card's buttons for opening its folder draw as they do in the app, but it knows no
/// editors and no other terminals: which a Mac has is that Mac's own business. It logs each
/// service it declines.
public struct NativeStandIn: NativeServices {
    private nonisolated static let log = Logger(
        subsystem: "systems.synesthetic.sutra",
        category: "native stand-in"
    )

    /// Nonisolated, as the environment makes one wherever a view reads it without its model.
    public nonisolated init() {}

    public func revealInFinder(_ path: String) {
        declined("reveal \(path) in Finder")
    }

    public func copy(_ text: String) {
        declined("copy \(text)")
    }

    public func chooseFolder() -> String? {
        declined("show the open panel")
        return nil
    }

    public func open(link: String) {
        declined("open \(link)")
    }

    public func openInFinder(_ folder: String) {
        declined("open \(folder) in Finder")
    }

    public func open(_ folder: String, in app: Native.FolderApp) {
        declined("open \(folder) in \(app.name)")
    }

    public var finder: Native.FolderApp? {
        Native().finder
    }

    public var editors: [Native.FolderApp] {
        []
    }

    /// Terminal alone, which macOS's own list starts with.
    public var terminals: [Native.FolderApp] {
        Array(Native().terminals.prefix(1))
    }

    public var opensAtLogin: Bool {
        false
    }

    public func setOpensAtLogin(_ open: Bool) -> Bool {
        declined(open ? "open at login" : "stop opening at login")
        return false
    }

    private func declined(_ service: String) {
        Self.log.notice("Declined to \(service, privacy: .public)")
    }
}
