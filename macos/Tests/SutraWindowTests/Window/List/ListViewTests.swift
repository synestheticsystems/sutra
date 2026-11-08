import AppKit
import SutraCore
@testable import SutraWindow
import SwiftUI
import Testing

/// The list in a real window, left alone: once it has laid out what it shows, it does no more work
/// until something changes.
@MainActor
struct ListViewTests {
    /// A core that only answers with fixed contents.
    private final class FixedCore: DashboardProtocol, @unchecked Sendable {
        let shown: WindowContents
        init(_ shown: WindowContents) { self.shown = shown }
        func contents() -> WindowContents { shown }
        func perform(action: Action) {}
        func stop() {}
    }

    /// The window the list spun in: the Activity sort scrolled into the middle of a checkout's open
    /// line of forty worktrees with long branch names, a few detached and so named by their folder
    /// beside a chip, under its group's open line, between other rows.
    @Test func aListScrolledIntoALongOpenGroupStopsLayingItselfOutOnceLeftAlone() throws {
        let dashboard = DashboardModel(core: FixedCore(Self.contents), native: NativeStandIn())
        let passes = LayoutPasses()
        let host = NSHostingView(rootView: CountingPasses(passes: passes) {
            ListView(dashboard: dashboard)
        })
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 1800), styleMask: [.titled],
            backing: .buffered, defer: false
        )
        window.contentView = host
        settle(for: 0.5)
        let scroller = try #require(scrollView(in: host))
        scroller.contentView.scroll(to: NSPoint(x: 0, y: 2_000))
        scroller.reflectScrolledClipView(scroller.contentView)
        settle(for: 1)
        #expect(passes.count > 0, "the list was laid out at all")

        let passesBefore = passes.count
        let timeBefore = mainThreadTime()
        settle(for: 1)
        #expect(passes.count == passesBefore, "no pass of layout comes while nothing changes")
        #expect(mainThreadTime() - timeBefore < 0.3, "the main thread rests")
    }

    /// Run the main run loop for `seconds`, so SwiftUI carries out its updates as in the app.
    private func settle(for seconds: TimeInterval) {
        RunLoop.main.run(until: Date(timeIntervalSinceNow: seconds))
    }

    private func scrollView(in view: NSView) -> NSScrollView? {
        (view as? NSScrollView) ?? view.subviews.lazy.compactMap(scrollView(in:)).first
    }

    /// The CPU time the main thread has used, in seconds.
    private func mainThreadTime() -> TimeInterval {
        var info = thread_basic_info()
        var count = mach_msg_type_number_t(
            MemoryLayout<thread_basic_info_data_t>.size / MemoryLayout<natural_t>.size
        )
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                thread_info(
                    pthread_mach_thread_np(pthread_self()), thread_flavor_t(THREAD_BASIC_INFO), $0,
                    &count
                )
            }
        }
        precondition(result == KERN_SUCCESS, "the main thread's times can be read")
        return [info.user_time, info.system_time].reduce(0) { total, time in
            total + TimeInterval(time.seconds) + TimeInterval(time.microseconds) / 1e6
        }
    }

    private static let contents = WindowContents(
        title: "Sutra",
        dockBadge: nil,
        toolbar: Toolbar(
            sort: SortControl(current: .activity, choices: []), filter: nil,
            clean: CleanControl(offered: false, busy: false, tooltip: "", menu: []),
            muted: false, muteTooltip: "", notificationsOff: false, notificationsTooltip: "",
            folder: nil, disk: nil
        ),
        banner: nil,
        stopped: nil,
        list: .rows(rows: [
            .line(line: folded(upstream("sutra"), "sutra"), place: top),
            .line(line: open(upstream("ankurah"), "ankurah"), place: top),
            .line(line: open(.checkout(folder: "/Users/me/code/ankurah"), "ankurah"),
                  place: Place(depth: 1, first: true)),
        ] + (0..<40).map { index in
            .card(card: worktree(index), place: Place(depth: 2, first: index == 0))
        } + [
            .line(line: open(.group(kind: .other), "Other"), place: top),
        ] + ["busyloop", "render"].enumerated().map { index, folder in
            .card(
                card: card("/tmp/claude/5f1c/scratchpad/\(folder)", name: folder),
                place: Place(depth: 1, first: index == 0)
            )
        } + (0..<20).map { index in
            .line(line: folded(upstream("app-\(index)"), "app-\(index)"), place: top)
        } + [
            .quietProjects(count: 390, open: false, label: "390 quiet", tooltip: ""),
        ]),
        footer: nil,
        dialog: nil,
        settings: SutraCore.Settings(keepFree: nil, keepFreeChoices: [], privateFolders: [])
    )

    /// The place of a group's line, atop the tree.
    private static let top = Place(depth: 0, first: false)

    /// The key of the group of the clones of `name`'s upstream.
    private static func upstream(_ name: String) -> LineKey {
        .group(kind: .upstream(remote: "github.com/me/\(name)"))
    }

    /// The open line of `key`, going by `name`.
    private static func open(_ key: LineKey, _ name: String) -> Line {
        Line(
            key: key, name: name, tooltip: "", total: "212 GB", cleansWhenIdle: nil, menu: [],
            fold: .open
        )
    }

    /// The line of `key`, going by `name`, folded shut over a robot at work.
    private static func folded(_ key: LineKey, _ name: String) -> Line {
        let fold = LineFold.folded(
            sessions: [robot], subagents: [], waiting: false, lozenges: [], moreLozenges: nil
        )
        return Line(
            key: key, name: name, tooltip: "", total: "212 GB", cleansWhenIdle: nil, menu: [],
            fold: fold
        )
    }

    /// The `index`th worktree of the open checkout's line: on a long branch, or every ninth
    /// detached and so named by its folder beside its chip, with a robot on every third.
    private static func worktree(_ index: Int) -> Card {
        let branch = ["feat/digest-trees-stored-fix\(index)-pg-sqlite",
                      "fix/postgres-connection-pool-exhaustion-\(index)",
                      "feat/live-query-diffing-for-ordered-results-\(index)"][index % 3]
        let folder = branch.replacingOccurrences(of: "/", with: "-")
        let detached = index % 9 == 4
        return card(
            "/Users/me/code/ankurah-wt/\(folder)", name: detached ? folder : branch,
            chip: detached ? .branch(text: "detached 4c0c1d2\(index)", tooltip: "") : nil,
            robots: index % 3 == 0 ? [robot] : []
        )
    }

    private static let main = Chip.branch(text: "main", tooltip: "")
    private static let robot = AgentBadge(robot: .claude, look: .working, tooltip: "", link: nil)

    private static func card(
        _ path: String, name: String, chip: Chip? = nil, robots: [AgentBadge] = []
    ) -> Card {
        Card(
            path: path, location: path, name: name, nameTooltip: "", chip: chip, age: "3h",
            unreadable: nil, lozenges: [],
            tray: Tray(
                disk: DiskUse(
                    total: "1.2 GB", totalTooltip: nil, warning: nil, entries: [], progress: nil
                ),
                sessions: robots, subagents: []
            ),
            waiting: false, menu: []
        )
    }
}

/// How many times SwiftUI has measured or placed the view at the root of a window.
@MainActor
private final class LayoutPasses {
    var count = 0
}

/// Its one subview as it is, counting each pass of layout through it. SwiftUI lays out on the main
/// thread.
private struct CountingPasses: Layout {
    let passes: LayoutPasses

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        MainActor.assumeIsolated { passes.count += 1 }
        return subviews.first?.sizeThatFits(proposal) ?? .zero
    }

    func placeSubviews(
        in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
    ) {
        MainActor.assumeIsolated { passes.count += 1 }
        subviews.first?.place(at: bounds.origin, proposal: proposal)
    }
}
