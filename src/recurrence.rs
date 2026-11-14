//! Repeating sends: the rule behind a Send Later series.
//!
//! A series is never expanded ahead of time. The queue holds exactly one
//! pending occurrence (a normal `ScheduledMessage` that carries its
//! `Repeat`); when that occurrence is `Sent`, the next one is computed from
//! this rule and queued. So every safety rule of the one-shot queue
//! (claim before send, `Uncertain` is never resent, `Missed` waits for a
//! decision) applies to each occurrence unchanged, and a crash can never
//! leave a pile of duplicate future items.
//!
//! The next occurrence is always strictly after "now": a series that was
//! down for a week does not catch up with a burst of sends.
//!
//! Wall-clock rules: "09:00" stays 09:00 local across daylight saving. A
//! time that does not exist that day moves to the next valid instant; a
//! time that happens twice fires at the first one. A month day that the
//! month lacks (the 31st) falls to the month's last day, and the series
//! returns to the original day the month after.

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp};
use serde::{Deserialize, Serialize};

/// Scan bound for the day-by-day search; far beyond any real rule.
const SCAN_LIMIT: u32 = 4000;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Unit {
    Day,
    Week,
    Month,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Repeat {
    /// Id of the first occurrence; every later occurrence keeps it.
    pub series: String,
    /// Every `every` days, weeks or months. At least 1, so the shortest
    /// interval is one day.
    pub every: u32,
    pub unit: Unit,
    /// Weeks only: bit 0 is Monday, bit 6 is Sunday. Empty means the
    /// weekday of the previous occurrence.
    #[serde(default)]
    pub weekdays: u8,
    /// Local wall time of every occurrence.
    pub hour: u8,
    pub minute: u8,
    /// Months only: the day of month to return to (1 to 31).
    #[serde(default)]
    pub day: u8,
    /// IANA zone the wall time is read in. Stored so travelling does not
    /// silently move 09:00. Empty falls back to the system zone.
    #[serde(default)]
    pub tz: String,
    /// Stop after this many sends.
    #[serde(default)]
    pub ends_after: Option<u32>,
    /// Stop after this local date (inclusive), `YYYY-MM-DD`.
    #[serde(default)]
    pub until: Option<String>,
    /// Occurrences sent so far.
    #[serde(default)]
    pub sent: u32,
    /// A paused series stays queued but is never sent.
    #[serde(default)]
    pub paused: bool,
    /// The date this occurrence was planned for, kept when it is sent or
    /// retried early or late, so the series keeps its phase. The next
    /// occurrence steps from here, not from the moment it actually went.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned: Option<i64>,
}

impl Repeat {
    /// Reject rules that would misbehave before they are queued.
    pub fn validate(&self) -> Result<(), String> {
        if self.every == 0 || self.every > 365 {
            return Err("repeat interval must be 1 to 365".into());
        }
        if self.hour > 23 || self.minute > 59 {
            return Err("repeat time is not a valid time of day".into());
        }
        if self.unit == Unit::Month && !(1..=31).contains(&self.day) {
            return Err("repeat day of month must be 1 to 31".into());
        }
        if self.weekdays & 0x80 != 0 {
            return Err("repeat weekdays are not valid".into());
        }
        if self.ends_after == Some(0) {
            return Err("a series must send at least once".into());
        }
        if let Some(until) = &self.until
            && until.parse::<Date>().is_err()
        {
            return Err("repeat end date must be YYYY-MM-DD".into());
        }
        if !self.tz.is_empty() && TimeZone::get(&self.tz).is_err() {
            return Err("repeat time zone is unknown".into());
        }
        Ok(())
    }

    fn zone(&self) -> TimeZone {
        if self.tz.is_empty() {
            TimeZone::system()
        } else {
            TimeZone::get(&self.tz).unwrap_or_else(|_| TimeZone::system())
        }
    }

    /// Short description for the queue: "every 3 days 09:00".
    pub fn describe(&self) -> String {
        const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
        let unit = match self.unit {
            Unit::Day => "day",
            Unit::Week => "week",
            Unit::Month => "month",
        };
        let mut out = if self.every == 1 {
            format!("every {unit}")
        } else {
            format!("every {} {unit}s", self.every)
        };
        if self.unit == Unit::Week && self.weekdays != 0 {
            let days: Vec<&str> = (0..7usize)
                .filter(|bit| self.weekdays & (1u8 << bit) != 0)
                .map(|bit| NAMES[bit])
                .collect();
            out.push_str(&format!(" on {}", days.join(" ")));
        }
        out.push_str(&format!(" {:02}:{:02}", self.hour, self.minute));
        out
    }

    /// The occurrence after `prev_due` (the one just sent or skipped) that
    /// lies strictly after `now`, or `None` when the series has ended.
    pub fn next_after(&self, prev_due: i64, now: i64) -> Option<i64> {
        self.validate().ok()?;
        let zone = self.zone();
        let mut date = Timestamp::from_second(prev_due)
            .ok()?
            .to_zoned(zone.clone())
            .date();
        for _ in 0..SCAN_LIMIT {
            date = self.step(date)?;
            let at = date
                .at(self.hour as i8, self.minute as i8, 0, 0)
                .to_zoned(zone.clone())
                .ok()?
                .timestamp()
                .as_second();
            if at > now {
                if let Some(until) = &self.until
                    && date > until.parse::<Date>().ok()?
                {
                    return None;
                }
                return Some(at);
            }
        }
        None
    }

    /// The first `count` occurrences starting at `first_due`, for the
    /// preview in the dialog.
    pub fn upcoming(&self, first_due: i64, count: usize) -> Vec<i64> {
        let mut out = vec![first_due];
        while out.len() < count {
            let last = *out.last().unwrap_or(&first_due);
            match self.next_after(last, last) {
                Some(next) => out.push(next),
                None => break,
            }
        }
        out
    }

    /// One rule step from `from` to the next calendar date.
    fn step(&self, from: Date) -> Option<Date> {
        let every = i64::from(self.every);
        match self.unit {
            Unit::Day => from.checked_add(Span::new().days(every)).ok(),
            Unit::Month => {
                let first = from
                    .first_of_month()
                    .checked_add(Span::new().months(every))
                    .ok()?;
                let day = (self.day.max(1) as i8).min(first.days_in_month());
                Date::new(first.year(), first.month(), day).ok()
            }
            Unit::Week => {
                let offset = i64::from(from.weekday().to_monday_zero_offset());
                let mask = if self.weekdays == 0 {
                    1u8 << offset
                } else {
                    self.weekdays
                };
                for ahead in 1..=(7 * every + 7) {
                    let pos = offset + ahead;
                    let week = pos / 7;
                    let weekday = pos % 7;
                    if mask & (1u8 << weekday) != 0 && week % every == 0 {
                        return from.checked_add(Span::new().days(ahead)).ok();
                    }
                }
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repeat(unit: Unit, every: u32) -> Repeat {
        Repeat {
            series: "s1".into(),
            every,
            unit,
            weekdays: 0,
            hour: 9,
            minute: 0,
            day: 15,
            tz: "Europe/Helsinki".into(),
            ends_after: None,
            until: None,
            sent: 0,
            paused: false,
            planned: None,
        }
    }

    fn at(date: &str, hour: i8, minute: i8) -> i64 {
        date.parse::<Date>()
            .unwrap()
            .at(hour, minute, 0, 0)
            .to_zoned(TimeZone::get("Europe/Helsinki").unwrap())
            .unwrap()
            .timestamp()
            .as_second()
    }

    fn local(ts: i64) -> String {
        Timestamp::from_second(ts)
            .unwrap()
            .to_zoned(TimeZone::get("Europe/Helsinki").unwrap())
            .strftime("%Y-%m-%d %H:%M")
            .to_string()
    }

    #[test]
    fn every_three_days() {
        let r = repeat(Unit::Day, 3);
        let first = at("2026-11-15", 9, 0);
        let next = r.next_after(first, first).unwrap();
        assert_eq!(local(next), "2026-11-18 09:00");
    }

    #[test]
    fn a_week_of_downtime_does_not_catch_up() {
        let r = repeat(Unit::Day, 1);
        let first = at("2026-11-01", 9, 0);
        let now = at("2026-11-08", 12, 0);
        let next = r.next_after(first, now).unwrap();
        assert_eq!(local(next), "2026-11-09 09:00");
    }

    #[test]
    fn wall_time_holds_across_the_spring_dst_change() {
        // Finland moves to summer time on 2027-03-28.
        let r = repeat(Unit::Day, 1);
        let first = at("2027-03-27", 9, 0);
        let next = r.next_after(first, first).unwrap();
        assert_eq!(local(next), "2027-03-28 09:00");
        assert_eq!(next - first, 23 * 3600);
    }

    #[test]
    fn a_time_inside_the_dst_gap_moves_forward() {
        // 03:30 does not exist in Helsinki on 2027-03-28.
        let mut r = repeat(Unit::Day, 1);
        r.hour = 3;
        r.minute = 30;
        let first = at("2027-03-27", 3, 30);
        let next = r.next_after(first, first).unwrap();
        assert_eq!(local(next), "2027-03-28 04:30");
    }

    #[test]
    fn weekdays_every_week() {
        let mut r = repeat(Unit::Week, 1);
        r.weekdays = 0b0000101; // Monday and Wednesday
        // 2026-11-16 is a Monday.
        let first = at("2026-11-16", 9, 0);
        let wed = r.next_after(first, first).unwrap();
        assert_eq!(local(wed), "2026-11-18 09:00");
        let mon = r.next_after(wed, wed).unwrap();
        assert_eq!(local(mon), "2026-11-23 09:00");
    }

    #[test]
    fn every_second_week_skips_the_off_week() {
        let mut r = repeat(Unit::Week, 2);
        r.weekdays = 0b0000001; // Monday
        let first = at("2026-11-16", 9, 0);
        let next = r.next_after(first, first).unwrap();
        assert_eq!(local(next), "2026-11-30 09:00");
    }

    #[test]
    fn week_with_no_weekdays_keeps_the_weekday_of_the_start() {
        let r = repeat(Unit::Week, 1);
        let first = at("2026-11-18", 9, 0); // Wednesday
        let next = r.next_after(first, first).unwrap();
        assert_eq!(local(next), "2026-11-25 09:00");
    }

    #[test]
    fn the_31st_falls_to_month_end_and_comes_back() {
        let mut r = repeat(Unit::Month, 1);
        r.day = 31;
        let jan = at("2027-01-31", 9, 0);
        let feb = r.next_after(jan, jan).unwrap();
        assert_eq!(local(feb), "2027-02-28 09:00");
        let mar = r.next_after(feb, feb).unwrap();
        assert_eq!(local(mar), "2027-03-31 09:00");
    }

    #[test]
    fn the_end_date_is_inclusive() {
        let mut r = repeat(Unit::Day, 1);
        r.until = Some("2026-11-17".into());
        let first = at("2026-11-15", 9, 0);
        let second = r.next_after(first, first).unwrap();
        let third = r.next_after(second, second).unwrap();
        assert_eq!(local(third), "2026-11-17 09:00");
        assert_eq!(r.next_after(third, third), None);
    }

    #[test]
    fn upcoming_lists_real_dates() {
        let r = repeat(Unit::Day, 3);
        let first = at("2026-11-15", 9, 0);
        let list: Vec<String> = r.upcoming(first, 3).into_iter().map(local).collect();
        assert_eq!(
            list,
            ["2026-11-15 09:00", "2026-11-18 09:00", "2026-11-21 09:00"]
        );
    }

    #[test]
    fn validation_rejects_bad_rules() {
        let mut r = repeat(Unit::Day, 0);
        assert!(r.validate().is_err());
        r.every = 1;
        r.hour = 24;
        assert!(r.validate().is_err());
        r.hour = 9;
        r.until = Some("not a date".into());
        assert!(r.validate().is_err());
        r.until = None;
        r.tz = "Mars/Olympus".into();
        assert!(r.validate().is_err());
        r.tz = "Europe/Helsinki".into();
        assert!(r.validate().is_ok());
    }

    #[test]
    fn describe_names_the_rule() {
        let mut r = repeat(Unit::Week, 2);
        r.weekdays = 0b0000101;
        assert_eq!(r.describe(), "every 2 weeks on Mon Wed 09:00");
        assert_eq!(repeat(Unit::Day, 1).describe(), "every day 09:00");
    }
}
