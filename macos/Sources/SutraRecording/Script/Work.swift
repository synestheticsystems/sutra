import Foundation
import SutraCore

/// One project of the invented work: its folder, the repository it is a worktree of, its dev
/// environment's units, the agents at work in it, and what it holds on disk. Moments are seconds
/// into the take, negative before it opens.
struct Project {
    /// Its folder, under the invented home.
    let path: String
    /// The repository it is a worktree of; `nil` for a folder in none.
    let worktree: Worktree?
    let units: [Unit]
    let sessions: [Session]
    let subagents: [Subagent]
    /// What its own files hold, apart from its build folders.
    let ownBytes: UInt64
    let buildFolders: [BuildFolder]
    /// The moment of its last activity apart from its agents': its last commit or file change.
    let activeAt: TimeInterval

    var name: String {
        (path as NSString).lastPathComponent
    }

    /// What its folder holds, build folders and all.
    var bytes: UInt64 {
        buildFolders.reduce(ownBytes) { $0 + $1.bytes }
    }

    /// The moment of its last activity at `moment`: `moment` itself while one of its sessions
    /// works, as a working agent writes to its project all along, or else the latest of its own
    /// and the moments its sessions last changed what they were doing.
    func lastActivity(at moment: TimeInterval) -> TimeInterval {
        sessions.reduce(activeAt) { latest, session in
            let phase = session.phase(at: moment)
            return max(latest, phase.status == .working ? moment : phase.from)
        }
    }

    /// Its subagents still at work at `moment`.
    func subagents(at moment: TimeInterval) -> [Subagent] {
        subagents.filter { moment < $0.until }
    }

    /// Whether it has an agent session waiting on the user at `moment`.
    func waiting(at moment: TimeInterval) -> Bool {
        sessions.contains { $0.phase(at: moment).status == .waitingForPermission }
    }
}

/// A project's place in a git repository.
struct Worktree {
    /// The git folder its repository's worktrees share.
    let commonDir: String
    let branch: String
    /// A linked worktree's own name; `nil` for the main worktree.
    let linkedName: String?

    /// The folder the repository goes by: the main worktree's, which holds the git folder.
    var repositoryFolder: String {
        (commonDir as NSString).deletingLastPathComponent
    }
}

/// A dev-runner unit, up and ready for the whole take.
struct Unit {
    let name: String
    let port: Int
}

/// A build folder in a project.
struct BuildFolder {
    /// Its name inside the project: `target`, `node_modules`.
    let name: String
    let bytes: UInt64
}

/// An agent session, and how it spends the take.
struct Session {
    let robot: Robot
    /// Its harness and the model it works with, as its tooltip names them: `Claude`,
    /// `claude-opus-5-5`.
    let harness: String
    let model: String
    let label: String
    /// What it does from each moment on, in order, the first from before the take opens.
    let phases: [Phase]

    /// The phase it is in at `moment`. Its first begins before the take opens, so at every moment
    /// of the take it is in one.
    func phase(at moment: TimeInterval) -> Phase {
        phases.last { $0.from <= moment }!
    }
}

/// What a session does from a moment of the take until its next phase.
struct Phase {
    let from: TimeInterval
    let status: Status
}

/// What a session is doing, in the states the app shows.
enum Status {
    case working
    /// Stopped to ask the user's permission, as Claude Code's prompt does.
    case waitingForPermission
    /// At rest, its turn over.
    case idle
}

/// A subagent at work for one of the sessions in its project, until a moment of the take.
struct Subagent {
    let robot: Robot
    let agentType: String
    let model: String
    let task: String
    /// The label of the session it works for.
    let worksFor: String
    /// The moment it finishes, and leaves the tray.
    let until: TimeInterval
}
