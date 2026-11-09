//! What's wrong with a settings file, in words, and where. Every error about one comes through here, so that the notice, the settings
//! window and the command line all say the same thing. A line number isn't always in the error: a file of changes is merged with the
//! defaults before it's read, which loses them, so the line is found again from what the error says it was reading.

use std::path::Path;

/// What's wrong with a settings file.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Problem {
    pub line: Option<usize>,
    /// That line, as it is in the file.
    pub source: Option<String>,
    pub what: String,
    /// What was probably meant.
    pub hint: Option<String>,
}

impl Problem {
    /// `error`, from reading the file at `path`.
    pub fn of(path: &Path, error: &str) -> Problem {
        let error = error.strip_prefix(&format!("{}: ", path.display())).unwrap_or(error);
        Problem::read(&std::fs::read_to_string(path).unwrap_or_default(), error)
    }

    /// `error`, from reading `text`.
    pub fn read(text: &str, error: &str) -> Problem {
        let error = error.trim();
        let mut p = if let Some(at) = error.strip_prefix("TOML parse error at line ") {
            let line = at.split([',', '\n']).next().and_then(|n| n.trim().parse().ok());
            // What's wrong: the lines after the drawing, which all have a `|` in them.
            let message: Vec<&str> = error.lines().skip(1).filter(|l| !l.trim_start().trim_start_matches(|c: char| c.is_ascii_digit()).trim_start().starts_with('|')).collect();
            let (what, hint) = plain(&message.join("\n"));
            Problem { line, what, hint, ..Problem::default() }
        } else if let Some((message, path)) = error.rsplit_once("\nin `").and_then(|(m, rest)| Some((m, rest.strip_suffix('`')?))) {
            let (what, hint) = plain(message);
            Problem { line: find_setting(text, path, message), what, hint, ..Problem::default() }
        } else if let Some(rest) = error.strip_prefix("meter `") {
            Problem { line: rest.split_once('`').and_then(|(label, after)| find_in_meter(text, label, after)), what: error.to_string(), ..Problem::default() }
        } else {
            let (what, hint) = plain(error);
            Problem { line: find_setting(text, "", error), what, hint, ..Problem::default() }
        };
        p.source = p.line.and_then(|n| text.lines().nth(n.checked_sub(1)?)).map(|l| l.trim().trim_start_matches('\u{feff}').to_string()).filter(|l| !l.is_empty());
        p
    }

    /// All of it, for a dialog.
    pub fn words(&self) -> String {
        let place = match (self.line, &self.source) {
            (Some(n), Some(source)) => format!("Line {n}: {source}\n"),
            (Some(n), None) => format!("Line {n}\n"),
            _ => String::new(),
        };
        let hint = self.hint.as_ref().map_or(String::new(), |h| format!("\n{h}"));
        format!("{place}{}{hint}", self.what)
    }
}

/// The words in backticks in `text`.
fn quoted(text: &str) -> Vec<&str> {
    text.split('`').skip(1).step_by(2).collect()
}

/// How many single changes turn `a` into `b`.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut last = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let was = row[j + 1];
            row[j + 1] = (was + 1).min(row[j] + 1).min(last + usize::from(ca != *cb));
            last = was;
        }
    }
    row[b.len()]
}

/// Which of `choices` `word` was probably meant to be.
fn nearest<'a>(word: &str, choices: &[&'a str]) -> Option<&'a str> {
    let word = word.to_ascii_lowercase();
    let close = word.chars().count() / 3 + 1;
    choices.iter().map(|c| (distance(&word, &c.to_ascii_lowercase()), *c)).filter(|(d, _)| *d <= close).min_by_key(|(d, _)| *d).map(|(_, c)| c)
}

/// What an error from reading the settings means, and what was probably meant.
fn plain(message: &str) -> (String, Option<String>) {
    let message = message.trim();
    let names = quoted(message);
    let hint = |word: &str, choices: &[&str]| nearest(word, choices).map(|n| format!("Did you mean `{n}`?"));
    if message.starts_with("unknown field `")
        && let [word, choices @ ..] = names.as_slice()
    {
        // What's only there so that old files can still be read isn't a thing to suggest.
        let choices: Vec<&str> = choices.iter().copied().filter(|c| *c != "thumbnails").collect();
        return (format!("`{word}` isn't a setting here."), hint(word, &choices));
    }
    if message.starts_with("unknown variant `")
        && let [word, choices @ ..] = names.as_slice()
    {
        return (format!("`{word}` isn't one of the choices: {}.", choices.join(", ")), hint(word, choices));
    }
    if message.starts_with("out of range: `")
        && let (Some(word), Some((_, range))) = (names.first(), message.split_once(", expected "))
    {
        return (format!("`{word}` is out of range: it goes from {}.", range.trim()), None);
    }
    if message.starts_with("bad color `")
        && let Some(word) = names.first()
    {
        return (format!("`{word}` isn't a color: use #RRGGBB, or #AARRGGBB for one you can see through."), None);
    }
    if let Some((found, expected)) = message.strip_prefix("invalid type: ").or_else(|| message.strip_prefix("invalid value: ")).and_then(|m| m.split_once(", expected ")) {
        let want = match expected.trim() {
            "a boolean" => "true or false",
            "a string" => "text in quotes",
            "a sequence" => "a list in square brackets",
            e if e.starts_with(['u', 'i', 'f']) && e[1..].chars().all(|c| c.is_ascii_digit()) => "a number",
            e => e,
        };
        let found = found.split_once(' ').map_or(found, |(_, value)| value).trim_matches('`');
        return (format!("This should be {want}, not {found}."), None);
    }
    (message.to_string(), None)
}

/// Whether `line` sets `key`, and the table a `[table]` line starts.
fn key_of(line: &str) -> Option<&str> {
    let (key, _) = line.split_once('=')?;
    Some(key.trim().trim_matches(['"', '\'']))
}

fn table_of(line: &str) -> Option<&str> {
    let line = line.trim();
    (line.starts_with('[') && !line.contains('=')).then(|| line.split('#').next().unwrap_or(line).trim().trim_matches(['[', ']']).trim())
}

/// The line of the setting at `path` (`twatch`, `meter.style`) that `message` is about: of those with that key, the one holding the
/// value it complains of. An unknown field's name is in the message, and not the path. Failing that, where its table starts.
fn find_setting(text: &str, path: &str, message: &str) -> Option<usize> {
    let parts: Vec<&str> = path.split('.').filter(|p| !p.is_empty() && !p.chars().all(|c| c.is_ascii_digit())).collect();
    let words = quoted(message);
    let unknown = message.starts_with("unknown field `").then(|| words.first().copied()).flatten();
    let (table, key) = match (unknown, parts.split_last()) {
        (Some(key), _) => (parts.join("."), Some(key)),
        (None, Some((key, table))) => (table.join("."), Some(*key)),
        (None, None) => (String::new(), None),
    };
    let value = message.split('"').nth(1).or(words.first().copied());
    let (mut here, mut start, mut found) = (String::new(), None, Vec::new());
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.starts_with('#') {
            continue;
        }
        if let Some(name) = table_of(line) {
            here = name.to_string();
            if here == table && start.is_none() {
                start = Some(i + 1);
            }
        } else if here == table && key_of(line).is_some_and(|k| Some(k) == key) {
            found.push((i + 1, line));
        }
    }
    let holding = found.iter().find(|(_, line)| value.is_some_and(|v| line.contains(v)));
    holding.or(found.first()).map(|(n, _)| *n).or(start).or_else(|| (!path.is_empty()).then(|| find_table_of(text, path)).flatten())
}

/// Where the table at the start of `path` is, for a path that's more than the file says.
fn find_table_of(text: &str, path: &str) -> Option<usize> {
    let first = path.split('.').next()?;
    text.lines().position(|l| table_of(l) == Some(first)).map(|i| i + 1)
}

/// The line of the meter labelled `label` that `after` (the rest of the error) complains of, or the line with its label.
fn find_in_meter(text: &str, label: &str, after: &str) -> Option<usize> {
    let lines: Vec<&str> = text.lines().collect();
    let names_it = |l: &&str| key_of(l.trim()) == Some("label") && l.contains(&format!("\"{label}\""));
    let at = lines.iter().position(names_it)?;
    let begins = lines[..at].iter().rposition(|l| table_of(l).is_some()).map_or(0, |i| i + 1);
    let ends = lines[at..].iter().position(|l| table_of(l).is_some()).map_or(lines.len(), |i| at + i);
    let bad_key = after.trim_start_matches(':').trim().split(' ').next().unwrap_or_default();
    let bad_word = quoted(after).into_iter().next();
    let is_it = |l: &&str| !l.trim_start().starts_with('#') && (key_of(l.trim()) == Some(bad_key) || bad_word.is_some_and(|w| l.split_once('=').is_some_and(|(_, value)| value.contains(w))));
    Some(lines[begins..ends].iter().position(is_it).map_or(at + 1, |i| begins + i + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_syntax_error_keeps_its_line_and_loses_the_drawing() {
        let text = "config_version = 2\ninterval_ms = 500\n[twatch\n";
        let p = Problem::read(text, "TOML parse error at line 3, column 8\n  |\n3 | [twatch\n  |        ^\nunclosed table, expected `]`\n");
        assert_eq!((p.line, p.source.as_deref(), p.what.as_str()), (Some(3), Some("[twatch"), "unclosed table, expected `]`"));
        assert_eq!(p.words(), "Line 3: [twatch\nunclosed table, expected `]`");
    }

    #[test]
    fn a_misspelled_setting_is_found_in_the_file_and_corrected() {
        let text = "config_version = 2\n\n[twatch]\ncheck_seconds = 60\nfavourites = [\"crab\"]\n";
        let p = Problem::read(text, "unknown field `favourites`, expected one of `favorites`, `more_sets`, `thumbnails`\nin `twatch`\n");
        assert_eq!(p.line, Some(5));
        assert_eq!(p.source.as_deref(), Some("favourites = [\"crab\"]"));
        assert_eq!(p.what, "`favourites` isn't a setting here.");
        assert_eq!(p.hint.as_deref(), Some("Did you mean `favorites`?"));
    }

    #[test]
    fn a_setting_kept_for_old_files_is_never_suggested() {
        let p = Problem::read("thumbnail = 1\n", "unknown field `thumbnail`, expected one of `favorites`, `thumbnails`");
        assert_eq!(p.hint, None);
    }

    #[test]
    fn a_wrong_kind_of_value_is_said_plainly() {
        let text = "config_version = 2\n\n# mine\ninterval_ms = \"fast\"\n";
        let p = Problem::read(text, "invalid type: string \"fast\", expected u64\nin `interval_ms`\n");
        assert_eq!(p.line, Some(4));
        assert_eq!(p.what, "This should be a number, not \"fast\".");
    }

    #[test]
    fn a_value_out_of_range_or_not_a_color_is_said_plainly() {
        let text = "config_version = 2\n\nshuffle_seconds = 1\n\n[hotmic]\ncolor = \"red\"\n";
        let p = Problem::read(text, "out of range: `1`, expected 5 to 3600\nin `shuffle_seconds`");
        assert_eq!((p.line, p.what.as_str()), (Some(3), "`1` is out of range: it goes from 5 to 3600."));
        let p = Problem::read(text, "bad color `red`, expected #RRGGBB or #AARRGGBB\nin `hotmic.color`");
        assert_eq!(p.line, Some(6));
        assert!(p.what.starts_with("`red` isn't a color"), "{}", p.what);
        let p = Problem::read("shuffle_seconds = -3\n", "invalid value: integer `-3`, expected u32\nin `shuffle_seconds`");
        assert_eq!(p.what, "This should be a number, not -3.");
    }

    #[test]
    fn of_several_meters_it_is_the_one_with_the_bad_value() {
        let text = "[[meter]]\nlabel = \"RAM\"\nstyle = \"pie\"\n\n[[meter]]\nlabel = \"GPU\"\nstyle = \"wibble\"\n";
        let p = Problem::read(text, "unknown variant `wibble`, expected one of `history`, `bar`, `pie`\nin `meter.style`\n");
        assert_eq!(p.line, Some(7));
        assert_eq!(p.what, "`wibble` isn't one of the choices: history, bar, pie.");
    }

    #[test]
    fn a_choice_that_is_nearly_right_is_corrected() {
        let p = Problem::read("[[meter]]\nstyle = \"histroy\"\n", "unknown variant `histroy`, expected one of `history`, `bar`\nin `meter.style`\n");
        assert_eq!(p.hint.as_deref(), Some("Did you mean `history`?"));
    }

    #[test]
    fn a_meter_error_is_placed_at_what_it_complains_of() {
        let text = "[[meter]]\nlabel = \"CPU\"\nexpr = \"1\"\n\n[[meter]]\nlabel = \"RAM\"\nexpr = \"nope + 1\"\nmax = 1\n";
        assert_eq!(Problem::read(text, "meter `RAM` uses `nope`, which is not a counter or builtin (a, b)").line, Some(7));
        assert_eq!(Problem::read(text, "meter `RAM`: time_axis = \"x\": use \"log\" or \"linear\"").line, Some(6), "it isn't in the file: the meter is");
        assert_eq!(Problem::read(text, "meter `TV`: anything").line, None, "a meter of the defaults, not the file");
    }

    #[test]
    fn something_else_is_passed_on_as_it_is() {
        let p = Problem::read("", "counter `x` (\\Nope\\Nothing): no such counter");
        assert_eq!(p.words(), "counter `x` (\\Nope\\Nothing): no such counter");
    }

    #[test]
    fn a_path_prefix_is_dropped() {
        let p = Problem::of(Path::new(r"C:\x\no-such.toml"), r"C:\x\no-such.toml: unclosed");
        assert_eq!(p.what, "unclosed");
    }
}
