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

    @Test func eachContainerIsARowKnownByItsIdAndDockersDiskTheOneRowOfIts() {
        let container = { (id: String, name: String) in
            ContainerCard(
                id: id, name: name, nameTooltip: "", image: "postgres:17",
                imageTooltip: "postgres:17", age: "14d", lozenges: [], accountedFor: nil
            )
        }
        let pg = container("e99d", "pg596")
        let wgs = container("faf5", "wgs-pg-faf527a08d23d003")
        let tray = Tray(disk: nil, sessions: [], subagents: [])
        let location = "~/.docker/run/docker.sock"
        var models = CardModels()
        let list = SutraCore.List.rows(rows: [
            .container(card: pg, place: Place(depth: 1, first: true)),
            .container(card: wgs, place: Place(depth: 1, first: false)),
            .dockerDisk(tray: tray, location: location, place: Place(depth: 1, first: false)),
        ])
        guard case .rows(let rows) = models.listing(of: list) else {
            Issue.record("a list of rows")
            return
        }
        #expect(rows.map(\.id) == [.container(id: "e99d"), .container(id: "faf5"), .dockerDisk])
        #expect(rows.last == .dockerDisk(
            tray: tray, location: location, place: Place(depth: 1, first: false)
        ))
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
        total: nil, whenIdleMark: nil, menu: [], fold: .open
    )

    private let checkout = Line(
        key: .checkout(folder: "/code/app"), name: "app", tooltip: "", total: nil,
        whenIdleMark: nil, menu: [], fold: .open
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
            case .container(let card, let place):
                "container \(card.name)" + at(place)
            case .dockerDisk(_, _, let place):
                "docker's disk" + at(place)
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
            case .line, .container, .dockerDisk, .elsewhere, .quietProjects: nil
            }
        }.first
    }
}
