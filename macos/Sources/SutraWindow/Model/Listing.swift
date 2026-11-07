import Observation
import SutraCore

/// The list as the window keeps it, or what stands in its place. It differs from the core's `List`
/// in one way: a card's row holds the card's own model, so the list redraws when the rows change
/// order or membership, and a card that changed in place redraws alone.
public enum Listing: Equatable {
    case noFolder(message: String, lost: String?)
    case scanning
    case empty(message: String)
    case rows([Row])
}

/// One row of the list.
public enum Row: Identifiable, Equatable {
    case card(CardModel)
    /// The cards that share a parent under its line: a repository's where two or more are listed
    /// together, a session's however few.
    case group(line: GroupLine, fold: Fold)
    case elsewhere(heading: String, lines: [ElsewhereLine], waiting: Bool)
    case quietProjects(open: Bool, label: String, tooltip: String)

    public enum ID: Hashable {
        case card(path: String)
        case group(parent: Parent)
        case elsewhere
        case quietProjects
    }

    public var id: ID {
        switch self {
        case .card(let model): .card(path: model.path)
        case .group(let line, _): .group(parent: line.parent)
        case .elsewhere: .elsewhere
        case .quietProjects: .quietProjects
        }
    }

    public static func == (lhs: Row, rhs: Row) -> Bool {
        switch (lhs, rhs) {
        // The same model is the same row, whatever its card now says: the card redraws itself.
        case (.card(let lhs), .card(let rhs)): lhs === rhs
        // A group is the same while its line reads the same over the same fold.
        case (.group(let lhsLine, let lhsFold), .group(let rhsLine, let rhsFold)):
            lhsLine == rhsLine && lhsFold == rhsFold
        case (.elsewhere(let lhsHeading, let lhsLines, let lhsWaiting),
              .elsewhere(let rhsHeading, let rhsLines, let rhsWaiting)):
            lhsHeading == rhsHeading && lhsLines == rhsLines && lhsWaiting == rhsWaiting
        case (.quietProjects(let lhsOpen, let lhsLabel, let lhsTooltip),
              .quietProjects(let rhsOpen, let rhsLabel, let rhsTooltip)):
            lhsOpen == rhsOpen && lhsLabel == rhsLabel && lhsTooltip == rhsTooltip
        default: false
        }
    }
}

/// What stands under a group's line, as the window keeps it: the models of the group's cards, or,
/// folded shut, the core's record of the agents in them, which the line shows.
public enum Fold: Equatable {
    case open([CardModel])
    case folded(sessions: [AgentBadge], subagents: [AgentBadge], waiting: Bool)

    public static func == (lhs: Fold, rhs: Fold) -> Bool {
        switch (lhs, rhs) {
        // The same models are the same cards, whatever they now say: each card redraws itself.
        case (.open(let lhsCards), .open(let rhsCards)):
            lhsCards.elementsEqual(rhsCards, by: ===)
        case (.folded(let lhsSessions, let lhsSubagents, let lhsWaiting),
              .folded(let rhsSessions, let rhsSubagents, let rhsWaiting)):
            lhsSessions == rhsSessions && lhsSubagents == rhsSubagents && lhsWaiting == rhsWaiting
        default: false
        }
    }
}

/// One project's card as the window keeps it, for as long as the project stays on the list.
@MainActor
@Observable
public final class CardModel {
    /// The project's folder, which the card is known by from one set of contents to the next.
    nonisolated let path: String
    private(set) var card: Card

    init(_ card: Card) {
        path = card.path
        self.card = card
    }

    /// Take the core's newest record of the card, redrawing it only when the record differs.
    func show(_ newer: Card) {
        if card != newer {
            card = newer
        }
    }
}

/// The models of the cards on the list, by folder. A card that leaves the list takes its model
/// with it, and a card that moves into a group or out of one keeps it.
@MainActor
struct CardModels {
    private var byPath: [String: CardModel] = [:]

    /// `list` as the window keeps it, reusing the model of every card still on the list.
    mutating func listing(of list: SutraCore.List) -> Listing {
        switch list {
        case .noFolder(let message, let lost):
            byPath = [:]
            return .noFolder(message: message, lost: lost)
        case .scanning:
            byPath = [:]
            return .scanning
        case .empty(let message):
            byPath = [:]
            return .empty(message: message)
        case .rows(let rows):
            let previous = byPath
            var kept: [String: CardModel] = [:]
            func model(of card: Card) -> CardModel {
                let model = previous[card.path] ?? CardModel(card)
                model.show(card)
                kept[card.path] = model
                return model
            }
            func models(under fold: GroupFold) -> Fold {
                switch fold {
                case .open(let cards):
                    .open(cards.map(model(of:)))
                case .folded(let sessions, let subagents, let waiting):
                    .folded(sessions: sessions, subagents: subagents, waiting: waiting)
                }
            }
            let shown = rows.map { row -> Row in
                switch row {
                case .card(let card):
                    return .card(model(of: card))
                case .group(let line, let fold):
                    return .group(line: line, fold: models(under: fold))
                case .elsewhere(let heading, let lines, let waiting):
                    return .elsewhere(heading: heading, lines: lines, waiting: waiting)
                case .quietProjects(_, let open, let label, let tooltip):
                    return .quietProjects(open: open, label: label, tooltip: tooltip)
                }
            }
            byPath = kept
            return .rows(shown)
        }
    }
}
