import Foundation
import SutraCore

extension Arrangement {
    /// What the window shows at `moment` of the take, the list arranged so, as the live core
    /// shows the invented work in the store edition: its toolbar names the folder it watches, and
    /// nothing is idle to clean. The menus of the clean and of the cards are left out, as nothing
    /// on them has anything real to act on.
    func contents(at moment: TimeInterval) -> WindowContents {
        let waiting = Scenario.projects.flatMap(\.sessions).count(where: {
            $0.phase(at: moment).status == .waitingForPermission
        })
        return WindowContents(
            title: waiting == 0 ? "Sutra" : "Sutra \u{b7} \(waiting) waiting",
            dockBadge: waiting == 0 ? nil : "\(waiting)",
            toolbar: Toolbar(
                sort: SortControl(current: sort, choices: Self.sortChoices),
                filter: nil,
                clean: CleanControl(
                    offered: false, busy: false, tooltip: "No idle build folders to clean", menu: []
                ),
                muted: false,
                muteTooltip: "Mute all sounds",
                notificationsOff: false,
                notificationsTooltip: "Disable notifications",
                folder: "Watching ~ (not Desktop or Downloads)"
            ),
            banner: nil,
            stopped: nil,
            list: .rows(rows: rows(at: moment)),
            footer: nil,
            dialog: nil,
            settings: Self.settings
        )
    }

    private static let sortChoices = [
        SortChoice(order: .activity, label: "Activity", tooltip: "Newest activity first"),
        SortChoice(order: .name, label: "Name", tooltip: "By name"),
        SortChoice(order: .disk, label: "Disk", tooltip: "Largest folders first"),
    ]

    /// Never cleaning by itself, and watching Documents of the folders macOS guards, as the app
    /// starts out.
    private static let settings = SutraCore.Settings(
        keepFree: nil,
        keepFreeChoices: [nil, 25_000_000_000, 50_000_000_000, 100_000_000_000, 200_000_000_000]
            .map { bytes in
                KeepFreeChoice(
                    bytes: bytes,
                    label: bytes.map { "Keep \(Words.size($0)) Free" } ?? "Never Clean by Itself"
                )
            },
        privateFolders: [
            PrivateFolderSwitch(folder: "Desktop", watched: false),
            PrivateFolderSwitch(folder: "Documents", watched: true),
            PrivateFolderSwitch(folder: "Downloads", watched: false),
        ]
    )
}
