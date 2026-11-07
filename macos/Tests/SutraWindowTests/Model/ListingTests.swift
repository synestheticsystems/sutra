import SutraCore
@testable import SutraWindow
import Testing

/// The core's list as the window keeps it: an open group's cards in rows of their own under the
/// group's line, each card with its model from one set of contents to the next.
@MainActor
struct ListingTests {
    @Test func anOpenGroupListsItsLineThenEachOfItsCardsInARowOfItsOwn() {
        let rows = shown(.rows(rows: [
            .card(card: card("/code/notes")),
            .group(line: repository, fold: .open(cards: worktrees)),
            .group(line: session, fold: .open(cards: [card("/tmp/5f1c/scratchpad/render")])),
        ]))
        #expect(rows == [
            "card /code/notes",
            "line app, open",
            "nested /code/app, first, unbroken",
            "nested /code/app-fix, unbroken",
            "line Sutra launch plan, open",
            "nested /tmp/5f1c/scratchpad/render, first, dotted",
        ])
    }

    @Test func aFoldedGroupListsItsLineAlone() {
        let folded = GroupFold.folded(
            sessions: [], subagents: [], waiting: false, lozenges: [], moreLozenges: nil
        )
        let rows = shown(.rows(rows: [.group(line: repository, fold: folded)]))
        #expect(rows == ["line app, folded"])
    }

    @Test func dockersCardIsARowOfItsOwnKnownAsTheOneDockerCard() {
        let docker = DockerCard(
            name: "Docker", nameTooltip: "", age: nil, lozenges: [], tray: nil,
            location: "~/.docker/run/docker.sock"
        )
        var models = CardModels()
        guard case .rows(let rows) = models.listing(of: .rows(rows: [.docker(card: docker)])) else {
            Issue.record("a list of rows")
            return
        }
        #expect(rows.map(\.id) == [.docker])
        #expect(rows == [.docker(docker)])
    }

    @Test func aCardKeepsItsModelAsItMovesIntoAGroupAndOutAgain() throws {
        var models = CardModels()
        let alone = models.listing(of: .rows(rows: [.card(card: card("/code/app"))]))
        let nested = models.listing(of: .rows(rows: [
            .group(line: repository, fold: .open(cards: worktrees)),
        ]))
        let aloneAgain = models.listing(of: .rows(rows: [.card(card: card("/code/app"))]))
        let model = try #require(self.model(at: "/code/app", in: alone))
        #expect(self.model(at: "/code/app", in: nested) === model)
        #expect(self.model(at: "/code/app", in: aloneAgain) === model)
    }

    private let repository = GroupLine(
        parent: .repository(commonDir: "/code/app/.git"), name: "app", tooltip: "", scratch: nil,
        total: nil, menu: []
    )

    /// The repository's two worktrees.
    private var worktrees: [Card] {
        [card("/code/app"), card("/code/app-fix")]
    }

    private let session = GroupLine(
        parent: .session(folder: "/tmp/5f1c"), name: "Sutra launch plan", tooltip: "",
        scratch: ScratchMark(tooltip: "", sessionRuns: true), total: nil, menu: []
    )

    private func card(_ path: String) -> Card {
        Card(
            path: path, location: path, name: path, nameTooltip: "", chip: nil, age: nil,
            unreadable: nil, lozenges: [], tray: nil, waiting: false, menu: []
        )
    }

    /// The rows the window keeps `list` as, a few words each.
    private func shown(_ list: SutraCore.List) -> [String] {
        var models = CardModels()
        guard case .rows(let rows) = models.listing(of: list) else { return [] }
        return rows.map { row in
            switch row {
            case .card(let model):
                "card \(model.path)"
            case .groupLine(let line, .open):
                "line \(line.name), open"
            case .groupLine(let line, .folded):
                "line \(line.name), folded"
            case .nested(let model, let guide):
                "nested \(model.path)" + (guide.first ? ", first" : "")
                    + (guide.dotted ? ", dotted" : ", unbroken")
            case .docker:
                "docker"
            case .elsewhere:
                "elsewhere"
            case .quietProjects:
                "quiet"
            }
        }
    }

    /// The model of the card at `path` in `listing`, alone or nested.
    private func model(at path: String, in listing: Listing) -> CardModel? {
        guard case .rows(let rows) = listing else { return nil }
        return rows.lazy.compactMap { row -> CardModel? in
            switch row {
            case .card(let model), .nested(let model, _): model.path == path ? model : nil
            case .groupLine, .docker, .elsewhere, .quietProjects: nil
            }
        }.first
    }
}
