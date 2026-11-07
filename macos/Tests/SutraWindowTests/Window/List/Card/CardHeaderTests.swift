import AppKit
@testable import SutraWindow
import SwiftUI
import Testing

/// A card's name and its chip, as its header sets them: side by side where both fit at their ideal
/// widths, the chip under the name where they do not.
@MainActor
struct CardHeaderTests {
    private var name: some View {
        Text("feat-digest-trees-stored-fix3-batched-writes")
            .font(.title3.weight(.semibold))
            .lineLimit(1)
            .truncationMode(.middle)
    }

    private var chip: some View {
        Text("detached 28b7bc82")
            .font(.callout.monospaced())
            .lineLimit(1)
            .truncationMode(.middle)
    }

    @Test func theChipStandsBesideTheNameWhereBothFitAtTheirIdealWidths() {
        let nameSize = size(of: name, within: nil)
        let chipSize = size(of: chip, within: nil)
        let ideal = size(of: NameAndChip { name; chip }, within: nil)
        #expect(abs(ideal.width - (nameSize.width + 8 + chipSize.width)) < 0.001)
        #expect(ideal.height < nameSize.height + chipSize.height, "one line")
        #expect(size(of: NameAndChip { name; chip }, within: ideal.width) == ideal)
    }

    @Test func theChipGoesUnderTheNameWhereTheyDoNotBothFit() {
        let nameSize = size(of: name, within: nil)
        let chipSize = size(of: chip, within: nil)
        let ideal = size(of: NameAndChip { name; chip }, within: nil)
        let under = size(of: NameAndChip { name; chip }, within: ideal.width - 0.5)
        #expect(under.width == nameSize.width)
        #expect(under.height == nameSize.height + 4 + chipSize.height)
    }

    @Test func underTheNameEachIsCutToTheWidthThereIs() {
        let nameSize = size(of: name, within: nil)
        let chipSize = size(of: chip, within: nil)
        let cut = size(of: NameAndChip { name; chip }, within: 100)
        #expect(cut.width <= 100)
        #expect(cut.height == nameSize.height + 4 + chipSize.height, "each still one line")
    }

    /// The size `view` takes when offered `width`, as wide as it likes when `nil`.
    private func size(of view: some View, within width: CGFloat?) -> CGSize {
        NSHostingController(rootView: view)
            .sizeThatFits(in: CGSize(width: width ?? .infinity, height: .infinity))
    }
}
