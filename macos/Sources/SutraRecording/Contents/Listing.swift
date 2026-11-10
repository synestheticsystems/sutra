import Foundation
import SutraCore

/// How the user has arranged the list during a take: its sort, the lines opened, and whether the
/// quiet projects show. Every line starts folded, as the app's do, and a take opens with Cedar
/// Web's line open, as a user who opened it to watch its worktrees left it: newest activity first,
/// the quiet projects behind their row.
struct Arrangement: Equatable {
    var sort = SortOrder.activity
    /// The lines opened.
    var opened: Set<LineKey> = [Scenario.cedarWebLine]
    var quietShown = false

    /// Fold the line of `line` shut, or open it.
    mutating func toggleFold(of line: LineKey) {
        if opened.remove(line) == nil {
            opened.insert(line)
        }
    }

    /// The list's rows at `moment`, as the live core lays them out: the groups in focus in the
    /// sort, each a line and while it is open its cards, then the row standing for the quiet
    /// groups, with them below it while it is open.
    func rows(at moment: TimeInterval) -> [ListRow] {
        let groups = Group.of(Scenario.projects)
        let focused = groups.filter { $0.inFocus(in: sort, at: moment) }
        let quiet = groups.filter { !$0.inFocus(in: sort, at: moment) }
        var rows = arranged(focused, at: moment).flatMap { rowsOf($0, at: moment) }
        if !quiet.isEmpty {
            let count = quiet.reduce(0) { $0 + $1.projects.count }
            rows.append(.quietProjects(
                count: UInt32(count), open: quietShown, label: "\(count) quiet",
                tooltip: Project.whatMakesOneQuiet(in: sort)
            ))
            if quietShown {
                rows += arranged(quiet, at: moment).flatMap { rowsOf($0, at: moment) }
            }
        }
        return rows
    }

    /// The rows of `group` at `moment`: its line, then, while it is open, its cards under it.
    private func rowsOf(_ group: Group, at moment: TimeInterval) -> [ListRow] {
        let key = group.key
        let open = opened.contains(key)
        let cards = group.cardsInOrder(sort, at: moment)
        let line = Line(
            key: key,
            name: group.name,
            tooltip: group.remote ?? "What no upstream groups",
            total: Words.size(group.projects.reduce(0) { $0 + $1.bytes }),
            whenIdleMark: nil,
            menu: [],
            fold: open ? .open : folded(cards, at: moment)
        )
        var rows: [ListRow] = [.line(line: line, place: Place(depth: 0, first: false))]
        if open {
            for (index, project) in cards.enumerated() {
                let card = project.card(naming: group.naming, at: moment)
                rows.append(.card(card: card, place: Place(depth: 1, first: index == 0)))
            }
        }
        return rows
    }

    /// What a line folded shut shows of `cards` at `moment`: their robots, the throb while one
    /// waits, and the first two of their servers, every unit of the invented work being up, each
    /// naming its card's folder in its tip and counting itself and those after it should the line
    /// need its room, with the rest counted.
    private func folded(_ cards: [Project], at moment: TimeInterval) -> LineFold {
        let units = cards.flatMap { card in card.units.map { (unit: $0, folder: card.path) } }
        let counting = { (from: Int) -> MoreLozenges in
            let counted = units[from...]
            return MoreLozenges(
                label: "+\(counted.count)",
                tooltip: counted.map { "\($0.unit.name) :\($0.unit.port)" }
                    .joined(separator: "\n")
            )
        }
        let drawn = min(2, units.count)
        return .folded(
            sessions: cards.flatMap { $0.sessionBadges(at: moment) },
            subagents: cards.flatMap { $0.subagentBadges(at: moment) },
            waiting: cards.contains { $0.waiting(at: moment) },
            lozenges: (0..<drawn).map { index in
                LineLozenge(
                    lozenge: units[index].unit.lozenge(onALineOver: units[index].folder),
                    givesWayTo: counting(index)
                )
            },
            moreLozenges: drawn < units.count ? counting(drawn) : nil
        )
    }

    /// `groups` in the sort: by their newest work, by name with Other last, or by the disk they
    /// take, groups that take alike keeping the Activity order.
    private func arranged(_ groups: [Group], at moment: TimeInterval) -> [Group] {
        let byActivity = groups.sorted {
            ($0.newest(at: moment), $1.name.lowercased()) > ($1.newest(at: moment), $0.name.lowercased())
        }
        return switch sort {
        case .activity:
            byActivity
        case .name:
            groups.sorted {
                ($0.remote == nil ? 1 : 0, $0.name.lowercased())
                    < ($1.remote == nil ? 1 : 0, $1.name.lowercased())
            }
        case .disk:
            Array(byActivity.enumerated()).sorted {
                $0.element.bytes == $1.element.bytes
                    ? $0.offset < $1.offset
                    : $0.element.bytes > $1.element.bytes
            }.map(\.element)
        }
    }
}

/// The projects of one upstream, or of none, under one line, as the live core groups them.
private struct Group {
    /// The remote they were cloned from; `nil` for Other, which the folders in no repository stand
    /// in.
    let remote: String?
    /// In the order the live core's map lists them, newest activity first.
    let projects: [Project]

    /// The groups of `projects`, in the order their first projects come.
    static func of(_ projects: [Project]) -> [Group] {
        var remotes: [String?] = []
        for project in projects where !remotes.contains(project.worktree?.remote) {
            remotes.append(project.worktree?.remote)
        }
        return remotes.map { remote in
            Group(remote: remote, projects: projects.filter { $0.worktree?.remote == remote })
        }
    }

    var key: LineKey {
        .group(kind: remote.map { .upstream(remote: $0) } ?? .other)
    }

    /// Its upstream's own name, as the live core names a group's line.
    var name: String {
        remote.map { ($0 as NSString).lastPathComponent } ?? "Other"
    }

    var bytes: UInt64 {
        projects.reduce(0) { $0 + $1.bytes }
    }

    /// The checkout the group's line stands for too: the one main worktree, when the group holds it
    /// and its linked worktrees alone, which then hang from the group's line, the main worktree's
    /// card first.
    private var checkout: Project? {
        let mains = projects.filter { $0.worktree?.linkedName == nil }
        guard mains.count == 1, let main = mains.first, projects.count > 1,
              projects.allSatisfy({ $0.worktree?.commonDir == main.worktree?.commonDir })
        else { return nil }
        return main
    }

    /// What its cards go by: under their repository's own line, their branches; else their
    /// folders' names.
    var naming: Naming {
        checkout == nil ? .folder : .branch
    }

    /// Its cards in `sort` at `moment`, the checkout its line stands for first.
    func cardsInOrder(_ sort: SutraCore.SortOrder, at moment: TimeInterval) -> [Project] {
        let first = checkout.map { [$0] } ?? []
        let rest = projects.filter { $0.path != checkout?.path }
        let name = { (project: Project) in
            (naming == .branch ? project.worktree?.branch : nil) ?? project.name
        }
        let sorted = switch sort {
        case .activity:
            rest.sorted {
                ($0.slice(at: moment), name($1).lowercased())
                    > ($1.slice(at: moment), name($0).lowercased())
            }
        case .name:
            rest.sorted { name($0).lowercased() < name($1).lowercased() }
        case .disk:
            rest.sorted { $0.bytes > $1.bytes }
        }
        return first + sorted
    }

    /// Whether the live core keeps it in focus in `sort` at `moment`: any of its projects is.
    func inFocus(in sort: SutraCore.SortOrder, at moment: TimeInterval) -> Bool {
        projects.contains { $0.inFocus(in: sort, at: moment) }
    }

    /// The newest five-minute slice of its projects' work at `moment`.
    func newest(at moment: TimeInterval) -> Int {
        projects.map { $0.slice(at: moment) }.max() ?? .min
    }
}

extension Project {
    /// What makes a group quiet in `sort`, with the focus's thresholds spelled out, as the live
    /// core writes it.
    fileprivate static func whatMakesOneQuiet(in sort: SutraCore.SortOrder) -> String {
        let days = Int(focusRecent / 86_400)
        return switch sort {
        case .activity:
            "Groups with no agent session, no dev-runner environment, nothing listening on a port, "
                + "no container running, and no activity in the last \(days) days. Scratch from "
                + "Claude Code sessions that have ended waits here too, as does Docker while no "
                + "container runs."
        case .name, .disk:
            "Groups with no agent session, no dev-runner environment, nothing listening on a port, "
                + "no container running, no activity in the last \(days) days, and no folder of "
                + "\(Words.size(focusLargeBytes)) or more"
        }
    }

    /// How recent its last activity must be to keep a project in focus.
    private static let focusRecent: TimeInterval = 7 * 86_400
    /// How much its folder must hold to keep a project in focus on size alone, where the sort
    /// weighs size.
    private static let focusLargeBytes: UInt64 = 1 << 30

    /// Whether the live core keeps it in focus in `sort` at `moment`: it has an agent or a dev
    /// environment, activity within the last week, or, by name or by size, a folder holding a
    /// gigabyte or more. The invented work keeps no scratch, so none waits for its session's end.
    fileprivate func inFocus(in sort: SutraCore.SortOrder, at moment: TimeInterval) -> Bool {
        let large = sort != .activity && bytes >= Self.focusLargeBytes
        return !sessions.isEmpty || !subagents(at: moment).isEmpty || !units.isEmpty
            || moment - lastActivity(at: moment) <= Self.focusRecent
            || large
    }

    /// The five-minute slice of the clock holding its last activity at `moment`.
    fileprivate func slice(at moment: TimeInterval) -> Int {
        Int((Scenario.opening.timeIntervalSince1970 + lastActivity(at: moment)) / 300)
    }
}
