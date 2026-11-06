import Foundation
import SutraCore

/// The invented work a recording shows, in folders under an invented home that no Mac has, so
/// nothing in it comes from a real machine. Cedar Web has three worktrees at work, the main one
/// serving its site on 5173; Harbor API works on a cache and serves on 3000; Lantern Docs was
/// touched earlier in the day; three quiet projects wait behind their row. As the take plays, the
/// layout fix stops to ask permission and works on once it has it, and the search page finishes.
/// Moments are seconds into the take, which opens as the user continues past the welcome screen.
enum Scenario {
    /// The invented home, which the window writes `~`.
    static let home = "/Demo"

    /// The moment each take opens on: Thursday 5 November 2026, 17:00:05 UTC. It falls five
    /// seconds into one of the five-minute slices of the clock the Activity sort ranks by, so no
    /// card changes its place during a take of under five minutes.
    static let opening = Date(timeIntervalSince1970: 1_793_898_005)

    /// The welcome dance's seed: the one the approved artwork was exported with, so every take
    /// dances as the reviewed animation does.
    static let danceSeed: UInt64 = 33

    // Each cue falls on a whole second, as the script's clock looks again each whole second.

    /// The layout fix's session stops to ask permission, and its card throbs.
    static let permissionAsked: TimeInterval = 6
    /// It has its permission, and works on.
    static let permissionGiven: TimeInterval = 20
    /// The search page's subagents finish, and its session's turn ends with them.
    static let searchFinished: TimeInterval = 24

    /// In the order the live core's map lists them, newest activity first.
    static let projects = [
        cedarWeb, cedarWebSearch, cedarWebLayout, harborAPI, lanternDocs, atlasNotes, orchardKit,
        pineScripts,
    ]

    private static let cedarWebGit = "/Demo/Projects/cedar-web/.git"
    private static let day: TimeInterval = 86_400

    private static let cedarWeb = Project(
        path: "/Demo/Projects/cedar-web",
        worktree: Worktree(commonDir: cedarWebGit, branch: "main", linkedName: nil),
        units: [Unit(name: "web", port: 5173)],
        sessions: [
            Session(
                robot: .claude, harness: "Claude", model: "claude-opus-5-5",
                label: "Router cleanup",
                phases: [Phase(from: -1_500, status: .working)]
            ),
        ],
        subagents: [],
        ownBytes: 900_000_000,
        buildFolders: [BuildFolder(name: "node_modules", bytes: 1_600_000_000)],
        activeAt: -1_500
    )

    private static let cedarWebSearch = Project(
        path: "/Demo/Projects/cedar-web-search",
        worktree: Worktree(
            commonDir: cedarWebGit, branch: "feature/search", linkedName: "cedar-web-search"
        ),
        units: [],
        sessions: [
            Session(
                robot: .claude, harness: "Claude", model: "claude-opus-5-5",
                label: "Search results page",
                phases: [
                    Phase(from: -2_400, status: .working),
                    Phase(from: searchFinished, status: .idle),
                ]
            ),
        ],
        subagents: [
            Subagent(
                robot: .claude, agentType: "general-purpose", model: "claude-sonnet-5-5",
                task: "Index the docs for search", worksFor: "Search results page",
                until: searchFinished
            ),
            Subagent(
                robot: .claude, agentType: "general-purpose", model: "claude-sonnet-5-5",
                task: "Write the result card tests", worksFor: "Search results page",
                until: searchFinished
            ),
        ],
        ownBytes: 400_000_000,
        buildFolders: [BuildFolder(name: "node_modules", bytes: 1_200_000_000)],
        activeAt: -2_400
    )

    private static let cedarWebLayout = Project(
        path: "/Demo/Projects/cedar-web-layout",
        worktree: Worktree(
            commonDir: cedarWebGit, branch: "fix/layout", linkedName: "cedar-web-layout"
        ),
        units: [],
        sessions: [
            Session(
                robot: .claude, harness: "Claude", model: "claude-sonnet-5-5",
                label: "Fix the sidebar overflow",
                phases: [
                    Phase(from: -600, status: .working),
                    Phase(from: permissionAsked, status: .waitingForPermission),
                    Phase(from: permissionGiven, status: .working),
                ]
            ),
        ],
        subagents: [],
        ownBytes: 300_000_000,
        buildFolders: [BuildFolder(name: "node_modules", bytes: 600_000_000)],
        activeAt: -600
    )

    private static let harborAPI = Project(
        path: "/Demo/Projects/harbor-api",
        worktree: Worktree(
            commonDir: "/Demo/Projects/harbor-api/.git", branch: "feature/cache", linkedName: nil
        ),
        units: [Unit(name: "api", port: 3000)],
        sessions: [
            Session(
                robot: .codex, harness: "Codex", model: "gpt-5.5",
                label: "Cache the rate lookups",
                phases: [Phase(from: -3_000, status: .working)]
            ),
        ],
        subagents: [],
        ownBytes: 200_000_000,
        buildFolders: [BuildFolder(name: "target", bytes: 6_800_000_000)],
        activeAt: -3_000
    )

    private static let lanternDocs = Project(
        path: "/Demo/Projects/lantern-docs",
        worktree: Worktree(
            commonDir: "/Demo/Projects/lantern-docs/.git", branch: "main", linkedName: nil
        ),
        units: [], sessions: [], subagents: [],
        ownBytes: 140_000_000, buildFolders: [],
        activeAt: -38 * 60
    )

    private static let atlasNotes = Project(
        path: "/Demo/Projects/atlas-notes",
        worktree: Worktree(
            commonDir: "/Demo/Projects/atlas-notes/.git", branch: "docs/onboarding",
            linkedName: nil
        ),
        units: [], sessions: [], subagents: [],
        ownBytes: 48_000_000, buildFolders: [],
        activeAt: -12 * day
    )

    private static let orchardKit = Project(
        path: "/Demo/Projects/orchard-kit",
        worktree: Worktree(
            commonDir: "/Demo/Projects/orchard-kit/.git", branch: "main", linkedName: nil
        ),
        units: [], sessions: [], subagents: [],
        ownBytes: 310_000_000, buildFolders: [],
        activeAt: -23 * day
    )

    private static let pineScripts = Project(
        path: "/Demo/Projects/pine-scripts",
        worktree: nil,
        units: [], sessions: [], subagents: [],
        ownBytes: 12_000_000, buildFolders: [],
        activeAt: -41 * day
    )
}
