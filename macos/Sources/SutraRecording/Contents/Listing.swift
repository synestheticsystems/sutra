import Foundation
import SutraCore

/// How the user has arranged the list during a take: its sort, the groups folded shut, and whether
/// the quiet projects show. A take opens as the app's first launch does: newest activity first,
/// every repository's group open, the quiet projects behind their row. The invented work keeps no
/// scratch, so no group starts folded.
struct Arrangement: Equatable {
    var sort = SortOrder.activity
    /// The parents whose groups are folded shut.
    var folded: Set<Parent> = []
    var quietShown = false

    /// Fold the group of `parent` shut, or open it again.
    mutating func toggleFold(of parent: Parent) {
        if folded.remove(parent) == nil {
            folded.insert(parent)
        }
    }

    /// The list's rows at `moment`, as the live core lays them out: the projects in focus in the
    /// sort, then the row standing for the quiet projects, with them below it while it is open.
    func rows(at moment: TimeInterval) -> [ListRow] {
        let focused = Scenario.projects.filter { $0.inFocus(at: moment) }
        let quiet = Scenario.projects.filter { !$0.inFocus(at: moment) }
        var rows = layOut(focused, at: moment)
        if !quiet.isEmpty {
            rows.append(.quietProjects(
                count: UInt32(quiet.count), open: quietShown, label: "\(quiet.count) quiet",
                tooltip: Project.whatMakesOneQuiet
            ))
            if quietShown {
                rows += arranged(quiet, at: moment).map { project in
                    .card(card: project.card(naming: .alone, at: moment))
                }
            }
        }
        return rows
    }

    /// `projects` as the sort lays them out above the quiet row. The Activity sort gathers the
    /// worktrees of a repository with two or more of them under its line, which takes the place
    /// its newest card would; the other sorts list every card alone.
    private func layOut(_ projects: [Project], at moment: TimeInterval) -> [ListRow] {
        guard sort == .activity else {
            return arranged(projects, at: moment).map { project in
                .card(card: project.card(naming: .alone, at: moment))
            }
        }
        var ranked: [(rank: ActivityRank, row: ListRow)] = []
        func alone(_ project: Project) {
            let card = project.card(naming: .alone, at: moment)
            ranked.append((project.activityRank(at: moment), .card(card: card)))
        }
        var repositories: [String: Repository] = [:]
        for project in projects {
            if let worktree = project.worktree {
                repositories[worktree.commonDir, default: Repository(worktree: worktree)]
                    .cards.append(project)
            } else {
                alone(project)
            }
        }
        for repository in repositories.values {
            // A line over one card would only take room, so that card stands alone.
            if let card = repository.onlyCard {
                alone(card)
            } else {
                let line = row(of: repository, at: moment)
                ranked.append((repository.activityRank(at: moment), line))
            }
        }
        return ranked.sorted { $0.rank < $1.rank }.map(\.row)
    }

    /// The line of `repository`: open, its cards nested under it, the main worktree first;
    /// folded shut, the robots of the agents in them, throbbing while one of them waits.
    private func row(of repository: Repository, at moment: TimeInterval) -> ListRow {
        let cards = repository.cards.sorted {
            $0.nestedRank(at: moment) < $1.nestedRank(at: moment)
        }
        let commonDir = repository.worktree.commonDir
        let parent = Parent.repository(commonDir: commonDir)
        let fold: GroupFold = if folded.contains(parent) {
            .folded(
                sessions: cards.flatMap { $0.sessionBadges(at: moment) },
                subagents: cards.flatMap { $0.subagentBadges(at: moment) },
                waiting: cards.contains { $0.waiting(at: moment) }
            )
        } else {
            .open(cards: cards.map { $0.card(naming: .nested, at: moment) })
        }
        // Every worktree of the repository adds to the total, listed or not.
        let worktrees = Scenario.projects.filter { $0.worktree?.commonDir == commonDir }
        let folder = repository.worktree.repositoryFolder
        let line = GroupLine(
            parent: parent,
            name: (folder as NSString).lastPathComponent,
            tooltip: Words.tilde(folder),
            scratch: nil,
            total: Words.size(worktrees.reduce(0) { $0 + $1.bytes })
        )
        return .group(line: line, fold: fold)
    }

    /// `projects` in the sort, every card alone, as the quiet section lists them in every sort.
    /// Projects the sort cannot tell apart keep the order of the live core's map.
    private func arranged(_ projects: [Project], at moment: TimeInterval) -> [Project] {
        let listed = Array(projects.enumerated())
        let sorted = switch sort {
        case .activity:
            listed.sorted {
                $0.element.activityRank(at: moment) < $1.element.activityRank(at: moment)
            }
        case .name:
            listed.sorted {
                ($0.element.name.lowercased(), $0.offset)
                    < ($1.element.name.lowercased(), $1.offset)
            }
        case .disk:
            listed.sorted {
                $0.element.bytes == $1.element.bytes
                    ? $0.offset < $1.offset
                    : $0.element.bytes > $1.element.bytes
            }
        }
        return sorted.map(\.element)
    }
}

/// Where the Activity sort stands a card, or a repository's line: the newest five-minute slice of
/// the clock first, each slice in the order of the names they go by, ignoring case, and two of one
/// name by their folders, so that no two tie.
private typealias ActivityRank = (newestFirst: Int, name: String, folder: String)

/// A repository's worktrees above the quiet row, as the Activity sort gathers them.
private struct Repository {
    /// The worktree of the first card gathered, whose git folder all of them share.
    let worktree: Worktree
    var cards: [Project] = []

    /// The card it gathered, while it gathered only one.
    var onlyCard: Project? {
        cards.count == 1 ? cards[0] : nil
    }

    /// Its line goes by the repository's folder, and takes the place of its newest card.
    func activityRank(at moment: TimeInterval) -> ActivityRank {
        // A repository has its line only once it gathers two cards.
        let newest = cards.map { $0.slice(at: moment) }.max()!
        let folder = worktree.repositoryFolder
        return (-newest, (folder as NSString).lastPathComponent.lowercased(), worktree.commonDir)
    }
}

extension Project {
    /// What makes a project quiet, with the focus's thresholds spelled out.
    fileprivate static let whatMakesOneQuiet = "No agent session, no dev-runner environment, "
        + "no activity in the last \(Int(focusRecent / 86_400)) days, and less than "
        + "\(Words.size(focusLargeBytes)) on disk"

    /// How recent its last activity must be to keep a project in focus.
    private static let focusRecent: TimeInterval = 7 * 86_400
    /// How much its folder must hold to keep a project in focus on size alone.
    private static let focusLargeBytes: UInt64 = 1 << 30

    /// Whether the live core keeps it in focus at `moment`: it has an agent or a dev environment,
    /// activity within the last week, or a folder holding a gigabyte or more.
    fileprivate func inFocus(at moment: TimeInterval) -> Bool {
        !sessions.isEmpty || !subagents(at: moment).isEmpty || !units.isEmpty
            || moment - lastActivity(at: moment) <= Self.focusRecent
            || bytes >= Self.focusLargeBytes
    }

    /// The five-minute slice of the clock holding its last activity at `moment`.
    fileprivate func slice(at moment: TimeInterval) -> Int {
        Int((Scenario.opening.timeIntervalSince1970 + lastActivity(at: moment)) / 300)
    }

    fileprivate func activityRank(at moment: TimeInterval) -> ActivityRank {
        (-slice(at: moment), name.lowercased(), path)
    }

    /// Where it stands under its repository's line: the main worktree first, then by slice, each
    /// slice in the order of the branches the cards go by there.
    fileprivate func nestedRank(at moment: TimeInterval) -> (Int, Int, String, String) {
        (worktree?.linkedName == nil ? 0 : 1, -slice(at: moment),
         (worktree?.branch ?? name).lowercased(), path)
    }
}
