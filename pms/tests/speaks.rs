//! What PimpMyShell tells you itself, made in every language it speaks. The language is one for the whole process, so this is a
//! test binary of its own, with one test: changing it here changes nothing for the unit tests, which read English, and nothing
//! here runs beside it.

use std::path::Path;

use pms::notices::{self, Notice};

/// Every notice there is, each way it can be made.
fn every_notice() -> Vec<Notice> {
    let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let file = Path::new(r"C:\nowhere\pms.toml");
    vec![
        notices::favorite_live(&names(&["crab"])),
        notices::favorite_live(&names(&["crab", "lobster"])),
        notices::favorite_live(&names(&["crab", "lobster", "shrimp"])),
        notices::console_spared(),
        notices::explorer_heavy(44, 360),
        notices::watchlist(Path::new(r"C:\nowhere\watchlist.txt"), &(1..=8).map(|n| (n, "not a channel!".to_string())).collect::<Vec<_>>()),
        notices::settings_issues(file, &names(&["unknown field `x`, expected `y`", "a", "b", "c", "d", "e", "f"])),
        notices::network_settings(Path::new(r"C:\nowhere\pms-network.toml"), "TOML parse error at line 1, column 1"),
        notices::notification_access(),
        notices::newer_streamlink("9.9.9", "8.6.3-1"),
        notices::no_streamlink(),
        notices::missing_part(pms::parts::TWATCH),
        notices::missing_part(pms::parts::POOM),
        notices::problem("The meters couldn't start", "error: no meter #3"),
        notices::taskbar_replaced(Some("StartAllBack")),
        notices::taskbar_replaced(None),
        notices::toasts_refused("no Grid", &["Windhawk", "TranslucentTB"]),
        notices::toasts_refused("no Grid", &[]),
        notices::toasts_failed("access denied"),
        notices::config(file, "TOML parse error at line 3", true),
        notices::config(file, "TOML parse error at line 3", false),
    ]
}

/// A `{name}` left in what's said: a placeholder nothing filled in.
fn unfilled(text: &str) -> Option<&str> {
    let at = text.find('{')?;
    let rest = &text[at + 1..];
    let end = rest.find('}')?;
    rest[..end].chars().all(|c| c.is_ascii_lowercase() || c == '_').then(|| &text[at..at + end + 2])
}

#[test]
fn every_notice_is_said_whole_in_every_language() {
    for (at, language) in pms::lang::LANGUAGES.iter().enumerate() {
        pms::lang::set(at);
        assert_eq!(pms::lang::current(), at);
        for n in every_notice() {
            let said = [&n.title, &n.body, &n.heading, &n.detail, &n.hint];
            assert!(!n.title.is_empty() && !n.body.is_empty() && !n.hint.is_empty(), "{}: {} has nothing to say", language.code, n.key);
            for text in said {
                assert_eq!(unfilled(text), None, "{}: {}: {text}", language.code, n.key);
                assert!(!text.contains("  "), "{}: {}: a gap in {text:?}", language.code, n.key);
            }
            assert!(n.fix.as_ref().is_none_or(|(label, _)| !label.is_empty()), "{}: {}'s fix has no words", language.code, n.key);
        }
        // And the rest of what's said: why Explorer wasn't restarted, what can't work without a part, the Network page.
        for why in [pms::lang::explorer::COPYING, pms::lang::explorer::FOLDER_OPEN, pms::lang::explorer::DIALOG_OPEN] {
            assert!(!pms::explorer::why_not(why).is_empty());
        }
        for part in [pms::parts::TWATCH, pms::parts::PIMPBOX, pms::parts::POOM, "nothing.exe"] {
            assert!(!pms::parts::without(part).is_empty());
        }
        assert_eq!(unfilled(&pms::update::addresses()), None, "{}", language.code);
        assert!(pms::net::USES.iter().all(|(_, title, part, about)| !title.t().is_empty() && !part.t().is_empty() && !about.t().is_empty()));
        assert!(pms::net::check("http://nope").is_err_and(|e| !e.is_empty()));
    }
    // And a change is heard at once, by whatever's said next.
    let last = pms::lang::LANGUAGES.len() - 1;
    let theirs = notices::console_spared().title;
    pms::lang::set(0);
    let ours = notices::console_spared().title;
    assert_eq!(ours, "The PIMPBOX is still running, though memory is low");
    if last > 0 {
        assert_ne!(theirs, ours, "{} says it its own way", pms::lang::LANGUAGES[last].code);
    }
}
