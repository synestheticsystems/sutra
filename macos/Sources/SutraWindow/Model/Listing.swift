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
    case elsewhere(heading: String, agents: [ElsewhereAgent], waiting: Bool)
    case quietProjects(open: Bool, label: String, tooltip: String)

    public enum ID: Hashable {
        case card(path: String)
        case elsewhere
        case quietProjects
    }

    public var id: ID {
        switch self {
        case .card(let model): .card(path: model.path)
        case .elsewhere: .elsewhere
        case .quietProjects: .quietProjects
        }
    }

    public static func == (lhs: Row, rhs: Row) -> Bool {
        switch (lhs, rhs) {
        // The same model is the same row, whatever its card now says: the card redraws itself.
        case (.card(let lhs), .card(let rhs)): lhs === rhs
        case (.elsewhere(let lhsHeading, let lhsAgents, let lhsWaiting),
              .elsewhere(let rhsHeading, let rhsAgents, let rhsWaiting)):
            lhsHeading == rhsHeading && lhsAgents == rhsAgents && lhsWaiting == rhsWaiting
        case (.quietProjects(let lhsOpen, let lhsLabel, let lhsTooltip),
              .quietProjects(let rhsOpen, let rhsLabel, let rhsTooltip)):
            lhsOpen == rhsOpen && lhsLabel == rhsLabel && lhsTooltip == rhsTooltip
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
/// with it.
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
            var kept: [String: CardModel] = [:]
            let shown = rows.map { row -> Row in
                switch row {
                case .card(let card):
                    let model = byPath[card.path] ?? CardModel(card)
                    model.show(card)
                    kept[card.path] = model
                    return .card(model)
                case .elsewhere(let heading, let agents, let waiting):
                    return .elsewhere(heading: heading, agents: agents, waiting: waiting)
                case .quietProjects(_, let open, let label, let tooltip):
                    return .quietProjects(open: open, label: label, tooltip: tooltip)
                }
            }
            byPath = kept
            return .rows(shown)
        }
    }
}
