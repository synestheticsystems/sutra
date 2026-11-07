import Observation
import SutraCore

/// The list as the window keeps it, or what stands in its place. It differs from the core's `List`
/// in two ways. A card's row holds the card's own model, so the list redraws when the rows change
/// order or membership, and a card that changed in place redraws alone. And each card of an open
/// group is a row of its own under the group's line, as a lazy list lays out each row whole: a
/// group of forty cards held in one row would be measured whole whenever any of it came into view.
public enum Listing: Equatable {
    case noFolder(message: String, lost: String?)
    case scanning
    case empty(message: String)
    case rows([Row])
}

/// One row of the list.
public enum Row: Identifiable, Equatable {
    case card(CardModel)
    /// The line over the cards that share a parent: a repository's where two or more are listed
    /// together, a session's however few. Open, the line's cards follow it, each nested in a row of
    /// its own.
    case groupLine(line: GroupLine, fold: Fold)
    /// A card of an open group, under the group's line.
    case nested(CardModel, guide: GuideStretch)
    case elsewhere(heading: String, lines: [ElsewhereLine], waiting: Bool)
    case quietProjects(open: Bool, label: String, tooltip: String)

    public enum ID: Hashable {
        /// A card's, alone or nested.
        case card(path: String)
        case groupLine(parent: Parent)
        case elsewhere
        case quietProjects
    }

    public var id: ID {
        switch self {
        case .card(let model), .nested(let model, _): .card(path: model.path)
        case .groupLine(let line, _): .groupLine(parent: line.parent)
        case .elsewhere: .elsewhere
        case .quietProjects: .quietProjects
        }
    }

    public static func == (lhs: Row, rhs: Row) -> Bool {
        switch (lhs, rhs) {
        // The same model is the same row, whatever its card now says: the card redraws itself.
        case (.card(let lhs), .card(let rhs)): lhs === rhs
        case (.groupLine(let lhsLine, let lhsFold), .groupLine(let rhsLine, let rhsFold)):
            lhsLine == rhsLine && lhsFold == rhsFold
        case (.nested(let lhs, let lhsGuide), .nested(let rhs, let rhsGuide)):
            lhs === rhs && lhsGuide == rhsGuide
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

/// Whether a group's line stands open over its cards, or folded shut with the core's record of
/// the agents and the servers in them, which the line then shows.
public enum Fold: Equatable {
    case open
    case folded(
        sessions: [AgentBadge], subagents: [AgentBadge], waiting: Bool, units: [UnitLozenge],
        moreUnits: MoreUnits?
    )
}

/// A nested card's stretch of the guide its group's cards hang from, which the card's row draws
/// down the card and up through the room above it, so that the stretches of a group's rows join
/// into one line running down from the chevron of the group's line.
public struct GuideStretch: Equatable {
    /// In dots under a session's scratch, broken as its mark is; unbroken under a repository,
    /// whose worktrees are kept.
    let dotted: Bool
    /// The card hangs first under the line, so its stretch reaches on up into the line, to just
    /// below the chevron.
    let first: Bool
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
            var shown: [Row] = []
            for row in rows {
                switch row {
                case .card(let card):
                    shown.append(.card(model(of: card)))
                case .group(let line, .open(let cards)):
                    shown.append(.groupLine(line: line, fold: .open))
                    for (place, card) in cards.enumerated() {
                        let guide = GuideStretch(dotted: line.scratch != nil, first: place == 0)
                        shown.append(.nested(model(of: card), guide: guide))
                    }
                case .group(
                    let line,
                    .folded(let sessions, let subagents, let waiting, let units, let moreUnits)
                ):
                    let fold = Fold.folded(
                        sessions: sessions, subagents: subagents, waiting: waiting, units: units,
                        moreUnits: moreUnits
                    )
                    shown.append(.groupLine(line: line, fold: fold))
                case .elsewhere(let heading, let lines, let waiting):
                    shown.append(.elsewhere(heading: heading, lines: lines, waiting: waiting))
                case .quietProjects(_, let open, let label, let tooltip):
                    shown.append(.quietProjects(open: open, label: label, tooltip: tooltip))
                }
            }
            byPath = kept
            return .rows(shown)
        }
    }
}
