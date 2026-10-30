//! PimpMyShell's version, and comparing versions: plain `MAJOR.MINOR.PATCH`, a leading `v` (a tag) allowed. Anything
//! else - a pre-release suffix among it - is not a version the updater takes.

/// This build's version, Cargo.toml's.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// `text` as (major, minor, patch).
pub fn parse(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.');
    let mut next = || -> Option<u64> {
        let p = parts.next()?;
        (!p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse().ok())?
    };
    let v = (next()?, next()?, next()?);
    parts.next().is_none().then_some(v)
}

/// Whether `candidate` is a newer version than `current`; false if either is not a version.
pub fn newer(candidate: &str, current: &str) -> bool {
    matches!((parse(candidate), parse(current)), (Some(a), Some(b)) if a > b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse("v0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse("1.20.3"), Some((1, 20, 3)));
        assert_eq!(parse("0.1"), None);
        assert_eq!(parse("0.1.0-beta"), None);
        assert!(newer("v0.1.1", "0.1.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("garbage", "0.1.0"));
    }
}
