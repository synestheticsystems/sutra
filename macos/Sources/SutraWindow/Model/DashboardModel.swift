import Observation
import SutraCore

/// What the window shows, kept as the core last described it, and the way back to the core for
/// what the user does. Each part of the contents is its own observed property, replaced only when
/// the core's record of it differs, so a change redraws only the views that read that part; each
/// card has a model of its own (see `Listing`), so a change to one card redraws that card alone.
@MainActor
@Observable
public final class DashboardModel {
    private(set) var title: String
    /// What the Dock badge says: how many agents wait on the user, or a warning once the map no
    /// longer updates.
    public private(set) var dockBadge: String?
    private(set) var toolbar: Toolbar
    private(set) var banner: Banner?
    /// The line over the list once the map no longer updates; nothing in the window moves then.
    private(set) var stopped: String?
    public private(set) var listing: Listing
    private(set) var footer: Footer?
    private(set) var settings: SutraCore.Settings
    /// The help panel shows in the list's place. The window's own state: the core never hears of it.
    private(set) var helpShown = false

    /// The core's dialog, until the user answers it here.
    private var receivedDialog: Dialog?
    /// The dialog the user answered last. The core drops it only once its answer is performed, and
    /// contents read in between must not raise it again.
    private var answeredDialog: UInt64?
    /// Where the current dialog shows: in the window the user was in when it came, so a question
    /// a choice in Settings raises is asked in Settings.
    private(set) var dialogPlace = DialogPlace.dashboard
    /// Whether the Settings window has the keyboard, as the Settings view reports it.
    @ObservationIgnored var settingsInFront = false

    @ObservationIgnored private let core: any DashboardProtocol
    /// The macOS services the window reaches itself, which its views reach too.
    @ObservationIgnored let native: any NativeServices
    @ObservationIgnored private var cards = CardModels()

    /// Reads what the window shows from `core` at once, and again on each `refresh`.
    public init(core: any DashboardProtocol, native: any NativeServices) {
        self.core = core
        self.native = native
        let contents = core.contents()
        var cards = CardModels()
        title = contents.title
        dockBadge = contents.dockBadge
        toolbar = contents.toolbar
        banner = contents.banner
        stopped = contents.stopped
        listing = cards.listing(of: contents.list)
        footer = contents.footer
        receivedDialog = contents.dialog
        settings = contents.settings
        self.cards = cards
    }

    /// The dialog to show: the core's oldest unanswered one, unless the user just answered it.
    var dialog: Dialog? {
        receivedDialog.flatMap { $0.id == answeredDialog ? nil : $0 }
    }

    /// Read the contents again, after the core said they changed.
    func refresh() {
        let contents = core.contents()
        replace(\.title, with: contents.title)
        replace(\.dockBadge, with: contents.dockBadge)
        replace(\.toolbar, with: contents.toolbar)
        replace(\.banner, with: contents.banner)
        replace(\.stopped, with: contents.stopped)
        replace(\.listing, with: cards.listing(of: contents.list))
        replace(\.footer, with: contents.footer)
        if contents.dialog?.id != receivedDialog?.id {
            replace(\.dialogPlace, with: settingsInFront ? .settings : .dashboard)
        }
        replace(\.receivedDialog, with: contents.dialog)
        replace(\.settings, with: contents.settings)
    }

    /// Assigning an equal value would still tell every view that reads the property to redraw.
    private func replace<Value: Equatable>(
        _ property: ReferenceWritableKeyPath<DashboardModel, Value>,
        with value: Value
    ) {
        if self[keyPath: property] != value {
            self[keyPath: property] = value
        }
    }

    func perform(_ action: Action) {
        core.perform(action: action)
    }

    func answer(_ dialog: Dialog, yes: Bool) {
        guard answeredDialog != dialog.id else { return }
        answeredDialog = dialog.id
        perform(.answer(dialog: dialog.id, yes: yes))
    }

    /// The Settings window closed, taking with it any dialog shown there, which the dashboard's
    /// window shows instead.
    func settingsClosed() {
        settingsInFront = false
        replace(\.dialogPlace, with: .dashboard)
    }

    public func toggleHelp() {
        helpShown.toggle()
    }

    /// ⌘F: the filter field acts on the list, so the help panel gives the list its place back.
    func showFilter() {
        helpShown = false
        perform(.showFilter)
    }

    /// Escape closes what is open: the help panel, or else the filter field. Returns whether there
    /// was anything to close, so a key that closed nothing goes on to the rest of the window.
    func escape() -> Bool {
        if helpShown {
            helpShown = false
            return true
        }
        if toolbar.filter != nil {
            perform(.hideFilter)
            return true
        }
        return false
    }

    /// Carry out a menu line: the window's own with its native services, the rest with the core.
    func choose(_ action: MenuAction) {
        switch action {
        case .revealInFinder(let path):
            native.revealInFinder(path)
        case .copyPath(let path):
            native.copy(path)
        case .chooseFolder:
            chooseFolder()
        case .dashboard(let action):
            perform(action)
        }
    }

    /// The store edition's open panel; the folder chosen goes to the core.
    func chooseFolder() {
        if let folder = native.chooseFolder() {
            perform(.folderGranted(folder: folder))
        }
    }
}
