//! Moments written in RFC 3339's form in UTC, as `2026-11-05T19:00:15.325Z` reads: how Claude Code
//! dates each entry of a transcript, and Docker's engine the moment a container started.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The moment `text` names, written `YYYY-MM-DDTHH:MM:SS`, then a dot and up to nine digits of a
/// second when it falls between seconds, then `Z`: JavaScript's `toISOString` writes three digits,
/// and Go, which Docker is written in, as many as the moment needs. `None` for any other form, and
/// for a moment before 1970.
pub(crate) fn parse(text: &str) -> Option<SystemTime> {
    let text = text.strip_suffix('Z')?;
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (text, None),
    };
    let bytes = whole.as_bytes();
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if bytes.len() != 19 || separators.iter().any(|&(at, byte)| bytes[at] != byte) {
        return None;
    }
    let number = |digits: &[u8]| {
        digits.iter().all(u8::is_ascii_digit).then(|| {
            digits
                .iter()
                .fold(0, |number, digit| number * 10 + u64::from(digit - b'0'))
        })
    };
    let (year, month, day) = (
        number(&bytes[0..4])?,
        number(&bytes[5..7])?,
        number(&bytes[8..10])?,
    );
    let (hour, minute, second) = (
        number(&bytes[11..13])?,
        number(&bytes[14..16])?,
        number(&bytes[17..19])?,
    );
    let valid = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && second < 60;
    if !valid {
        return None;
    }
    let nanos = match fraction.map(str::as_bytes) {
        None => 0,
        Some(digits @ [_, ..]) if digits.len() <= 9 => {
            // At most nine digits, so the shift is at most a billion and the product fits.
            number(digits)? * 10_u64.pow(9 - digits.len() as u32)
        }
        Some(_) => return None,
    };
    let days = days_since_epoch(year, month, day)?;
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds) + Duration::from_nanos(nanos))
}

/// The days from 1970-01-01 to a date of the Gregorian calendar, or `None` for a date before it.
/// This is Howard Hinnant's `days_from_civil`, which counts each year from March so that a leap
/// day ends it.
fn days_since_epoch(year: u64, month: u64, day: u64) -> Option<u64> {
    let year = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let (era, year_of_era) = (year / 400, year % 400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    (era * 146_097 + day_of_era).checked_sub(719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moments_javascript_and_go_write_in_utc_parse_and_any_other_form_does_not() {
        let ns = |ns: u64| Some(UNIX_EPOCH + Duration::from_nanos(ns));
        let ms = |ms: u64| Some(UNIX_EPOCH + Duration::from_millis(ms));
        let cases = [
            ("2026-11-05T19:00:15.325Z", ms(1_793_905_215_325)),
            ("2024-02-29T23:59:59.999Z", ms(1_709_251_199_999)),
            ("2000-03-01T00:00:00.000Z", ms(951_868_800_000)),
            ("1970-01-01T00:00:00.000Z", ms(0)),
            // As Docker dates a container's start: trailing zeros dropped, or the fraction whole.
            (
                "2026-10-29T23:34:41.97809393Z",
                ns(1_793_316_881_978_093_930),
            ),
            (
                "2026-11-09T22:53:36.426100545Z",
                ns(1_794_264_816_426_100_545),
            ),
            ("2026-11-05T19:00:15Z", ms(1_793_905_215_000)),
            // What the engine says of a container that never started.
            ("0001-01-01T00:00:00Z", None),
            ("2026-11-05T19:00:15.Z", None),
            ("2026-11-05T19:00:15.1234567891Z", None),
            ("2026-11-05T19:00:15.325+01:00", None),
            ("2026-13-05T19:00:15.325Z", None),
            ("2026-11-05T24:00:15.325Z", None),
            ("2026-11-05T19:00:+5.325Z", None),
            ("1969-12-31T23:59:59.999Z", None),
            ("yesterday", None),
        ];
        for (text, moment) in cases {
            assert_eq!(parse(text), moment, "{text}");
        }
    }
}
