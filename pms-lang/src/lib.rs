//! What PimpMyShell says, in every language it speaks.
//!
//! The texts live in `lang/`, a file per language, and are compiled in (build.rs): a section of the files is a module here,
//! a text in it a constant, and each language an array of them in the same order. So saying something is one look in an
//! array by a number that was known when this was built. Nothing is read from disk, parsed or allocated, when a
//! program starts or after, and the part of PimpMyShell that lives inside Explorer never opens a file for it.
//!
//! ```ignore
//! use pms_lang::welcome as w;
//! title.text(w::HELLO_TITLE.t());                              // in the language of now
//! note.text(w::DOWNLOADING.with(&[("done", &3), ("total", &9)])); // "{done} of {total} MB", filled in
//! ```
//!
//! Which language is one number for the whole process (`set`), so a change shows the next time anything's said: a
//! window that draws itself again says it all anew.

use std::fmt::{Display, Write as _};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A language PimpMyShell speaks.
#[derive(Debug, PartialEq)]
pub struct Language {
    /// Its file's name, and what the settings keep: `en_US`; `en_RN`, which is what Red Hat called its Redneck in 1998.
    pub code: &'static str,
    /// What it calls itself.
    pub name: &'static str,
    /// The Windows languages it's the one for, by how their names begin, with spaces between: "en" for English. None
    /// for a language that's only ever chosen.
    pub windows: &'static str,
}

/// What the settings say for "whatever Windows speaks".
pub const AUTO: &str = "auto";

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The language of now, as its place in `LANGUAGES`. English until something says otherwise.
#[inline]
pub fn current() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

/// From now on, everything's said in language `index`. One that isn't there changes nothing.
pub fn set(index: usize) {
    if index < LANGUAGES.len() {
        CURRENT.store(index, Ordering::Relaxed);
    }
}

/// The language of now.
pub fn language() -> &'static Language {
    &LANGUAGES[current().min(LANGUAGES.len() - 1)]
}

/// The language with this code, however it's spelled: `en_RN`, `en-rn`.
pub fn find(code: &str) -> Option<usize> {
    let same = |a: &str, b: &str| a.len() == b.len() && a.chars().zip(b.chars()).all(|(a, b)| a.eq_ignore_ascii_case(&b) || (matches!(a, '-' | '_') && matches!(b, '-' | '_')));
    LANGUAGES.iter().position(|l| same(l.code, code))
}

/// The language to speak for what the settings say (a code, or `AUTO`) on a PC whose Windows speaks `windows` (its
/// languages as Windows names them, the preferred one first: "de-DE"). A code that isn't one of ours is as good as
/// `AUTO`, and with nothing to go by it's English.
pub fn choose(setting: &str, windows: &[String]) -> usize {
    if let Some(chosen) = find(setting) {
        return chosen;
    }
    // "en" is the one for "en-GB", and not for a language whose name only starts with those letters.
    let begins = |theirs: &str, ours: &str| theirs.get(..ours.len()).is_some_and(|start| start.eq_ignore_ascii_case(ours)) && matches!(theirs[ours.len()..].chars().next(), None | Some('-' | '_'));
    let speaks = |l: &Language, theirs: &str| l.windows.split_whitespace().any(|ours| begins(theirs, ours));
    windows.iter().find_map(|theirs| LANGUAGES.iter().position(|l| speaks(l, theirs))).unwrap_or(0)
}

/// `template` with each `{name}` replaced by what `with` has under that name. `{{` and `}}` are braces as they stand,
/// and a name `with` hasn't is left as it is, to be seen.
pub fn fill(template: &str, with: &[(&str, &dyn Display)]) -> String {
    let mut out = String::with_capacity(template.len() + 16 * with.len());
    let mut rest = template;
    while let Some(at) = rest.find(['{', '}']) {
        out.push_str(&rest[..at]);
        let (brace, after) = (&rest[at..at + 1], &rest[at + 1..]);
        if after.starts_with(brace) {
            out.push_str(brace);
            rest = &after[1..];
            continue;
        }
        let named = (brace == "{").then(|| after.split_once('}')).flatten();
        match named.and_then(|(name, after)| with.iter().find(|(n, _)| *n == name).map(|(_, value)| (value, after))) {
            Some((value, after)) => {
                let _ = write!(out, "{value}");
                rest = after;
            }
            None => {
                out.push_str(brace);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A section of the language files, as a module: a constant for each of its texts, and the texts themselves, an array
/// for each language.
macro_rules! section {
    ($name:ident: $count:literal { $($key:ident = $index:literal,)* } $texts:expr) => {
        pub mod $name {
            /// One of this section's texts: which, not what it says.
            #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
            pub struct Id(u16);
            $(pub const $key: Id = Id($index);)*
            static TEXTS: [[&str; $count]; super::LANGUAGES.len()] = $texts;
            static KEYS: [&str; $count] = [$(stringify!($key)),*];

            impl Id {
                /// What it says in the language of now.
                #[inline]
                pub fn t(self) -> &'static str {
                    TEXTS[super::current()][self.0 as usize]
                }

                /// What it says in the language of now, with each `{name}` in it filled in.
                pub fn with(self, with: &[(&str, &dyn std::fmt::Display)]) -> String {
                    super::fill(self.t(), with)
                }

                /// What it says in English, with each `{name}` in it filled in, whatever the language of now: for a log, which
                /// is always in English, of something a person is told in theirs.
                pub fn english(self, with: &[(&str, &dyn std::fmt::Display)]) -> String {
                    super::fill(self.of(0), with)
                }

                /// What it says in one language, whatever the language of now.
                pub fn of(self, language: usize) -> &'static str {
                    TEXTS[language.min(super::LANGUAGES.len() - 1)][self.0 as usize]
                }

                /// What it's called in the language files.
                pub fn key(self) -> &'static str {
                    KEYS[self.0 as usize]
                }
            }

            /// Every text of the section.
            pub fn all() -> impl Iterator<Item = Id> {
                (0..$count as u16).map(Id)
            }
        }
    };
}

include!(concat!(env!("OUT_DIR"), "/texts.rs"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_first_and_what_everything_starts_in() {
        assert_eq!(LANGUAGES[0].code, "en_US");
        assert!(MISSING[0].is_empty() && STALE[0].is_empty(), "the source has every text, and is never behind itself");
        assert_eq!(FINGERPRINTS.len(), TEXTS);
        assert!(LANGUAGES.iter().all(|l| !l.name.is_empty()));
    }

    #[test]
    fn a_language_is_found_however_its_code_is_spelled() {
        assert_eq!(find("en_US"), Some(0));
        assert_eq!(find("EN-us"), Some(0));
        assert_eq!(find("en"), None);
        assert_eq!(find(AUTO), None);
        assert_eq!(find(""), None);
    }

    #[test]
    fn the_settings_choose_and_windows_does_when_they_dont() {
        let windows = |names: &[&str]| names.iter().map(|n| n.to_string()).collect::<Vec<_>>();
        assert_eq!(choose(AUTO, &windows(&["en-GB"])), 0);
        assert_eq!(choose(AUTO, &windows(&["enx-ZZ", "EN"])), 0);
        assert_eq!(choose(AUTO, &windows(&["tlh-Piqd"])), 0, "nothing we speak: English");
        assert_eq!(choose(AUTO, &[]), 0);
        assert_eq!(choose("no_SUCH", &windows(&["en-US"])), 0);
        // A language that's only ever chosen is chosen, and is nobody's Windows language.
        if let Some(chosen) = LANGUAGES.iter().position(|l| l.windows.is_empty()) {
            assert_eq!(choose(LANGUAGES[chosen].code, &windows(&["en-US"])), chosen);
            assert_ne!(choose(AUTO, &windows(&["en-US", LANGUAGES[chosen].code])), chosen);
        }
    }

    #[test]
    fn names_are_filled_in_and_braces_can_be_braces() {
        assert_eq!(fill("{done} of {total} MB", &[("done", &3), ("total", &"9")]), "3 of 9 MB");
        assert_eq!(fill("{total}: {done}, {done}", &[("done", &3), ("total", &9)]), "9: 3, 3", "in any order, as often as wanted");
        assert_eq!(fill("a {{brace}} and {this}", &[("this", &"that")]), "a {brace} and that");
        assert_eq!(fill("{nobody} home", &[("this", &"that")]), "{nobody} home", "left to be seen");
        assert_eq!(fill("plain", &[]), "plain");
        assert_eq!(fill("odd } one {", &[]), "odd } one {");
        assert_eq!(fill("caf\u{e9} {n}\u{20ac}", &[("n", &2)]), "caf\u{e9} 2\u{20ac}");
    }

    /// A text that's been through a program reading UTF-8 as Windows-1252 and writing it back (Windows PowerShell's Get-Content
    /// and Set-Content do, by default) has `•` as `â€¢` and `…` as `â€¦`. Its characters, taken back to the bytes Windows-1252
    /// has for them, are UTF-8 again; real words (é, ñ, ü, …) almost never are.
    #[test]
    fn no_language_file_has_been_read_as_windows_1252() {
        // Windows-1252's 0x80..0x9F; the five it hasn't are kept as their own code point, as such a program keeps them.
        const HIGH: [char; 32] = [
            '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
        ];
        let byte = |c: char| match HIGH.iter().position(|&h| h == c) {
            Some(at) => Some(0x80 + at as u8),
            None => u8::try_from(u32::from(c)).ok(),
        };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../lang");
        for language in &LANGUAGES {
            let path = dir.join(format!("{}.toml", language.code));
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            for (n, line) in text.lines().enumerate() {
                let mut run = Vec::new();
                for c in line.chars().chain(std::iter::once('\n')) {
                    if !c.is_ascii() && let Some(b) = byte(c) {
                        run.push(b);
                        continue;
                    }
                    let garbled = run.len() > 1 && std::str::from_utf8(&run).is_ok();
                    assert!(!garbled, "lang/{}.toml line {}: {:?} is UTF-8 that was read as Windows-1252 ({line})", language.code, n + 1, String::from_utf8_lossy(&run));
                    run.clear();
                }
            }
        }
        // And it does see one.
        assert!(std::str::from_utf8(&"â€¢".chars().filter_map(byte).collect::<Vec<_>>()).is_ok_and(|s| s == "•"));
        assert!(std::str::from_utf8(&"café".chars().filter_map(byte).filter(|b| !b.is_ascii()).collect::<Vec<_>>()).is_err());
    }

    #[test]
    fn every_language_says_every_text() {
        for (index, language) in LANGUAGES.iter().enumerate() {
            for id in welcome::all() {
                assert!(!id.of(index).is_empty(), "{}: welcome.{}", language.code, id.key().to_lowercase());
            }
        }
        // And what's said changes with the language, for everything said after.
        let was = current();
        set(LANGUAGES.len() - 1);
        assert_eq!(welcome::all().next().map(|id| id.t()), welcome::all().next().map(|id| id.of(LANGUAGES.len() - 1)));
        set(LANGUAGES.len());
        assert_eq!(current(), LANGUAGES.len() - 1, "a language that isn't there changes nothing");
        set(was);
    }
}
