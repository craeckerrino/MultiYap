//! Versionsnummern vergleichen, für die Update-Prüfung im Programm (crates/web/src/updates.rs).

/// „X.Y.Z“ als Zahlen; nur Ziffern und zwei Punkte.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let mut next = || -> Option<u64> {
        let part = parts.next()?;
        (!part.is_empty() && part.len() <= 9 && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse().ok())?
    };
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Ist `candidate` neuer als `current`?
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!((parse_version(candidate), parse_version(current)), (Some(a), Some(b)) if a > b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert_eq!(parse_version("0.97.0"), Some((0, 97, 0)));
        assert_eq!(parse_version("0.97"), None);
        assert_eq!(parse_version("../1.2.3"), None);
        assert!(is_newer("0.98.0", "0.97.9"));
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(!is_newer("0.97.0", "0.97.0"));
        assert!(!is_newer("x", "0.97.0"));
    }
}
