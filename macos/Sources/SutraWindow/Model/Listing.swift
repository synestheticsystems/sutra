import Observation
import SutraCore

/// The list as the window keeps it, or what stands in its place. It differs from the core's `List`
/// in one way: a card's row holds the card's own model, so the list redraws when the rows change
/// order or membership, and a card that changed in place redraws alone. The core lays the rows out
/// flat, a row for each line and for each card at its place in its group's tree, as a lazy list
/// lays out each row whole: a group of forty cards held in one row would be measured whole whenever
/// any of it came into view.
public enum Listing: Equatable {
    case noFolder(message: String, lost: String?)
    case scanning
    case empty(message: String)
    case rows([Row])
}

/// One row of the list.
public enum Row: Identifiable, Equatable {
    /// A line: a group's, which names the upstream its cards came from, or within a group a
    /// checkout's, which holds its own card and the copies made from it. Open, its rows follow it.
    case line(Line, place: Place)
    case card(CardModel, place: Place)
    /// Docker's card, alone under its group's line: the containers it runs for no project, and its
    /// disk use.
    case docker(DockerCard, place: Place)
    /// The agents working outside every project, which stand in Other.
    case elsewhere(heading: String, lines: [ElsewhereLine], waiting: Bool, place: Place)
    case quietProjects(open: Bool, label: String, tooltip: String)

    public enum ID: Hashable {
        case card(path: String)
        case line(LineKey)
        /// The one Docker card there is.
        case docker
        case elsewhere
        case quietProjects
    }

    public var id: ID {
        switch self {
        case .card(let model, _): .card(path: model.path)
        case .line(let line, _): .line(line.key)
        case .docker: .docker
        case .elsewhere: .elsewhere
        case .quietProjects: .quietProjects
        }
    }

    public static func == (lhs: Row, rhs: Row) -> Bool {
        switch (lhs, rhs) {
        // The same model is the same row, whatever its card now says: the card redraws itself.
        case (.card(let lhs, let lhsPlace), .card(let rhs, let rhsPlace)):
            lhs === rhs && lhsPlace == rhsPlace
        case (.line(let lhs, let lhsPlace), .line(let rhs, let rhsPlace)):
            lhs == rhs && lhsPlace == rhsPlace
        case (.docker(let lhs, let lhsPlace), .docker(let rhs, let rhsPlace)):
            lhs == rhs && lhsPlace == rhsPlace
        case (.elsewhere(let lhsHeading, let lhsLines, let lhsWaiting, let lhsPlace),
              .elsewhere(let rhsHeading, let rhsLines, let rhsWaiting, let rhsPlace)):
            lhsHeading == rhsHeading && lhsLines == rhsLines && lhsWaiting == rhsWaiting
                && lhsPlace == rhsPlace
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
/// with it, and a card that moves from one line to another keeps it.
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
            let shown = rows.map { row -> Row in
                switch row {
                case .card(let card, let place):
                    let model = previous[card.path] ?? CardModel(card)
                    model.show(card)
                    kept[card.path] = model
                    return .card(model, place: place)
                case .line(let line, let place):
                    return .line(line, place: place)
                case .docker(let card, let place):
                    return .docker(card, place: place)
                case .elsewhere(let heading, let lines, let waiting, let place):
                    return .elsewhere(heading: heading, lines: lines, waiting: waiting, place: place)
                case .quietProjects(_, let open, let label, let tooltip):
                    return .quietProjects(open: open, label: label, tooltip: tooltip)
                }
            }
            byPath = kept
            return .rows(shown)
        }
    }
}
