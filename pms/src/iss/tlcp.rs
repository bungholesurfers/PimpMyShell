//! What Lightstreamer's TLCP says, as NASA's ISSLive! telemetry comes by it: the requests, as addresses (every one is a plain
//! GET, its parameters in the query string, which TLCP allows), and the lines that come back.
//!
//! An update is `U,<subscription>,<item>,<field>|<field>|...`: the item a position in the subscription's group, counting from 1,
//! and each field the next of its schema. In a later update an empty field is one that hasn't changed, `^3` three that haven't, `#`
//! is null and `$` empty; anything else is the value, percent-encoded where it has to be.
//!
//! The telemetry's time stamps are hours: the day of the year times 24, plus the hour, GMT, with the 1st of January as day 1.

use std::collections::HashMap;

/// Where the telemetry is.
pub const SERVER: &str = "https://push.lightstreamer.com/lightstreamer/";
const PROTOCOL: &str = "TLCP-2.5.0";
/// The client id TLCP's specification says every custom-made client sends (`LS_cid`): it names the protocol, not an account.
const CLIENT: &str = "mgQkwtwdysogQz2BJ4Ji%20kOj2Bg";

/// The urine tank, in percent.
pub const URINE: &str = "NODE3000005";
/// The urine processor's state: 2 stop, 4 shutdown, 8 maintenance, 16 normal (it's distilling), 32 standby, 64 idle, 128 starting.
pub const PROCESSOR: &str = "NODE3000004";
/// The waste water tank and the clean (drinking) water tank, in percent.
pub const WASTE: &str = "NODE3000008";
pub const DRINKING: &str = "NODE3000009";
/// The station's clock: its `Status.Class` is 24 while the ground has the station's signal.
pub const CLOCK: &str = "TIME_000001";
/// What a check asks for, in this order: the item numbers in its answer are places in it.
pub const ITEMS: [&str; 5] = [URINE, PROCESSOR, WASTE, DRINKING, CLOCK];
const FIELDS: [&str; 3] = ["Value", "TimeStamp", "Status.Class"];
/// Live, the tanks and the processor are one subscription, with no `Status.Class`, and the clock another, with only that: the
/// clock's `Value` is the time, which changes every second, and would answer every long poll at once.
pub const TANKS: [&str; 4] = [URINE, PROCESSOR, WASTE, DRINKING];
const TANK_FIELDS: [&str; 2] = ["Value", "TimeStamp"];
const SIGNAL_FIELDS: [&str; 1] = ["Status.Class"];
/// The class `Status.Class` has while there's a signal.
pub const SIGNAL: &str = "24";

fn spaced(words: &[&str]) -> String {
    words.join("%20")
}

/// One check: a session made, with every item's snapshot in its answer, and nothing waited for.
pub fn snapshot() -> String {
    format!(
        "{SERVER}create_session.txt?LS_protocol={PROTOCOL}&LS_adapter_set=ISSLIVE&LS_cid={CLIENT}&LS_polling=true&LS_polling_millis=0&LS_idle_millis=0&LS_op=add&LS_subId=1&LS_group={}&LS_schema={}&LS_mode=MERGE&LS_snapshot=true",
        spaced(&ITEMS),
        spaced(&FIELDS)
    )
}

/// How long the server keeps a live session between one poll and the next, in milliseconds.
const KEPT_FOR: u32 = 15_000;

/// Live: a session made with the tanks' subscription (1) and their snapshot, to poll from then on.
pub fn live() -> String {
    format!(
        "{SERVER}create_session.txt?LS_protocol={PROTOCOL}&LS_adapter_set=ISSLIVE&LS_cid={CLIENT}&LS_polling=true&LS_polling_millis={KEPT_FOR}&LS_idle_millis=0&LS_op=add&LS_subId=1&LS_group={}&LS_schema={}&LS_mode=MERGE&LS_snapshot=true",
        spaced(&TANKS),
        spaced(&TANK_FIELDS)
    )
}

/// Live: the clock's subscription (2), added to `session`.
pub fn add_signal(session: &str) -> String {
    format!("{SERVER}control.txt?LS_protocol={PROTOCOL}&LS_session={session}&LS_reqId=1&LS_op=add&LS_subId=2&LS_group={CLOCK}&LS_schema={}&LS_mode=MERGE&LS_snapshot=true", spaced(&SIGNAL_FIELDS))
}

/// Live: what's changed in `session`, held by the server up to `wait_ms` (it has a limit of its own, 30 s) until something has.
pub fn poll(session: &str, wait_ms: u32) -> String {
    format!("{SERVER}bind_session.txt?LS_protocol={PROTOCOL}&LS_session={session}&LS_polling=true&LS_polling_millis={KEPT_FOR}&LS_idle_millis={wait_ms}")
}

/// The end of `session`, rather than leave it for the server to time out.
pub fn destroy(session: &str) -> String {
    format!("{SERVER}control.txt?LS_protocol={PROTOCOL}&LS_session={session}&LS_reqId=9&LS_op=destroy")
}

/// The subscriptions a session has, by their numbers: each one's items and fields.
pub fn subscriptions(live: bool) -> Vec<(u32, &'static [&'static str], &'static [&'static str])> {
    if live { vec![(1, &TANKS, &TANK_FIELDS), (2, &[CLOCK], &SIGNAL_FIELDS)] } else { vec![(1, &ITEMS, &FIELDS)] }
}

/// A field of an update.
#[derive(Clone, PartialEq, Debug)]
pub enum Field {
    /// This many fields, from here on, are as they were.
    Same(usize),
    Null,
    Value(String),
}

/// A line of an answer.
#[derive(Clone, PartialEq, Debug)]
pub enum Line {
    /// The session's made: its id.
    Session(String),
    Update { sub: u32, item: usize, fields: Vec<Field> },
    /// The poll's done: poll again.
    Loop,
    /// The session's over, or was never made, or a request was refused: why.
    End(String),
    Refused(String),
    /// Anything else (the server's name, the client's address, a request done, a keep-alive).
    Other,
}

/// `%XX`s back into what they stand for.
fn decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(byte) = text.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn fields(text: &str) -> Vec<Field> {
    text.split('|')
        .map(|f| match f {
            "" => Field::Same(1),
            "#" => Field::Null,
            "$" => Field::Value(String::new()),
            _ => match f.strip_prefix('^').and_then(|n| n.parse().ok()) {
                Some(n) => Field::Same(n),
                None => Field::Value(decoded(f)),
            },
        })
        .collect()
}

/// An answer's lines.
pub fn parse(answer: &str) -> Vec<Line> {
    answer
        .lines()
        .map(|line| {
            let line = line.trim_end_matches('\r');
            let (tag, rest) = line.split_once(',').unwrap_or((line, ""));
            match tag {
                "CONOK" => Line::Session(rest.split(',').next().unwrap_or_default().to_string()),
                "U" => {
                    let mut parts = rest.splitn(3, ',');
                    match (parts.next().and_then(|s| s.parse().ok()), parts.next().and_then(|s| s.parse().ok()), parts.next()) {
                        (Some(sub), Some(item), Some(values)) => Line::Update { sub, item, fields: fields(values) },
                        _ => Line::Other,
                    }
                }
                "LOOP" => Line::Loop,
                "END" | "CONERR" => Line::End(rest.to_string()),
                "REQERR" | "ERROR" => Line::Refused(rest.to_string()),
                _ => Line::Other,
            }
        })
        .collect()
}

/// What's known of each item, by its name and its field's: as the updates of a session have left it.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Known(HashMap<(&'static str, &'static str), String>);

impl Known {
    /// Takes in an update for subscription `sub`, item `item`, of a session with `subs`.
    pub fn take(&mut self, subs: &[(u32, &'static [&'static str], &'static [&'static str])], sub: u32, item: usize, update: &[Field]) {
        let Some((_, items, schema)) = subs.iter().find(|s| s.0 == sub) else { return };
        let Some(name) = item.checked_sub(1).and_then(|i| items.get(i)) else { return };
        let mut at = 0;
        for field in update {
            match field {
                Field::Same(n) => at += n,
                Field::Null => {
                    if let Some(key) = schema.get(at) {
                        self.0.remove(&(*name, *key));
                    }
                    at += 1;
                }
                Field::Value(v) => {
                    if let Some(key) = schema.get(at) {
                        self.0.insert((*name, *key), v.clone());
                    }
                    at += 1;
                }
            }
        }
    }

    pub fn get(&self, item: &str, field: &str) -> Option<&str> {
        self.0.iter().find(|((i, f), _)| *i == item && *f == field).map(|(_, v)| v.as_str())
    }

    pub fn number(&self, item: &str, field: &str) -> Option<f64> {
        self.get(item, field)?.trim().parse().ok().filter(|v: &f64| v.is_finite())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Days since 1970 of a date (Howard Hinnant's days_from_civil).
pub(crate) fn days(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The year a day since 1970 is in.
fn year_of(days_since: i64) -> i64 {
    let z = days_since + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    yoe + era * 400 + i64::from(month <= 2)
}

/// A telemetry time stamp as seconds since 1970, given the time now (seconds since 1970): in this year, or in the last, if this
/// year's would be more than a day ahead of now (a stamp from the 31st of December, read just after New Year).
pub fn when(stamp: f64, now: i64) -> Option<i64> {
    if !stamp.is_finite() || stamp < 24.0 {
        return None;
    }
    let seconds = ((stamp - 24.0) * 3600.0).round() as i64;
    let year = year_of(now.div_euclid(86_400));
    let this = days(year, 1, 1) * 86_400 + seconds;
    Some(if this > now + 86_400 { days(year - 1, 1, 1) * 86_400 + seconds } else { this })
}

/// What a check got, 2026-11-17 at 15:20 GMT.
#[cfg(test)]
pub(crate) const FIXTURE: &str = "CONOK,S1bc9db29d9da68d6Maa0T2054044,50000,0,*\r\nSERVNAME,Lightstreamer HTTP Server\r\nCLIENTIP,203.0.113.7\r\nCONS,unlimited\r\nSUBOK,1,5,3\r\nCONF,1,unlimited,filtered\r\nU,1,1,29|7718.118305555582|24\r\nU,1,2,32|7713.197138611078|24\r\nU,1,5,27789650398|7719.347332777778|24\r\nU,1,3,81.4199981689453125|7719.3468627777365|24\r\nU,1,4,66.69000244140625|7719.3468627777365|24\r\nLOOP,15000\r\n";

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn the_check_is_the_address_that_worked() {
        assert_eq!(
            snapshot(),
            "https://push.lightstreamer.com/lightstreamer/create_session.txt?LS_protocol=TLCP-2.5.0&LS_adapter_set=ISSLIVE&LS_cid=mgQkwtwdysogQz2BJ4Ji%20kOj2Bg&LS_polling=true&LS_polling_millis=0&LS_idle_millis=0&LS_op=add&LS_subId=1&LS_group=NODE3000005%20NODE3000004%20NODE3000008%20NODE3000009%20TIME_000001&LS_schema=Value%20TimeStamp%20Status.Class&LS_mode=MERGE&LS_snapshot=true"
        );
        for url in [snapshot(), live(), add_signal("S1"), poll("S1", 30_000), destroy("S1")] {
            assert!(url.starts_with("https://") && !url.contains(' '), "{url}");
        }
        assert!(destroy("Sabc").contains("LS_op=destroy") && destroy("Sabc").contains("LS_session=Sabc") && destroy("Sabc").contains("LS_reqId="));
    }

    #[test]
    fn a_check_comes_back_as_each_items_fields() {
        let lines = parse(FIXTURE);
        assert_eq!(lines[0], Line::Session("S1bc9db29d9da68d6Maa0T2054044".into()));
        assert_eq!(lines.last(), Some(&Line::Loop));
        let mut known = Known::default();
        for line in &lines {
            if let Line::Update { sub, item, fields } = line {
                known.take(&subscriptions(false), *sub, *item, fields);
            }
        }
        assert_eq!(known.number(URINE, "Value"), Some(29.0));
        assert_eq!(known.number(PROCESSOR, "Value"), Some(32.0));
        let hundredths = |item| known.number(item, "Value").map(|v| (v * 100.0).round() as i64);
        assert_eq!((hundredths(WASTE), hundredths(DRINKING)), (Some(8142), Some(6669)));
        assert_eq!(known.get(CLOCK, "Status.Class"), Some(SIGNAL));
        assert_eq!(known.number(URINE, "TimeStamp"), Some(7718.118305555582));
    }

    #[test]
    fn a_later_update_says_only_what_changed() {
        let subs = subscriptions(false);
        let mut known = Known::default();
        known.take(&subs, 1, 1, &fields("29|7718.1|24"));
        known.take(&subs, 1, 1, &fields("30||"));
        assert_eq!((known.get(URINE, "Value"), known.get(URINE, "TimeStamp"), known.get(URINE, "Status.Class")), (Some("30"), Some("7718.1"), Some("24")));
        known.take(&subs, 1, 1, &fields("^2|9"));
        assert_eq!((known.get(URINE, "Value"), known.get(URINE, "Status.Class")), (Some("30"), Some("9")));
        known.take(&subs, 1, 1, &fields("#|$|%2324"));
        assert_eq!((known.get(URINE, "Value"), known.get(URINE, "TimeStamp"), known.get(URINE, "Status.Class")), (None, Some(""), Some("#24")));
        known.take(&subs, 7, 1, &fields("1|2|3"));
        known.take(&subs, 1, 99, &fields("1|2|3"));
        known.take(&subs, 1, 0, &fields("1|2|3"));
        assert_eq!(known.get(URINE, "TimeStamp"), Some(""), "what's no subscription's or no item's is nobody's");
        // Live, the clock's its own subscription, with only its class.
        let live = subscriptions(true);
        let mut known = Known::default();
        known.take(&live, 2, 1, &fields("9"));
        known.take(&live, 1, 4, &fields("72.99|7710.1"));
        assert_eq!((known.get(CLOCK, "Status.Class"), known.number(DRINKING, "Value")), (Some("9"), Some(72.99)));
    }

    #[test]
    fn the_rest_of_what_can_come() {
        assert_eq!(parse("END,31,Destroy invoked by client"), [Line::End("31,Destroy invoked by client".into())]);
        assert_eq!(parse("CONERR,2,Requested Adapter Set not available"), [Line::End("2,Requested Adapter Set not available".into())]);
        assert_eq!(parse("REQERR,1,19,Specified subscription not found"), [Line::Refused("1,19,Specified subscription not found".into())]);
        assert_eq!(parse("REQOK,9\r\nPROBE\r\nU,1,x,1"), [Line::Other, Line::Other, Line::Other]);
    }

    #[test]
    fn time_stamps_are_hours_of_the_year_with_new_years_day_as_day_one() {
        // 2026-11-17 15:20 GMT.
        let now = days(2026, 11, 17) * 86_400 + 15 * 3600 + 20 * 60;
        assert_eq!(when(7710.12, now), Some(days(2026, 11, 17) * 86_400 + 6 * 3600 + 7 * 60 + 12), "the 17th of November, 06:07");
        assert_eq!(when(24.0, now), Some(days(2026, 1, 1) * 86_400), "midnight on New Year's Day");
        // Just after New Year, a stamp from the 31st of December is last year's.
        let new_year = days(2027, 1, 1) * 86_400 + 600;
        assert_eq!(when(365.0 * 24.0 + 23.5, new_year), Some(days(2026, 12, 31) * 86_400 + 23 * 3600 + 1800));
        assert_eq!(when(24.1, new_year), Some(days(2027, 1, 1) * 86_400 + 360));
        // A leap year has a 366th day.
        assert_eq!(when(366.0 * 24.0 + 1.0, days(2028, 12, 31) * 86_400 + 7200), Some(days(2028, 12, 31) * 86_400 + 3600));
        assert_eq!(when(f64::NAN, now), None);
        assert_eq!(when(3.0, now), None);
        assert_eq!(year_of(days(2026, 1, 1)), 2026);
        assert_eq!(year_of(days(2026, 12, 31)), 2026);
        assert_eq!(year_of(days(2000, 2, 29)), 2000);
    }
}
