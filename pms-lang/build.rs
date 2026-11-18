//! Turns the language files (`lang/*.toml`, one per language) into tables that are compiled in: for each section of
//! `en_US.toml`, a constant per text and, per language, an array of the texts in the same order. Asking for a text is
//! then one look in an array, and there's nothing to read, parse or allocate when a program starts.
//!
//! `en_US.toml` is the source: it says which texts there are. A text another language hasn't (or has as "") is the English
//! one, put in here, so every table is whole. A text another language has that English hasn't is a mistake, and so is a
//! `{placeholder}` English hasn't: both stop the build, with the file and the line.
//!
//! The files are a small part of TOML, read by hand so that this needs no crate: `[section]`, `key = "text"` (with
//! `\"`, `\\`, `\n`, `\t` and `é`) or `key = 'text'` (as it stands), and `#` comments.
//!
//! A translation says which English it was made from: `key = "text"  # en:1a2b3c4d`, a fingerprint of the English text at
//! the time (`cargo xtask lang --stamp <code>` writes them). One whose English has changed since, or that has none, is
//! stale: it's still used, as the best there is, and `cargo xtask lang` lists it to be looked at again.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The language every other is a translation of.
const SOURCE: &str = "en_US";
/// The section that's about the language itself, and isn't texts.
const ABOUT: &str = "language";

/// A text: what it's called, what it says, the line it's on, and the fingerprint of the English it was made from.
struct Text {
    key: String,
    says: String,
    line: usize,
    from: Option<u32>,
}

/// A language's translations, by section and key: what each says, and the fingerprint of the English it was made from.
type Translated<'a> = BTreeMap<(&'a str, &'a str), (&'a str, Option<u32>)>;

/// A fingerprint of an English text (FNV-1a, 32 bits): enough to tell that it changed, which is all it's for.
fn fingerprint(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193))
}

/// A section of a file: its name, and its texts in the order they come.
struct Section {
    name: String,
    texts: Vec<Text>,
}

/// A language file, named for its language's code.
struct File {
    code: String,
    path: PathBuf,
    sections: Vec<Section>,
}

impl File {
    /// The sections that are texts.
    fn said(&self) -> impl Iterator<Item = &Section> {
        self.sections.iter().filter(|s| s.name != ABOUT)
    }

    /// What it says about itself under `key`.
    fn about(&self, key: &str) -> Option<&str> {
        self.sections.iter().find(|s| s.name == ABOUT)?.texts.iter().find(|t| t.key == key).map(|t| t.says.as_str())
    }
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("lang");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(&dir) {
        Ok(entries) => entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "toml")).collect(),
        Err(e) => fail(&dir, 0, &format!("can't be read: {e}")),
    };
    // English first, as language 0, which is what everything starts in; then the rest by their codes.
    paths.sort_by_key(|p| (stem(p) != SOURCE, stem(p)));
    let files: Vec<File> = paths.iter().map(|p| read(p)).collect();
    let Some(source) = files.first().filter(|f| f.code == SOURCE) else { fail(&dir.join(format!("{SOURCE}.toml")), 0, "isn't there, and it's the one that says which texts there are") };

    let mut out = String::new();
    writeln!(out, "pub static LANGUAGES: [Language; {}] = [", files.len()).unwrap();
    for f in &files {
        let Some(name) = f.about("name").filter(|n| !n.is_empty()) else { fail(&f.path, 0, "has no `name` under [language]: what the language calls itself") };
        writeln!(out, "    Language {{ code: {:?}, name: {name:?}, windows: {:?} }},", f.code, f.about("windows").unwrap_or_default()).unwrap();
    }
    writeln!(out, "];").unwrap();

    // What each other language has, by section and key; and that it has nothing English hasn't.
    let mut theirs: Vec<Translated> = Vec::new();
    for f in &files[1..] {
        let mut has = BTreeMap::new();
        for section in f.said() {
            let Some(english) = source.said().find(|s| s.name == section.name) else { fail(&f.path, section.texts.first().map_or(0, |t| t.line), &format!("[{}] isn't a section of {SOURCE}.toml", section.name)) };
            for text in &section.texts {
                let Some(original) = english.texts.iter().find(|t| t.key == text.key) else { fail(&f.path, text.line, &format!("`{}` isn't in [{}] of {SOURCE}.toml: a text that's gone, or was renamed", text.key, section.name)) };
                let known = placeholders(&source.path, original.line, &original.says);
                if let Some(extra) = placeholders(&f.path, text.line, &text.says).into_iter().find(|p| !known.contains(p)) {
                    fail(&f.path, text.line, &format!("`{}` has {{{extra}}}, which the English hasn't, so nothing would ever fill it", text.key));
                }
                has.insert((section.name.as_str(), text.key.as_str()), (text.says.as_str(), text.from));
            }
        }
        theirs.push(has);
    }

    let count: usize = source.said().map(|s| s.texts.len()).sum();
    writeln!(out, "/// How many texts there are, in every section together.\npub const TEXTS: usize = {count};").unwrap();
    let mut missing: Vec<Vec<String>> = vec![Vec::new(); files.len()];
    let mut stale: Vec<Vec<String>> = vec![Vec::new(); files.len()];
    let mut fingerprints = String::new();
    for section in source.said() {
        writeln!(out, "section! {{ {}: {} {{", section.name, section.texts.len()).unwrap();
        for (i, text) in section.texts.iter().enumerate() {
            placeholders(&source.path, text.line, &text.says);
            writeln!(out, "    {} = {i},", text.key.to_uppercase()).unwrap();
        }
        writeln!(out, "}} [").unwrap();
        for language in 0..files.len() {
            write!(out, "    [").unwrap();
            for text in &section.texts {
                // A text a language hasn't, or has as nothing, is the English one.
                let translated = if language == 0 { None } else { theirs[language - 1].get(&(section.name.as_str(), text.key.as_str())).copied().filter(|(t, _)| !t.is_empty()) };
                let own = if language == 0 { Some(text.says.as_str()) } else { translated.map(|(t, _)| t) };
                if let Some((_, from)) = translated
                    && from != Some(fingerprint(&text.says))
                {
                    stale[language].push(format!("{}.{}", section.name, text.key));
                }
                // English saying nothing somewhere is nothing to translate.
                if own.is_none() && !text.says.is_empty() {
                    missing[language].push(format!("{}.{}", section.name, text.key));
                }
                write!(out, "{:?}, ", own.unwrap_or(&text.says)).unwrap();
            }
            writeln!(out, "],").unwrap();
        }
        writeln!(out, "] }}").unwrap();
        for text in &section.texts {
            writeln!(fingerprints, "    ({:?}, {:?}, {:#010x}),", section.name, text.key, fingerprint(&text.says)).unwrap();
        }
    }
    // Translations made from English that has changed since, or that don't say which English they were made from.
    writeln!(out, "pub static STALE: [&[&str]; {}] = [", files.len()).unwrap();
    for keys in &stale {
        writeln!(out, "    &{keys:?},").unwrap();
    }
    writeln!(out, "];").unwrap();
    // Each English text's fingerprint, by section and key: what `cargo xtask lang --stamp` writes beside a translation.
    writeln!(out, "pub static FINGERPRINTS: &[(&str, &str, u32)] = &[\n{fingerprints}];").unwrap();
    // What each language hasn't a text of its own for, for whoever's translating (`cargo xtask lang`).
    writeln!(out, "pub static MISSING: [&[&str]; {}] = [", files.len()).unwrap();
    for keys in &missing {
        writeln!(out, "    &{keys:?},").unwrap();
    }
    writeln!(out, "];").unwrap();

    let to = Path::new(&std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("texts.rs");
    std::fs::write(&to, out).unwrap_or_else(|e| fail(&to, 0, &format!("can't be written: {e}")));
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Stops the build, saying where.
fn fail(path: &Path, line: usize, why: &str) -> ! {
    let at = if line > 0 { format!(":{line}") } else { String::new() };
    eprintln!("error: {}{at}: {why}", path.display());
    std::process::exit(1);
}

/// A name the texts' constants and sections can have: small letters, digits and underlines, starting with a letter.
fn is_name(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_lowercase()) && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn read(path: &Path) -> File {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| fail(path, 0, &format!("can't be read: {e}")));
    let code = stem(path);
    if !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || code.is_empty() {
        fail(path, 0, "is named for its language's code, which is letters, digits and underlines: en_US.toml");
    }
    let mut sections: Vec<Section> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let (n, line) = (i + 1, raw.trim());
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let Some((name, after)) = rest.split_once(']') else { fail(path, n, "a section's name ends with ]") };
            if !is_name(name) || !(after.trim().is_empty() || after.trim().starts_with('#')) {
                fail(path, n, "a section is [name]: small letters, digits and underlines");
            }
            if sections.iter().any(|s| s.name == name) {
                fail(path, n, &format!("[{name}] is here twice"));
            }
            sections.push(Section { name: name.to_string(), texts: Vec::new() });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { fail(path, n, "expected `key = \"text\"`") };
        let key = key.trim();
        if !is_name(key) {
            fail(path, n, &format!("`{key}` can't name a text: small letters, digits and underlines, starting with a letter"));
        }
        let Some(section) = sections.last_mut() else { fail(path, n, "a text belongs under a [section]") };
        if section.texts.iter().any(|t| t.key == key) {
            fail(path, n, &format!("`{key}` is here twice"));
        }
        let (says, comment) = string(path, n, value.trim());
        let from = comment.trim().strip_prefix("en:").and_then(|hex| u32::from_str_radix(hex.trim(), 16).ok());
        section.texts.push(Text { key: key.to_string(), says, line: n, from });
    }
    File { code, path: path.to_path_buf(), sections }
}

/// What a quoted string says: `"..."` with its escapes, or `'...'` as it stands; and the comment after it, if any. Nothing
/// else may follow it.
fn string(path: &Path, n: usize, value: &str) -> (String, String) {
    let mut chars = value.chars();
    let quote = chars.next();
    let mut out = String::new();
    let mut closed = false;
    match quote {
        Some('\'') => {
            for c in chars.by_ref() {
                if c == '\'' {
                    closed = true;
                    break;
                }
                out.push(c);
            }
        }
        Some('"') => {
            while let Some(c) = chars.next() {
                match c {
                    '"' => {
                        closed = true;
                        break;
                    }
                    '\\' => match chars.next() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some('"') => out.push('"'),
                        Some('\\') => out.push('\\'),
                        Some(u @ ('u' | 'U')) => {
                            let digits: String = chars.by_ref().take(if u == 'u' { 4 } else { 8 }).collect();
                            match u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32) {
                                Some(c) => out.push(c),
                                None => fail(path, n, &format!("\\{u}{digits} isn't a character")),
                            }
                        }
                        other => fail(path, n, &format!("\\{} isn't an escape this reads: \\\" \\\\ \\n \\t \\u00e9", other.map(String::from).unwrap_or_default())),
                    },
                    c => out.push(c),
                }
            }
        }
        _ => fail(path, n, "a text is in quotes: key = \"text\""),
    }
    let after = chars.as_str().trim();
    if !closed || !(after.is_empty() || after.starts_with('#')) {
        fail(path, n, "a text is one quoted string on one line, with at most a # comment after it");
    }
    (out, after.trim_start_matches('#').to_string())
}

/// The names in a text's `{placeholders}`. `{{` and `}}` are braces as they stand; any other brace must be one of a pair
/// around a name.
fn placeholders(path: &Path, line: usize, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(['{', '}']) {
        let (brace, after) = (&rest[at..at + 1], &rest[at + 1..]);
        if after.starts_with(brace) {
            rest = &after[1..];
            continue;
        }
        let name = (brace == "{").then(|| after.split_once('}')).flatten().map(|(name, _)| name);
        match name {
            Some(name) if is_name(name) => {
                found.push(name.to_string());
                rest = &after[name.len() + 1..];
            }
            _ => fail(path, line, &format!("a brace in {text:?} that isn't one of a {{name}}: write {{{{ or }}}} for a brace that's just a brace")),
        }
    }
    found
}
