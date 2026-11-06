import Foundation

/// How the window writes paths, sizes and times, as the core's `format` module writes them, so
/// the invented work reads as the live core would write it.
enum Words {
    /// `path` with the invented home written `~`.
    static func tilde(_ path: String) -> String {
        guard path.hasPrefix(Scenario.home + "/") else { return path }
        return "~" + path.dropFirst(Scenario.home.count)
    }

    /// How long before `now` `then` was, in the largest whole unit; `nil` under a minute.
    static func wholeUnits(from then: Date, to now: Date) -> String? {
        let seconds = max(0, Int(now.timeIntervalSince(then)))
        switch seconds {
        case ..<60: return nil
        case ..<3_600: return "\(seconds / 60)m"
        case ..<86_400: return "\(seconds / 3_600)h"
        default: return "\(seconds / 86_400)d"
        }
    }

    /// How long before `now` `then` was: `now`, `3m`, `2h`, `5d`.
    static func age(of then: Date, at now: Date) -> String {
        wholeUnits(from: then, to: now) ?? "now"
    }

    /// A size as a card shows it, in the largest decimal unit that keeps the number under 1000, the
    /// way Finder counts, with a decimal only under 10 and none that says nothing: `999 B`,
    /// `3.1 GB`, `12 GB`.
    static func size(_ bytes: UInt64) -> String {
        guard bytes >= 1_000 else { return "\(bytes) B" }
        let units = ["KB", "MB", "GB", "TB", "PB", "EB"]
        var value = Double(bytes) / 1_000
        var unit = 0
        // 999.95 and up would print as 1000.0, so it moves to the next unit.
        while value >= 999.95, unit + 1 < units.count {
            value /= 1_000
            unit += 1
        }
        let tenths = String(format: "%.1f", value)
        let number = if value >= 10 {
            String(format: "%.0f", value)
        } else if tenths.hasSuffix(".0") {
            String(tenths.dropLast(2))
        } else {
            tenths
        }
        return "\(number) \(units[unit])"
    }

    /// `1 subagent`, `2 subagents`.
    static func plural(_ count: Int, _ noun: String) -> String {
        count == 1 ? "1 \(noun)" : "\(count) \(noun)s"
    }

    /// A moment as the user's clock reads it, to the minute: `2026-11-05 09:00`.
    static func timestamp(_ moment: Date) -> String {
        moment.formatted(.verbatim(
            """
            \(year: .defaultDigits)-\(month: .twoDigits)-\(day: .twoDigits) \
            \(hour: .twoDigits(clock: .twentyFourHour, hourCycle: .zeroBased)):\(minute: .twoDigits)
            """,
            timeZone: .current,
            calendar: Calendar(identifier: .gregorian)
        ))
    }
}
