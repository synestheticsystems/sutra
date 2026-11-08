import SutraCore

/// How the list reads as a take opens, as `shown` writes it: Cedar Web's line open over its three
/// worktrees, its main one first, the other lines folded, and the quiet row.
let takeOpening = [
    "cedar-web: open", "main", "feature/search", "fix/layout", "harbor-api: folded",
    "lantern-docs: folded", "quiet:3",
]

/// What the list in `contents` shows, a few words a row: a line as `name: open` or
/// `name: folded`, a card by its name, Docker's card as `docker`, and the quiet row as
/// `quiet:<count>`.
func shown(_ contents: WindowContents) -> [String] {
    guard case .rows(let rows) = contents.list else { return [] }
    return rows.map { row in
        switch row {
        case .line(let line, _):
            "\(line.name): " + (line.fold == .open ? "open" : "folded")
        case .card(let card, _):
            card.name
        case .docker:
            "docker"
        case .elsewhere:
            "elsewhere"
        case .quietProjects(let count, _, _, _):
            "quiet:\(count)"
        }
    }
}

/// The card in `contents` whose folder is `path`, wherever the list holds it.
func card(at path: String, in contents: WindowContents) -> Card? {
    guard case .rows(let rows) = contents.list else { return nil }
    let cards = rows.compactMap { row -> Card? in
        switch row {
        case .card(let card, _): card
        case .line, .docker, .elsewhere, .quietProjects: nil
        }
    }
    return cards.first { $0.path == path }
}
