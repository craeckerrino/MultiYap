//! Zeitangaben von Twitch (RFC 3339, zum Beispiel „2026-11-15T12:00:00.123456789Z“) in Millisekunden seit 1970,
//! wie `Date.parse` im Browser. Nur das Format, das Twitch liefert; alles andere ergibt None.

use crate::i18n::tr;
/// Tage seit 1970-01-01 für ein Datum im gregorianischen Kalender (Algorithmus von Howard Hinnant).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn digits(text: &str) -> Option<u32> {
    (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok()).flatten()
}

/// Millisekunden seit 1970 (UTC) oder None, wenn es kein gültiger Zeitpunkt ist.
pub fn parse_rfc3339_ms(text: &str) -> Option<f64> {
    let text = text.trim();
    if text.len() < 20 || !text.is_ascii() {
        return None;
    }
    let (date, rest) = text.split_once(['T', 't'])?;
    let mut date_parts = date.split('-');
    let year = digits(date_parts.next()?)? as i64;
    let month = digits(date_parts.next()?)?;
    let day = digits(date_parts.next()?)?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    // Zeitzone am Ende: Z oder ±HH:MM.
    let (time, offset_minutes) = if let Some(time) = rest.strip_suffix(['Z', 'z']) {
        (time, 0i64)
    } else {
        let sign_at = rest.rfind(['+', '-'])?;
        let (time, zone) = rest.split_at(sign_at);
        let sign = if zone.starts_with('-') { -1 } else { 1 };
        let (hours, minutes) = zone[1..].split_once(':')?;
        (time, sign * (digits(hours)? as i64 * 60 + digits(minutes)? as i64))
    };
    let (clock, fraction) = time.split_once('.').map_or((time, ""), |(c, f)| (c, f));
    let mut clock_parts = clock.split(':');
    let hour = digits(clock_parts.next()?)?;
    let minute = digits(clock_parts.next()?)?;
    let second = digits(clock_parts.next()?)?;
    if clock_parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    // Nur Millisekunden zählen, weitere Stellen werden abgeschnitten (wie im Browser).
    let millis = if fraction.is_empty() {
        0
    } else {
        let padded: String = fraction.chars().chain(std::iter::repeat('0')).take(3).collect();
        if !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        padded.parse::<i64>().ok()?
    };
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour as i64 * 3_600 + minute as i64 * 60 + second as i64 - offset_minutes * 60;
    Some((seconds * 1000 + millis) as f64)
}

fn plural(count: i64, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// Alter eines Accounts (bisher format.ts der Nutzerkarte), zum Beispiel „3 Jahre, 2 Monate“ oder „12 Tage“.
/// `start` und `end` als (Jahr, Monat 0 bis 11, Tag) in Ortszeit, `elapsed_days` volle Tage dazwischen.
pub fn account_age(start: (i64, i64, i64), end: (i64, i64, i64), elapsed_days: i64) -> String {
    let mut years = end.0 - start.0;
    let mut months = end.1 - start.1;
    if end.2 < start.2 {
        months -= 1;
    }
    if months < 0 {
        years -= 1;
        months += 12;
    }
    if years > 0 {
        return if months > 0 { format!("{}, {}", plural(years, tr("Jahr", "year"), tr("Jahre", "years")), plural(months, tr("Monat", "month"), tr("Monate", "months"))) } else { plural(years, tr("Jahr", "year"), tr("Jahre", "years")) };
    }
    if months > 0 {
        return plural(months, tr("Monat", "month"), tr("Monate", "months"));
    }
    plural(elapsed_days.max(0), tr("Tag", "day"), tr("Tage", "days"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_alter() {
        let now = (2026, 10, 15);
        assert_eq!(account_age((2023, 8, 10), now, 0), "3 Jahre, 2 Monate");
        assert_eq!(account_age((2025, 10, 15), now, 0), "1 Jahr");
        assert_eq!(account_age((2026, 9, 1), now, 45), "1 Monat");
        assert_eq!(account_age((2026, 10, 3), now, 12), "12 Tage");
        assert_eq!(account_age((2026, 10, 14), now, 1), "1 Tag");
    }

    #[test]
    fn twitch_zeitstempel() {
        assert_eq!(parse_rfc3339_ms("1970-01-01T00:00:00Z"), Some(0.0));
        // Wie Date.parse('2026-11-15T12:00:00Z') im Browser.
        assert_eq!(parse_rfc3339_ms("2026-11-15T12:00:00Z"), Some(1_794_744_000_000.0));
        assert_eq!(parse_rfc3339_ms("2026-11-15T12:00:00.123456789Z"), Some(1_794_744_000_123.0));
        assert_eq!(parse_rfc3339_ms("2026-11-15T13:00:00+01:00"), Some(1_794_744_000_000.0));
        assert_eq!(parse_rfc3339_ms("2024-02-29T00:00:00Z"), Some(1_709_164_800_000.0));
    }

    #[test]
    fn ungueltig() {
        for text in ["", "kein Datum", "2026-13-01T00:00:00Z", "2026-11-15 12:00:00", "2026-11-15T25:00:00Z", "2026-11-15T12:00:00.ab Z"] {
            assert_eq!(parse_rfc3339_ms(text), None, "{text}");
        }
    }
}
