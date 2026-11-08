import SutraCore
@testable import SutraWindow
import Testing

/// Where the dashboard model sends a menu line: the window's own to the native services it holds,
/// the rest to the core.
@MainActor
struct DashboardModelTests {
    /// A core that lists nothing and keeps what it is asked to do.
    private final class Core: DashboardProtocol, @unchecked Sendable {
        var performed: [Action] = []

        func contents() -> WindowContents {
            WindowContents(
                title: "Sutra", dockBadge: nil,
                toolbar: Toolbar(
                    sort: nil, filter: nil,
                    clean: CleanControl(offered: false, busy: false, tooltip: "", menu: []),
                    muted: false, muteTooltip: "", notificationsOff: false,
                    notificationsTooltip: "", folder: nil, disk: nil
                ),
                banner: nil, stopped: nil, list: .scanning, footer: nil, dialog: nil,
                settings: SutraCore.Settings(keepFree: nil, keepFreeChoices: [], privateFolders: [])
            )
        }

        func perform(action: Action) {
            performed.append(action)
        }

        func stop() {}
    }

    /// Services that keep what they are asked to do, and whose open panel chooses `chosen`.
    private final class Services: NativeServices {
        var asked: [String] = []
        var chosen: String?

        func revealInFinder(_ path: String) { asked.append("reveal \(path)") }
        func copy(_ text: String) { asked.append("copy \(text)") }
        func chooseFolder() -> String? {
            asked.append("choose a folder")
            return chosen
        }
        func open(link: String) { asked.append("open \(link)") }
        func openInFinder(_ folder: String) { asked.append("open \(folder) in Finder") }
        func open(_ folder: String, in app: Native.FolderApp) { asked.append("open \(folder)") }
        var finder: Native.FolderApp? { nil }
        var editors: [Native.FolderApp] { [] }
        var terminals: [Native.FolderApp] { [] }
        var opensAtLogin: Bool { false }
        func setOpensAtLogin(_ open: Bool) -> Bool { false }
    }

    private let core = Core()
    private let services = Services()

    private func model() -> DashboardModel {
        DashboardModel(core: core, native: services)
    }

    @Test func revealingAndCopyingAPathAskTheServicesAndNotTheCore() {
        let model = model()
        model.choose(.revealInFinder(path: "/Demo/Projects/app"))
        model.choose(.copyPath(path: "/Demo/Projects/app"))
        #expect(services.asked == ["reveal /Demo/Projects/app", "copy /Demo/Projects/app"])
        #expect(core.performed.isEmpty)
    }

    @Test func theFolderChosenInTheOpenPanelGoesToTheCoreAndACancelSendsNothing() {
        let model = model()
        model.choose(.chooseFolder)
        #expect(core.performed.isEmpty)
        services.chosen = "/Demo"
        model.choose(.chooseFolder)
        #expect(services.asked == ["choose a folder", "choose a folder"])
        #expect(core.performed == [.folderGranted(folder: "/Demo")])
    }

    @Test func theCoresOwnMenuLinesGoToTheCore() {
        model().choose(.dashboard(action: .cleanIdle))
        #expect(core.performed == [.cleanIdle])
        #expect(services.asked.isEmpty)
    }
}
