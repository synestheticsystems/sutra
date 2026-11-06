import SutraCore

/// How the list reads as a take opens, as `shown` writes it.
let takeOpening = [
    "cedar-web: main, feature/search, fix/layout", "harbor-api", "lantern-docs", "quiet:3",
]

/// What the list in `contents` shows, a few words a row, as the core's own tests write it: a
/// card's name, a repository's line as `name: card, card` or `name: folded`, and the quiet row as
/// `quiet:<count>`.
func shown(_ contents: WindowContents) -> [String] {
    guard case .rows(let rows) = contents.list else { return [] }
    return rows.map { row in
        switch row {
        case .card(let card):
            card.name
        case .repository(_, let name, _, _, .open(let cards)):
            "\(name): \(cards.map(\.name).joined(separator: ", "))"
        case .repository(_, let name, _, _, .folded):
            "\(name): folded"
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
    let cards = rows.flatMap { row -> [Card] in
        switch row {
        case .card(let card): [card]
        case .repository(_, _, _, _, .open(let cards)): cards
        case .repository, .elsewhere, .quietProjects: []
        }
    }
    return cards.first { $0.path == path }
}
