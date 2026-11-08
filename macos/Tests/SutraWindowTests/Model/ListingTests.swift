import SutraCore
@testable import SutraWindow
import Testing

/// The core's list as the window keeps it: a row for each of the core's, each card with its model
/// from one set of contents to the next, wherever it hangs.
@MainActor
struct ListingTests {
    @Test func eachOfTheCoresRowsIsARowAtItsPlace() {
        let rows = shown(.rows(rows: [
            .line(line: group, place: Place(depth: 0, first: false)),
            .line(line: checkout, place: Place(depth: 1, first: true)),
            .card(card: card("/code/app"), place: Place(depth: 2, first: true)),
            .card(card: card("/code/app-fix"), place: Place(depth: 2, first: false)),
            .card(card: card("/code/app-2"), place: Place(depth: 1, first: false)),
        ]))
        #expect(rows == [
            "line app at 0",
            "line app at 1, first",
            "card /code/app at 2, first",
            "card /code/app-fix at 2",
            "card /code/app-2 at 1",
        ])
    }

    @Test func dockersCardIsARowOfItsOwnKnownAsTheOneDockerCard() {
        let docker = DockerCard(
            name: "Docker", nameTooltip: "", age: nil, lozenges: [], tray: nil,
            location: "~/.docker/run/docker.sock"
        )
        let place = Place(depth: 1, first: true)
        var models = CardModels()
        let list = SutraCore.List.rows(rows: [.docker(card: docker, place: place)])
        guard case .rows(let rows) = models.listing(of: list) else {
            Issue.record("a list of rows")
            return
        }
        #expect(rows.map(\.id) == [.docker])
        #expect(rows == [.docker(docker, place: place)])
    }

    @Test func aCardKeepsItsModelAsItMovesFromOneLineToAnother() throws {
        var models = CardModels()
        let underTheGroup = models.listing(of: .rows(rows: [
            .line(line: group, place: Place(depth: 0, first: false)),
            .card(card: card("/code/app"), place: Place(depth: 1, first: true)),
        ]))
        let underItsCheckout = models.listing(of: .rows(rows: [
            .line(line: group, place: Place(depth: 0, first: false)),
            .line(line: checkout, place: Place(depth: 1, first: true)),
            .card(card: card("/code/app"), place: Place(depth: 2, first: true)),
        ]))
        let model = try #require(self.model(at: "/code/app", in: underTheGroup))
        #expect(self.model(at: "/code/app", in: underItsCheckout) === model)
    }

    private let group = Line(
        key: .group(kind: .upstream(remote: "github.com/me/app")), name: "app", tooltip: "",
        total: nil, cleansWhenIdle: nil, menu: [], fold: .open
    )

    private let checkout = Line(
        key: .checkout(folder: "/code/app"), name: "app", tooltip: "", total: nil,
        cleansWhenIdle: nil, menu: [], fold: .open
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
            case .line(let line, let place):
                "line \(line.name)" + at(place)
            case .card(let model, let place):
                "card \(model.path)" + at(place)
            case .docker(_, let place):
                "docker" + at(place)
            case .elsewhere:
                "elsewhere"
            case .quietProjects:
                "quiet"
            }
        }
    }

    private func at(_ place: Place) -> String {
        " at \(place.depth)" + (place.first ? ", first" : "")
    }

    /// The model of the card at `path` in `listing`.
    private func model(at path: String, in listing: Listing) -> CardModel? {
        guard case .rows(let rows) = listing else { return nil }
        return rows.lazy.compactMap { row -> CardModel? in
            switch row {
            case .card(let model, _): model.path == path ? model : nil
            case .line, .docker, .elsewhere, .quietProjects: nil
            }
        }.first
    }
}
