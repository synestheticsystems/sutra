import Foundation
import SutraCore

/// What a card goes by, which its place on the list decides, as the live core decides it.
enum Naming {
    /// Under its repository's own line: its branch, which needs no chip.
    case branch
    /// Anywhere else: its folder's name, beside its branch on a chip.
    case folder
}

extension Project {
    /// Its card at `moment` of the take, going by what `naming` says, as the live core writes a
    /// card. Its right-click menu is left out: nothing on it has anything real to act on.
    func card(naming: Naming, at moment: TimeInterval) -> Card {
        let now = Scenario.opening + moment
        let activity = Scenario.opening + lastActivity(at: moment)
        let source = sessions.isEmpty ? "from git" : "from an agent session"
        let folder = "\(Words.tilde(path))\nLast activity \(Words.timestamp(activity)), \(source)"
        let byBranch = naming == .branch ? worktree?.branch : nil
        return Card(
            path: path,
            location: Words.tilde(path),
            name: byBranch ?? name,
            nameTooltip: byBranch.map { "\($0)\n\(folder)" } ?? folder,
            chip: byBranch == nil ? worktree.map(\.chip) : nil,
            age: Words.age(of: activity, at: now),
            unreadable: nil,
            lozenges: units.map(\.lozenge),
            tray: Tray(
                disk: diskUse,
                sessions: sessionBadges(at: moment),
                subagents: subagentBadges(at: moment)
            ),
            waiting: waiting(at: moment),
            menu: []
        )
    }

    /// The robots of its sessions at `moment`, in its order.
    func sessionBadges(at moment: TimeInterval) -> [AgentBadge] {
        let atWork = subagents(at: moment)
        return sessions.map { session in
            session.badge(
                at: moment, subagentsAtWork: atWork.count(where: { $0.worksFor == session.label })
            )
        }
    }

    /// The robots of its subagents at work at `moment`.
    func subagentBadges(at moment: TimeInterval) -> [AgentBadge] {
        subagents(at: moment).map(\.badge)
    }

    /// The folder's disk use: its whole size, then its build folders of a gigabyte or more, largest
    /// first, the smaller ones in the total's tooltip.
    private var diskUse: DiskUse {
        let worthShowing: UInt64 = 1 << 30
        let shown = buildFolders.filter { $0.bytes >= worthShowing }.sorted { $0.bytes > $1.bytes }
        let offTheCard = buildFolders.filter { $0.bytes < worthShowing }
        return DiskUse(
            total: Words.size(bytes),
            totalTooltip: offTheCard.isEmpty
                ? nil
                : offTheCard.map { "\($0.name)/ \(Words.size($0.bytes))" }.joined(separator: "\n"),
            warning: nil,
            entries: shown.map { folder in
                let location = Words.tilde("\(path)/\(folder.name)")
                return TrayEntry(
                    label: "\(folder.name)/",
                    tooltip: "\(Words.size(folder.bytes))\n\(location)",
                    cleaning: false
                )
            },
            progress: nil
        )
    }
}

extension Worktree {
    /// The branch chip: the branch, with which worktree of which repository this is in its
    /// tooltip.
    var chip: Chip {
        let which = linkedName.map {
            "Linked worktree \($0) of \(Words.tilde(repositoryFolder))"
        } ?? "Main worktree"
        return .branch(text: branch, tooltip: "\(branch)\n\(which)")
    }
}

extension Unit {
    /// Its lozenge, up and ready, which opens its port in the browser on a click.
    var lozenge: Lozenge {
        Lozenge(
            name: name, port: ":\(port)", look: .up, beside: nil, muted: false,
            notificationsOff: false, tooltip: "ready", link: "http://localhost:\(port)"
        )
    }

    /// Its lozenge on a folded line, whose tip names first the folder it serves in, `folder`, as the
    /// live core's does.
    func lozenge(onALineOver folder: String) -> Lozenge {
        var lozenge = self.lozenge
        lozenge.tooltip = "\(name) in \(Words.tilde(folder))\n\(lozenge.tooltip)"
        return lozenge
    }
}

extension Session {
    /// Its robot at `moment`, the light of its face saying what it does, with that in words in its
    /// tooltip, then its label, and its harness and model. It runs in a terminal, so a click opens
    /// nothing.
    func badge(at moment: TimeInterval, subagentsAtWork: Int) -> AgentBadge {
        let phase = phase(at: moment)
        var status = switch phase.status {
        case .working: "Busy"
        case .waitingForPermission: "waiting for permission"
        case .idle: "Idle"
        }
        if phase.status == .waitingForPermission,
           let waited = Words.wholeUnits(
               from: Scenario.opening + phase.from, to: Scenario.opening + moment
           ) {
            status += " (\(waited))"
        }
        if subagentsAtWork > 0 {
            status += ", \(Words.plural(subagentsAtWork, "subagent")) at work"
        }
        let look: AgentLook = switch phase.status {
        case .working: .working
        case .waitingForPermission: .waiting
        case .idle: .still
        }
        return AgentBadge(
            robot: robot, look: look, tooltip: "\(status)\n\(label)\n\(harness) \u{b7} \(model)",
            link: nil
        )
    }
}

extension Subagent {
    /// Its robot, at work, with its task, kind and model, and the session it works for, in its
    /// tooltip.
    var badge: AgentBadge {
        AgentBadge(
            robot: robot, look: .working,
            tooltip: "Busy\n\(task)\n\(agentType) \u{b7} \(model) \u{b7} for \(worksFor)",
            link: nil
        )
    }
}
