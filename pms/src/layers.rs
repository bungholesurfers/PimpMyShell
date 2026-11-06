//! A user's `pms.toml` holds only what they changed; everything else comes from the defaults that ship (`base`), so a
//! better default reaches everyone who didn't choose otherwise.
//!
//! The two are plain TOML tables, and `merge(base, over)` gives the settings in force:
//!
//! - a value in `over` replaces the one in `base`, table by table;
//! - `unset = ["key", ..]` in a table takes keys of `base` away (a section turned off, a default cleared);
//! - a `[[meter]]` with the label of a shipped meter changes that meter, field by field; any other is a meter of the
//!   user's own, added at the end; `removed_meters = [..]` takes shipped ones away, and `meter_order = [..]` says the
//!   order, should it be the user's.
//!
//! `diff(base, full)` is the opposite: the smallest `over` that gives `full`. `adopt` makes one from a file written
//! before this, which holds every setting: there, a value that's a past default isn't a choice the user made.

use toml::{Table, Value};

const UNSET: &str = "unset";
const METER: &str = "meter";
const REMOVED: &str = "removed_meters";
const ORDER: &str = "meter_order";
const LABEL: &str = "label";

fn label(meter: &Value) -> &str {
    meter.get(LABEL).and_then(Value::as_str).unwrap_or("")
}

fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn strings(v: Option<Value>) -> Vec<String> {
    v.and_then(|v| v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())).unwrap_or_default()
}

fn list(items: impl IntoIterator<Item = String>) -> Value {
    Value::Array(items.into_iter().map(Value::String).collect())
}

fn meters(t: &mut Table) -> Vec<Value> {
    match t.remove(METER) {
        Some(Value::Array(a)) => a,
        _ => Vec::new(),
    }
}

fn merge_table(base: &Table, over: &Table) -> Table {
    let mut out = base.clone();
    let mut over = over.clone();
    // Only a list of names is ours: a counter may be called `unset`.
    if matches!(over.get(UNSET), Some(Value::Array(_))) {
        for key in strings(over.remove(UNSET)) {
            out.remove(&key);
        }
    }
    for (key, value) in over {
        match (out.get_mut(&key), value) {
            (Some(Value::Table(b)), Value::Table(o)) => *b = merge_table(b, &o),
            (_, value) => {
                out.insert(key, value);
            }
        }
    }
    out
}

fn merge_meters(base: Vec<Value>, over: &[Value], removed: &[String], order: &[String]) -> Vec<Value> {
    let mut out: Vec<Value> = base.into_iter().filter(|m| !removed.iter().any(|r| same(r, label(m)))).collect();
    let mut taken = vec![false; out.len()];
    for meter in over {
        match (0..out.len()).find(|&i| !taken[i] && same(label(&out[i]), label(meter))) {
            Some(i) => {
                if let (Value::Table(b), Value::Table(o)) = (&out[i], meter) {
                    out[i] = Value::Table(merge_table(b, o));
                }
                taken[i] = true;
            }
            None => {
                out.push(meter.clone());
                taken.push(true);
            }
        }
    }
    let mut sorted = Vec::with_capacity(out.len());
    for name in order {
        if let Some(i) = out.iter().position(|m| same(label(m), name)) {
            sorted.push(out.remove(i));
        }
    }
    sorted.extend(out);
    sorted
}

/// The settings in force: `over` on top of `base`.
pub fn merge(base: &Table, over: &Table) -> Table {
    let (mut base, mut over) = (base.clone(), over.clone());
    let shipped = meters(&mut base);
    let own = meters(&mut over);
    let removed = strings(over.remove(REMOVED));
    let order = strings(over.remove(ORDER));
    let mut out = merge_table(&base, &over);
    let all = merge_meters(shipped, &own, &removed, &order);
    if !all.is_empty() {
        out.insert(METER.into(), Value::Array(all));
    }
    out
}

fn diff_table(base: &Table, full: &Table) -> Table {
    let mut over = Table::new();
    let mut unset = Vec::new();
    for (key, b) in base {
        match (b, full.get(key)) {
            (_, None) => unset.push(key.clone()),
            (Value::Table(b), Some(Value::Table(f))) => {
                let inner = diff_table(b, f);
                if !inner.is_empty() {
                    over.insert(key.clone(), Value::Table(inner));
                }
            }
            (b, Some(f)) if b != f => {
                over.insert(key.clone(), f.clone());
            }
            _ => {}
        }
    }
    for (key, f) in full {
        if !base.contains_key(key) {
            over.insert(key.clone(), f.clone());
        }
    }
    if !unset.is_empty() {
        over.insert(UNSET.into(), list(unset));
    }
    over
}

/// The smallest `over` for which `merge(base, over)` is `full`.
pub fn diff(base: &Table, full: &Table) -> Table {
    let (mut base, mut full) = (base.clone(), full.clone());
    let shipped = meters(&mut base);
    let all = meters(&mut full);
    let mut over = diff_table(&base, &full);

    let mut taken = vec![false; shipped.len()];
    let mut own = Vec::new();
    for meter in &all {
        match (0..shipped.len()).find(|&i| !taken[i] && same(label(&shipped[i]), label(meter))) {
            Some(i) => {
                taken[i] = true;
                if let (Value::Table(b), Value::Table(f)) = (&shipped[i], meter) {
                    let mut changes = diff_table(b, f);
                    if !changes.is_empty() {
                        changes.insert(LABEL.into(), Value::String(label(meter).to_string()));
                        own.push(Value::Table(changes));
                    }
                }
            }
            None => own.push(meter.clone()),
        }
    }
    let removed: Vec<String> = shipped.iter().zip(&taken).filter(|(_, t)| !**t).map(|(m, _)| label(m).to_string()).collect();
    let would_be = merge_meters(shipped.clone(), &own, &removed, &[]);
    let names = |l: &[Value]| l.iter().map(|m| label(m).to_string()).collect::<Vec<_>>();
    if names(&would_be) != names(&all) {
        over.insert(ORDER.into(), list(names(&all)));
    }
    if !removed.is_empty() {
        over.insert(REMOVED.into(), list(removed));
    }
    if !own.is_empty() {
        over.insert(METER.into(), Value::Array(own));
    }
    over
}

static NOTHING: std::sync::LazyLock<Table> = std::sync::LazyLock::new(Table::new);

fn adopt_table(full: &Table, base: &Table, past: &[&Table]) -> Table {
    let mut out = Table::new();
    let keys: std::collections::BTreeSet<&String> = full.keys().chain(base.keys()).collect();
    for key in keys {
        let (f, b) = (full.get(key), base.get(key));
        let kept = match (f, b) {
            (Some(Value::Table(f)), Some(Value::Table(b))) => {
                let inner: Vec<&Table> = past.iter().map(|p| p.get(key).and_then(Value::as_table).unwrap_or(&NOTHING)).collect();
                Some(Value::Table(adopt_table(f, b, &inner)))
            }
            // What an old default left as it is (even by not having it) wasn't anyone's choice: today's default it is.
            _ if past.iter().any(|p| p.get(key) == f) => b.cloned(),
            _ => f.cloned(),
        };
        if let Some(value) = kept {
            out.insert(key.clone(), value);
        }
    }
    out
}

/// What a file holding every setting (`full`) comes to on top of today's defaults: the settings in force after
/// adopting it, where whatever matches a past default (`past`, the older defaults) goes by `base` from now on.
pub fn adopt(base: &Table, full: &Table, past: &[Table]) -> Table {
    let (mut base, mut full) = (base.clone(), full.clone());
    let mut past: Vec<Table> = past.to_vec();
    let shipped = meters(&mut base);
    let all = meters(&mut full);
    let old: Vec<Vec<Value>> = past.iter_mut().map(meters).collect();
    let refs: Vec<&Table> = past.iter().collect();
    let mut out = adopt_table(&full, &base, &refs);

    let named = |l: &'_ [Value], name: &str| l.iter().position(|m| same(label(m), name));
    let mut kept: Vec<Value> = Vec::new();
    for meter in &shipped {
        let name = label(meter);
        match (named(&all, name), meter) {
            (Some(i), Value::Table(b)) => {
                let then: Vec<&Table> = old.iter().map(|o| named(o, name).and_then(|j| o[j].as_table()).unwrap_or(&NOTHING)).collect();
                if let Some(f) = all[i].as_table() {
                    kept.push(Value::Table(adopt_table(f, b, &then)));
                }
            }
            // Missing: taken away, unless the file may simply be older than this meter.
            (None, _) if old.iter().any(|o| named(o, name).is_none()) => kept.push(meter.clone()),
            _ => {}
        }
    }
    kept.extend(all.iter().filter(|m| named(&shipped, label(m)).is_none()).cloned());

    // The order too: theirs, unless it's one that shipped.
    let names = |l: &[Value]| l.iter().map(|m| label(m).to_lowercase()).collect::<Vec<_>>();
    let theirs = names(&all);
    let shipped_order = old.iter().any(|o| {
        let then = names(o);
        let common = |l: &[String], others: &[String]| l.iter().filter(|n| others.contains(n)).cloned().collect::<Vec<_>>();
        common(&theirs, &then) == common(&then, &theirs)
    });
    if !shipped_order {
        kept.sort_by_key(|m| theirs.iter().position(|n| *n == label(m).to_lowercase()).unwrap_or(usize::MAX));
    }
    if !kept.is_empty() {
        out.insert(METER.into(), Value::Array(kept));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(text: &str) -> Table {
        text.parse().unwrap()
    }

    const BASE: &str = r##"
interval_ms = 1000
theme = "auto"

[toasts]
duration_ms = 7000
style = "standard"

[backdrop]
tint = "#7F060606"

[counters]
ram = "a"
disk = "b"

[[meter]]
label = "CPU"
interval_ms = 250
caption = false

[[meter]]
label = "SWAP"
max = "swap_total"

[meter.show_when]
at = 0.85
until = 0.8

[[meter]]
label = "NET"
style = "nics"
"##;

    /// `full` comes back from its diff, and the diff holds what's expected.
    fn round_trip(full: &str, expected: &str) {
        let (base, full) = (t(BASE), t(full));
        let over = diff(&base, &full);
        assert_eq!(over, t(expected), "the diff");
        assert_eq!(merge(&base, &over), full, "the diff, merged");
    }

    #[test]
    fn nothing_changed_writes_nothing() {
        round_trip(BASE, "");
    }

    #[test]
    fn only_what_changed_is_written() {
        let full = BASE.replace("interval_ms = 1000", "interval_ms = 500").replace("duration_ms = 7000", "duration_ms = 9000");
        round_trip(&full, "interval_ms = 500\n[toasts]\nduration_ms = 9000");
    }

    #[test]
    fn a_changed_meter_keeps_only_its_change() {
        let full = BASE.replace("interval_ms = 250", "interval_ms = 100");
        round_trip(&full, "[[meter]]\nlabel = \"CPU\"\ninterval_ms = 100");
    }

    #[test]
    fn what_is_turned_off_stays_off() {
        let full = BASE.replace("[backdrop]\ntint = \"#7F060606\"\n", "").replace("[meter.show_when]\nat = 0.85\nuntil = 0.8\n", "");
        round_trip(&full, "unset = [\"backdrop\"]\n[[meter]]\nlabel = \"SWAP\"\nunset = [\"show_when\"]");
    }

    #[test]
    fn a_removed_counter_stays_removed() {
        round_trip(&BASE.replace("disk = \"b\"\n", ""), "[counters]\nunset = [\"disk\"]");
    }

    #[test]
    fn meters_can_go_come_and_move() {
        let base = t(BASE);
        let mut full = t(BASE);
        let Some(Value::Array(all)) = full.get_mut(METER) else { panic!() };
        all.remove(1);
        all.reverse();
        all.push(Value::Table(t("label = \"MINE\"\nexpr = \"1\"")));
        let over = diff(&base, &full);
        assert_eq!(merge(&base, &over), full);
        assert_eq!(over.get(REMOVED), Some(&list(["SWAP".to_string()])));
        assert_eq!(over.get(ORDER), Some(&list(["NET", "CPU", "MINE"].map(String::from))));
    }

    #[test]
    fn a_better_default_reaches_those_who_left_it_alone() {
        let old = t(BASE);
        let mut mine = old.clone();
        mine.insert("theme".into(), Value::String("dark".into()));
        let over = diff(&old, &mine);
        let better = t(&BASE.replace("interval_ms = 1000", "interval_ms = 750").replace("theme = \"auto\"", "theme = \"light\""));
        let now = merge(&better, &over);
        assert_eq!(now["interval_ms"].as_integer(), Some(750), "left alone: the new default");
        assert_eq!(now["theme"].as_str(), Some("dark"), "chosen: still theirs");
    }

    #[test]
    fn labels_match_in_any_case() {
        let over = t("[[meter]]\nlabel = \"cpu\"\ninterval_ms = 100");
        let merged = merge(&t(BASE), &over);
        assert_eq!(merged[METER].as_array().unwrap().len(), 3);
        assert_eq!(merged[METER][0]["interval_ms"].as_integer(), Some(100));
    }

    // What an older file holding every setting comes to.

    const OLD: &str = r##"
interval_ms = 1000
theme = "auto"

[toasts]
duration_ms = 7000
style = "standard"

[backdrop]
tint = "#7F060606"

[counters]
ram = "a"
disk = "b"

[[meter]]
label = "CPU"
interval_ms = 200
caption = true

[[meter]]
label = "SWAP"
max = "swap_total"

[[meter]]
label = "NET"
style = "bars"
"##;

    fn adopted(file: &str) -> Table {
        let (base, full) = (t(BASE), t(file));
        let over = diff(&base, &adopt(&base, &full, &[t(OLD)]));
        assert_eq!(merge(&base, &over), adopt(&base, &full, &[t(OLD)]));
        over
    }

    #[test]
    fn an_untouched_old_file_comes_to_nothing() {
        assert_eq!(adopted(OLD), Table::new());
        assert_eq!(adopted(BASE), Table::new());
    }

    #[test]
    fn past_defaults_go_and_choices_stay() {
        let file = OLD.replace("theme = \"auto\"", "theme = \"dark\"").replace("interval_ms = 200", "interval_ms = 300");
        assert_eq!(adopted(&file), t("theme = \"dark\"\n[[meter]]\nlabel = \"CPU\"\ninterval_ms = 300"));
    }

    #[test]
    fn what_an_old_default_lacked_takes_todays() {
        // No show_when in the file, and none in the old default: nobody turned it off.
        let over = adopted(OLD);
        assert!(!over.contains_key(METER));
    }

    #[test]
    fn a_setting_of_their_own_is_kept() {
        let over = adopted(&format!("update_check = true\n{OLD}"));
        assert_eq!(over.get("update_check"), Some(&Value::Boolean(true)));
    }

    #[test]
    fn a_meter_they_removed_stays_removed() {
        let file = OLD.replace("[[meter]]\nlabel = \"SWAP\"\nmax = \"swap_total\"\n\n", "");
        assert_eq!(adopted(&file).get(REMOVED), Some(&list(["SWAP".to_string()])));
    }

    #[test]
    fn a_meter_of_their_own_is_kept() {
        let over = adopted(&format!("{OLD}\n[[meter]]\nlabel = \"MINE\"\nexpr = \"1\""));
        assert_eq!(label(&over[METER][0]), "MINE");
    }
}
