import Foundation
@testable import SutraRecording
import Testing

/// The recording writes as the core's `format` module does: these are that module's own cases.
struct WordsTests {
    @Test func aSizeHasADecimalOnlyUnderTenAndNoneThatSaysNothing() {
        #expect(Words.size(999) == "999 B")
        #expect(Words.size(5_000_000) == "5 MB")
        #expect(Words.size(3_100_000_000) == "3.1 GB")
        #expect(Words.size(12_345_678_901) == "12 GB")
        #expect(Words.size(999_950) == "1 MB", "999.95 KB moves to the next unit")
    }

    @Test func anAgeReadsTheLargestWholeUnit() {
        let now = Date(timeIntervalSince1970: 1_000_000)
        let ago = { (seconds: TimeInterval) in Words.age(of: now - seconds, at: now) }
        #expect(ago(0) == "now")
        #expect(ago(59) == "now")
        #expect(ago(60) == "1m")
        #expect(ago(3_599) == "59m")
        #expect(ago(3_600) == "1h")
        #expect(ago(86_399) == "23h")
        #expect(ago(5 * 86_400) == "5d")
        #expect(Words.age(of: now + 600, at: now) == "now", "a time ahead of the clock")
    }

    @Test func aFolderUnderTheInventedHomeIsWrittenFromTilde() {
        #expect(Words.tilde("/Demo/Projects/cedar-web") == "~/Projects/cedar-web")
        #expect(Words.tilde("/Demolition/site") == "/Demolition/site")
    }
}
