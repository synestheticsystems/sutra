import AppKit
@testable import SutraWindow
import SwiftUI
import Testing

/// A line's header as `TitleAndLozenges` lays it out: the title, as many lozenges as there is room
/// for and the count of the rest, the lozenges giving way to the count, the last first, before the
/// title narrows. Each piece has a width of its own, so the width of the line tells which stand.
@MainActor
struct TitleAndLozengesTests {
    /// A title 200 wide that can narrow; lozenges 80 and 70 wide, taller than the rest; and the
    /// counts after none, one and both of them, 21, 22 and 23 wide, the last only with `more`.
    private func header(more: Bool = true) -> some View {
        TitleAndLozenges {
            Color.clear
                .frame(minWidth: 0, idealWidth: 200, maxWidth: 200)
                .frame(height: 16)
                .linePiece(.title)
            piece(80, high: 20).linePiece(.lozenge(0))
            piece(21).linePiece(.count(after: 0))
            piece(70, high: 20).linePiece(.lozenge(1))
            piece(22).linePiece(.count(after: 1))
            if more {
                piece(23).linePiece(.count(after: 2))
            }
        }
    }

    @Test func everyLozengeAndTheCountOfTheRestStandAfterTheTitleWhereThereIsRoom() {
        let every: CGFloat = 200 + 8 + 80 + 8 + 70 + 8 + 23
        #expect(size(of: header(), within: nil) == CGSize(width: every, height: 20))
        #expect(size(of: header(), within: every).width == every)
        #expect(size(of: header(), within: every + 100).width == every, "and no wider")
    }

    @Test func shortOfRoomTheLastLozengeGivesWayToTheCountFirst() {
        let every: CGFloat = 200 + 8 + 80 + 8 + 70 + 8 + 23
        let firstAndItsCount: CGFloat = 200 + 8 + 80 + 8 + 22
        #expect(size(of: header(), within: every - 0.5).width == firstAndItsCount)
    }

    @Test func onlyOnceTheCountStandsAloneDoesTheTitleNarrow() {
        let firstAndItsCount: CGFloat = 200 + 8 + 80 + 8 + 22
        let countAlone: CGFloat = 200 + 8 + 21
        #expect(size(of: header(), within: firstAndItsCount - 0.5).width == countAlone)
        #expect(size(of: header(), within: countAlone - 1).width == countAlone - 1)
        #expect(size(of: header(), within: 100).width == 100)
    }

    @Test func theLineStandsAsTallAsItsTallestPieceDrawnOrNot() {
        for width in [nil, 320, 229, 100] as [CGFloat?] {
            #expect(size(of: header(), within: width).height == 20, "within \(width as Any)")
        }
    }

    @Test func withNoCountPastThemTheLozengesStandAlone() {
        let both: CGFloat = 200 + 8 + 80 + 8 + 70
        let firstAndItsCount: CGFloat = 200 + 8 + 80 + 8 + 22
        #expect(size(of: header(more: false), within: nil).width == both)
        #expect(size(of: header(more: false), within: both - 0.5).width == firstAndItsCount)
    }

    /// A piece `wide` by `high`.
    private func piece(_ wide: CGFloat, high: CGFloat = 16) -> some View {
        Color.clear.frame(width: wide, height: high)
    }

    /// The size `view` takes when offered `width`, as wide as it likes when `nil`.
    private func size(of view: some View, within width: CGFloat?) -> CGSize {
        NSHostingController(rootView: view)
            .sizeThatFits(in: CGSize(width: width ?? .infinity, height: .infinity))
    }
}
