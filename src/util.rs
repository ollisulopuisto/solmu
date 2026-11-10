//! Small formatting helpers shared by the views.

use jiff::civil::Date;
use jiff::{Timestamp, Zoned};

use crate::i18n::Locale;

mod day_date;

/// Normalizes supported formatting without changing an international recipient.
/// Accepts 7 to 15 ASCII digits, an optional leading plus, and balanced brackets.
pub fn international_phone(input: &str) -> Option<String> {
    let input = input.trim();
    let input = input.strip_prefix('+').unwrap_or(input);
    let mut digits = String::new();
    let mut bracket = false;
    for character in input.chars() {
        match character {
            '0'..='9' => digits.push(character),
            ' ' | '-' => {}
            '(' if !bracket => bracket = true,
            ')' if bracket => bracket = false,
            _ => return None,
        }
    }
    (!bracket && (7..=15).contains(&digits.len()) && !digits.starts_with('0')).then_some(digits)
}

/// File-loader identifier for a native path. egui requires a slash after
/// `file://` on Windows or it interprets a drive path as a UNC hostname.
/// Keep native characters: egui's loader does not percent-decode URLs.
pub fn image_uri(path: &std::path::Path) -> String {
    image_uri_for_platform(&path.to_string_lossy(), cfg!(windows))
}

fn image_uri_for_platform(path: &str, windows: bool) -> String {
    format!("file://{}{path}", if windows { "/" } else { "" })
}

/// The local time zone.
fn zone() -> jiff::tz::TimeZone {
    #[cfg(test)]
    if let Some(clock) = fixed_clock::get() {
        return clock.zone;
    }
    jiff::tz::TimeZone::system()
}

/// Converts a Unix timestamp to local time.
fn zoned(unix_seconds: i64) -> Option<Zoned> {
    let timestamp = Timestamp::from_second(unix_seconds).ok()?;
    Some(timestamp.to_zoned(zone()))
}

/// Today's local date.
pub fn today() -> Date {
    #[cfg(test)]
    if let Some(when) = fixed_clock::get().and_then(|clock| zoned(clock.now)) {
        return when.date();
    }
    Zoned::now().date()
}

/// Whether the system shows times on a 12-hour clock. Read once per run.
pub fn twelve_hour_clock() -> bool {
    #[cfg(test)]
    if let Some(clock) = fixed_clock::get() {
        return clock.twelve_hour;
    }
    static TWELVE_HOUR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TWELVE_HOUR.get_or_init(clock_preference::twelve_hour)
}

/// Whether a time pattern uses a 12-hour hour field. Windows patterns spell
/// it `h`, ICU patterns `h` or `K`, and C formats `%I`, `%l`, or `%r`.
fn pattern_is_twelve_hour(pattern: &str) -> bool {
    if pattern.contains('%') {
        return ["%I", "%l", "%r", "%p"]
            .iter()
            .any(|field| pattern.contains(field));
    }
    // Skip quoted literals such as 'h' in "HH 'h' mm".
    let mut quoted = false;
    for character in pattern.chars() {
        match character {
            '\'' => quoted = !quoted,
            'h' | 'K' if !quoted => return true,
            _ => {}
        }
    }
    false
}

#[cfg(target_os = "linux")]
mod clock_preference {
    pub fn twelve_hour() -> bool {
        gnome().unwrap_or_else(locale)
    }

    /// GNOME keeps its own clock format, independent of the locale. It is
    /// read through the desktop portal, which works inside a Flatpak too.
    fn gnome() -> Option<bool> {
        if !fastframe_desktop::is_gnome() {
            return None;
        }
        clock_format(
            fastframe_desktop::setting("org.gnome.desktop.interface", "clock-format").as_deref(),
        )
    }

    /// Whether GNOME's `clock-format` names a 12-hour clock, or `None` for
    /// anything but its two values.
    pub(super) fn clock_format(value: Option<&str>) -> Option<bool> {
        match value? {
            "12h" => Some(true),
            "24h" => Some(false),
            _ => None,
        }
    }

    /// The time locale's own format, read without changing the process locale.
    fn locale() -> bool {
        // SAFETY: an empty name selects the environment's LC_TIME; the locale
        // is freed after its format string has been copied.
        unsafe {
            let locale = libc::newlocale(libc::LC_TIME_MASK, c"".as_ptr(), std::ptr::null_mut());
            if locale.is_null() {
                return false;
            }
            let format = libc::nl_langinfo_l(libc::T_FMT, locale);
            let twelve = !format.is_null()
                && super::pattern_is_twelve_hour(
                    &std::ffi::CStr::from_ptr(format).to_string_lossy(),
                );
            libc::freelocale(locale);
            twelve
        }
    }
}

#[cfg(target_os = "macos")]
mod clock_preference {
    use objc2_foundation::{NSDateFormatter, NSLocale, NSString};

    /// The "j" template asks for the locale's preferred hour, which follows
    /// the 24-hour switch in System Settings.
    pub fn twelve_hour() -> bool {
        let locale = NSLocale::currentLocale();
        NSDateFormatter::dateFormatFromTemplate_options_locale(
            &NSString::from_str("j"),
            0,
            Some(&locale),
        )
        .is_some_and(|pattern| super::pattern_is_twelve_hour(&pattern.to_string()))
    }
}

#[cfg(windows)]
mod clock_preference {
    use windows_sys::Win32::Globalization::{GetLocaleInfoEx, LOCALE_STIMEFORMAT};

    /// The user's time format from Region settings, such as "h:mm:ss tt".
    pub fn twelve_hour() -> bool {
        let mut buffer = [0u16; 80];
        // SAFETY: a null name means the user's default locale; the buffer
        // length is in UTF-16 units.
        let written = unsafe {
            GetLocaleInfoEx(
                std::ptr::null(),
                LOCALE_STIMEFORMAT,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
            )
        };
        written > 0
            && super::pattern_is_twelve_hour(&String::from_utf16_lossy(
                &buffer[..written as usize - 1],
            ))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod clock_preference {
    pub fn twelve_hour() -> bool {
        false
    }
}

/// The regional locales Solmu formats timestamps for. The OS region and the
/// UI language are independent choices: a Finn running Solmu in English
/// still reads "eilen klo 20:13". A region Solmu has no formats for keeps
/// the UI language's formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Finnish,
    EnglishUs,
}

impl Region {
    /// Maps an OS locale identifier (BCP 47 or POSIX, with an optional
    /// codeset or modifier) to a known region by its language and region
    /// subtags, so `fi_FI.UTF-8` and `fi-FI` both resolve to Finnish.
    fn from_identifier(identifier: &str) -> Option<Region> {
        let identifier = identifier
            .split(['.', '@'])
            .next()?
            .replace('_', "-")
            .to_lowercase();
        let mut subtags = identifier.split('-');
        let language = subtags.next()?;
        match language {
            "fi" => Some(Region::Finnish),
            "en" if subtags.next() == Some("us") => Some(Region::EnglishUs),
            _ => None,
        }
    }
}

/// The OS regional locale, read once per run.
pub fn region() -> Option<Region> {
    static REGION: std::sync::OnceLock<Option<Region>> = std::sync::OnceLock::new();
    *REGION
        .get_or_init(|| region_preference::identifier().and_then(|id| Region::from_identifier(&id)))
}

#[cfg(target_os = "linux")]
mod region_preference {
    /// The time locale's name, from the same variables the C library reads:
    /// LC_ALL wins over LC_TIME, which wins over LANG.
    pub fn identifier() -> Option<String> {
        ["LC_ALL", "LC_TIME", "LANG"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find(|value| !value.is_empty())
    }
}

#[cfg(target_os = "macos")]
mod region_preference {
    use objc2_foundation::NSLocale;

    /// The Region choice in System Settings, such as "fi_FI".
    pub fn identifier() -> Option<String> {
        Some(NSLocale::currentLocale().localeIdentifier().to_string())
    }
}

#[cfg(windows)]
mod region_preference {
    use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;

    /// The Region choice in Settings, such as "fi-FI".
    pub fn identifier() -> Option<String> {
        // LOCALE_NAME_MAX_LENGTH.
        let mut buffer = [0u16; 85];
        // SAFETY: the buffer length is in UTF-16 units.
        let written = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), buffer.len() as i32) };
        (written > 0).then(|| String::from_utf16_lossy(&buffer[..written as usize - 1]))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod region_preference {
    pub fn identifier() -> Option<String> {
        None
    }
}

/// Finnish weekday names, Monday first. Finnish writes weekdays and months
/// in lowercase.
const FI_WEEKDAYS: [&str; 7] = [
    "maanantai",
    "tiistai",
    "keskiviikko",
    "torstai",
    "perjantai",
    "lauantai",
    "sunnuntai",
];

/// The same weekdays in the essive case, for "on Tuesday at ...":
/// "tiistaina klo 22:13".
const FI_WEEKDAYS_ESSIVE: [&str; 7] = [
    "maanantaina",
    "tiistaina",
    "keskiviikkona",
    "torstaina",
    "perjantaina",
    "lauantaina",
    "sunnuntaina",
];

/// Finnish month names in the partitive, as a date words them: "14.
/// marraskuuta 2023".
const FI_MONTHS_PARTITIVE: [&str; 12] = [
    "tammikuuta",
    "helmikuuta",
    "maaliskuuta",
    "huhtikuuta",
    "toukokuuta",
    "kesäkuuta",
    "heinäkuuta",
    "elokuuta",
    "syyskuuta",
    "lokakuuta",
    "marraskuuta",
    "joulukuuta",
];

/// Weekday index with Monday as 0.
fn monday_index(weekday: jiff::civil::Weekday) -> usize {
    match weekday {
        jiff::civil::Weekday::Monday => 0,
        jiff::civil::Weekday::Tuesday => 1,
        jiff::civil::Weekday::Wednesday => 2,
        jiff::civil::Weekday::Thursday => 3,
        jiff::civil::Weekday::Friday => 4,
        jiff::civil::Weekday::Saturday => 5,
        jiff::civil::Weekday::Sunday => 6,
    }
}

/// The language stamp words come from: the region's own where it sets one,
/// else the UI language.
fn word_locale(region: Option<Region>, locale: Locale) -> Locale {
    match region {
        Some(Region::EnglishUs) => Locale::English,
        _ => locale,
    }
}

/// A weekday name in the region's own language where the region sets one,
/// else the UI language's.
fn regional_weekday(
    region: Option<Region>,
    locale: Locale,
    weekday: jiff::civil::Weekday,
) -> String {
    match region {
        Some(Region::Finnish) => FI_WEEKDAYS[monday_index(weekday)].to_owned(),
        Some(Region::EnglishUs) => weekday_name(Locale::English, weekday),
        None => weekday_name(locale, weekday),
    }
}

/// The numeric date piece of a bubble stamp, in the region's convention:
/// "14.11." in Finland, "11/14" in the US, and the zero-padded "14.11."
/// where no region rule applies. The year joins only when it differs from
/// today's.
fn bubble_date(region: Option<Region>, when: &Zoned, today: Date) -> String {
    let with_year = when.year() != today.year();
    match (region, with_year) {
        (Some(Region::Finnish), true) => {
            format!("{}.{}.{}", when.day(), when.month(), when.year())
        }
        (Some(Region::Finnish), false) => format!("{}.{}.", when.day(), when.month()),
        (Some(Region::EnglishUs), true) => {
            format!("{}/{}/{}", when.month(), when.day(), when.year())
        }
        (Some(Region::EnglishUs), false) => format!("{}/{}", when.month(), when.day()),
        (None, true) => format!("{:02}.{:02}.{}", when.day(), when.month(), when.year()),
        (None, false) => format!("{:02}.{:02}.", when.day(), when.month()),
    }
}

/// Time of day on the system's clock, such as "14:05" or "2:05 PM".
fn hour_minute(when: &Zoned) -> String {
    time_of_day(when.hour(), when.minute(), twelve_hour_clock())
}

/// Formats an hour and minute of day as 24-hour "14:05" or 12-hour "2:05 PM".
fn time_of_day(hour: i8, minute: i8, twelve_hour: bool) -> String {
    if twelve_hour {
        let (hour, meridiem) = match hour {
            0 => (12, "AM"),
            1..=11 => (hour, "AM"),
            12 => (12, "PM"),
            _ => (hour - 12, "PM"),
        };
        format!("{hour}:{minute:02} {meridiem}")
    } else {
        format!("{hour:02}:{minute:02}")
    }
}

/// Formats an hour, minute and second of day as 24-hour "14:05:09" or
/// 12-hour "2:05:09 PM".
fn time_with_seconds(hour: i8, minute: i8, second: i8, twelve_hour: bool) -> String {
    if twelve_hour {
        let (hour, meridiem) = match hour {
            0 => (12, "AM"),
            1..=11 => (hour, "AM"),
            12 => (12, "PM"),
            _ => (hour - 12, "PM"),
        };
        format!("{hour}:{minute:02}:{second:02} {meridiem}")
    } else {
        format!("{hour:02}:{minute:02}:{second:02}")
    }
}

/// Local message time such as "14:05".
pub fn clock(unix_seconds: i64) -> String {
    zoned(unix_seconds)
        .map(|when| hour_minute(&when))
        .unwrap_or_default()
}

/// Bubble timestamp: the time today, the day named for the two days before
/// it, then the date with the time in the OS region's convention - "eilen
/// 20:13" and "toissa päivänä 20:13" on a Finnish system, "Yesterday 2:05
/// PM" and "11/1 2:05 PM" on a US one.
pub fn bubble_stamp(locale: Locale, unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    bubble_stamp_relative_to(region(), locale, &when, today())
}

fn bubble_stamp_relative_to(
    region: Option<Region>,
    locale: Locale,
    when: &Zoned,
    today: Date,
) -> String {
    let time = hour_minute(when);
    let days = today
        .since(when.date())
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    // A future day is a clock skew, not a date to name.
    if days <= 0 {
        return time;
    }
    if let Some(Region::Finnish) = region {
        return match days {
            1 => format!("eilen {time}"),
            2 => format!("toissa päivänä {time}"),
            _ => format!("{} {time}", bubble_date(region, when, today)),
        };
    }
    let words = word_locale(region, locale);
    match days {
        1 => crate::i18n::gettext(words, "Yesterday {time}").replace("{time}", &time),
        2 => {
            crate::i18n::gettext(words, "The day before yesterday {time}").replace("{time}", &time)
        }
        _ => format!("{} {time}", bubble_date(region, when, today)),
    }
}

/// The full moment, with seconds and the local zone: "4.11.2026 13:03:45
/// EET" on a Finnish system, "11/4/2026 1:03:45 PM EET" on a US one.
/// Message info shows it for the message itself, where the bubble's short
/// stamp has no room for the date.
pub fn full_stamp(unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    full_stamp_in(region(), &when)
}

fn full_stamp_in(region: Option<Region>, when: &Zoned) -> String {
    let zone = when.strftime("%Z");
    match region {
        Some(Region::Finnish) => {
            let time = time_with_seconds(
                when.hour(),
                when.minute(),
                when.second(),
                twelve_hour_clock(),
            );
            format!(
                "{}.{}.{} {time} {zone}",
                when.day(),
                when.month(),
                when.year()
            )
        }
        Some(Region::EnglishUs) => {
            let time = time_with_seconds(
                when.hour(),
                when.minute(),
                when.second(),
                twelve_hour_clock(),
            );
            format!(
                "{}/{}/{} {time} {zone}",
                when.month(),
                when.day(),
                when.year()
            )
        }
        None => when.strftime("%d.%m.%Y %H:%M:%S %Z").to_string(),
    }
}

/// WhatsApp transcript timestamp such as `22:41, 8/18/2026`, or `20:13,
/// 4.11.2026` on a Finnish system.
pub fn copy_stamp(unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    copy_stamp_in(region(), &when)
}

fn copy_stamp_in(region: Option<Region>, when: &Zoned) -> String {
    if let Some(Region::Finnish) = region {
        return format!(
            "{}, {}.{}.{}",
            hour_minute(when),
            when.day(),
            when.month(),
            when.year()
        );
    }
    format!(
        "{}:{:02}, {}/{}/{}",
        when.hour(),
        when.minute(),
        when.month(),
        when.day(),
        when.year()
    )
}

/// Chat-row timestamp: time today, weekday this week, or date, worded in
/// the OS region's language and date convention.
pub fn chat_stamp(locale: Locale, unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    stamp_relative_to(region(), locale, when.date(), today(), &when)
}

fn stamp_relative_to(
    region: Option<Region>,
    locale: Locale,
    date: Date,
    today: Date,
    when: &Zoned,
) -> String {
    let days = today
        .since(date)
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    match days {
        0 => hour_minute(when),
        1 => match region {
            Some(Region::Finnish) => "eilen".to_owned(),
            _ => crate::i18n::gettext(word_locale(region, locale), "Yesterday").into_owned(),
        },
        2..=6 => regional_weekday(region, locale, date.weekday()),
        _ => short_date_in(region, locale, date),
    }
}

/// The first-name and last-name fields of the contact editor for a saved
/// `name`, given the first name saved with it.
///
/// Both may hold several words, so the name is never cut at a space: the
/// last name is what follows the saved first name. Without a first name to go
/// by (a profile name, a contact synced before first names were kept, or a
/// first name the full name does not start with) the whole name stays in the
/// first field, so saving it unchanged keeps it whole.
pub fn editor_names(name: &str, first: Option<&str>) -> (String, String) {
    let name = name.trim();
    if let Some(first) = first.map(str::trim).filter(|first| !first.is_empty())
        && let Some(rest) = name.strip_prefix(first)
        && (rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
        return (first.to_owned(), rest.trim().to_owned());
    }
    (name.to_owned(), String::new())
}

/// Message-info timestamp with date and minute, worded in the OS region's
/// language: "eilen klo 20:13" on a Finnish system, "Yesterday at 8:13 PM"
/// on a US one.
pub fn moment_stamp(locale: Locale, unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    moment_stamp_relative_to(region(), locale, &when, today())
}

fn moment_stamp_relative_to(
    region: Option<Region>,
    locale: Locale,
    when: &Zoned,
    today: Date,
) -> String {
    let time = hour_minute(when);
    let days = today
        .since(when.date())
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    if let Some(Region::Finnish) = region {
        return match days {
            0 => time,
            1 => format!("eilen klo {time}"),
            2..=6 => format!(
                "{} klo {time}",
                FI_WEEKDAYS_ESSIVE[monday_index(when.date().weekday())]
            ),
            _ => format!("{} klo {time}", short_date_in(region, locale, when.date())),
        };
    }
    let words = word_locale(region, locale);
    match days {
        0 => time,
        1 => crate::i18n::gettext(words, "Yesterday at {time}").replace("{time}", &time),
        2..=6 => crate::i18n::gettext(words, "{weekday} at {time}")
            .replace(
                "{weekday}",
                &regional_weekday(region, locale, when.date().weekday()),
            )
            .replace("{time}", &time),
        _ => crate::i18n::gettext(words, "{date} at {time}")
            .replace("{date}", &short_date_in(region, locale, when.date()))
            .replace("{time}", &time),
    }
}

/// The chat info line for a chat's disappearing-message timer, as WhatsApp
/// words it: "Disappearing messages: 24 hours", "Disappearing messages: 7
/// days", "Disappearing messages: 90 days", and "Disappearing messages: Off"
/// for a timer WhatsApp turned off. A duration WhatsApp never offers falls
/// back to its seconds, so an unexpected value reads as the number it is
/// rather than as nothing.
///
/// `None` draws no line at all: nobody has told us this chat's timer, and
/// saying so would be a guess.
pub fn disappearing_line(locale: Locale, seconds: Option<u32>) -> Option<String> {
    let seconds = seconds?;
    use crate::i18n::gettext;
    let value = match seconds {
        0 => gettext(locale, "Off").into_owned(),
        86_400 => gettext(locale, "24 hours").into_owned(),
        604_800 => gettext(locale, "7 days").into_owned(),
        7_776_000 => gettext(locale, "90 days").into_owned(),
        _ => format!("{seconds}s"),
    };
    let label = gettext(locale, "Disappearing messages");
    Some(format!("{label}: {value}"))
}

/// A contact's last-seen line as WhatsApp words it: "last seen today at
/// 14:05", "last seen yesterday at 14:05", a weekday within the week, and
/// only the date before that. The time follows the system's clock.
pub fn last_seen(locale: Locale, unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    last_seen_relative_to(locale, today(), &when)
}

fn last_seen_relative_to(locale: Locale, today: Date, when: &Zoned) -> String {
    use crate::i18n::gettext;
    let date = when.date();
    let time = hour_minute(when);
    let days = today
        .since(date)
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    match days {
        // A clock slightly ahead of ours still means today.
        ..=0 => gettext(locale, "last seen today at {time}").replace("{time}", &time),
        1 => gettext(locale, "last seen yesterday at {time}").replace("{time}", &time),
        2..=6 => {
            let weekday = weekday_name(locale, date.weekday());
            // Mid-sentence, only English and German keep the capital.
            let weekday = match locale {
                Locale::English | Locale::German => weekday,
                _ => weekday.to_lowercase(),
            };
            gettext(locale, "last seen {weekday} at {time}")
                .replace("{weekday}", &weekday)
                .replace("{time}", &time)
        }
        _ => gettext(locale, "last seen {date}").replace("{date}", &short_date(locale, date)),
    }
}

/// Conversation day separator: localized names for the last three calendar
/// days, then a numeric date in the OS region's order, with a year only when
/// it differs from today's. Bubble clocks and message-info moments are separate.
pub fn day_label(locale: Locale, unix_seconds: i64) -> String {
    let Some(when) = zoned(unix_seconds) else {
        return String::new();
    };
    day_label_relative_to(
        region(),
        locale,
        when.date(),
        today(),
        day_date::pattern(region(), locale),
    )
}

fn day_label_relative_to(
    region: Option<Region>,
    locale: Locale,
    date: Date,
    today: Date,
    pattern: &str,
) -> String {
    let days = today
        .since(date)
        .map(|span| span.get_days())
        .unwrap_or(i32::MAX);
    let words = word_locale(region, locale);
    match (region, days) {
        (Some(Region::Finnish), 0) => "Tänään".to_owned(),
        (Some(Region::Finnish), 1) => "Eilen".to_owned(),
        (Some(Region::Finnish), 2) => "Toissapäivänä".to_owned(),
        (_, 0) => crate::i18n::gettext(words, "Today").into_owned(),
        (_, 1) => crate::i18n::gettext(words, "Yesterday").into_owned(),
        (_, 2) => crate::i18n::gettext(words, "Day before yesterday").into_owned(),
        _ => day_date::format(date, date.year() != today.year(), pattern).unwrap_or_else(|| {
            day_date::format(
                date,
                date.year() != today.year(),
                day_date::fallback_pattern(locale),
            )
            .expect("numeric fallback date pattern")
        }),
    }
}

/// Local calendar day used to group messages.
pub fn day_key(unix_seconds: i64) -> Option<Date> {
    zoned(unix_seconds).map(|when| when.date())
}

/// Unix-second half-open range for a local calendar day.
pub fn day_bounds(date: Date) -> Option<(i64, i64)> {
    day_bounds_in(date, &zone())
}

/// [`day_bounds`] in `zone`. A day starts at its first instant, so one whose
/// midnight a clock change skips starts when the clock resumes, and the
/// range is 23 or 25 hours long around a change.
fn day_bounds_in(date: Date, zone: &jiff::tz::TimeZone) -> Option<(i64, i64)> {
    let start = date.to_zoned(zone.clone()).ok()?.start_of_day().ok()?;
    let end = date
        .tomorrow()
        .ok()?
        .to_zoned(zone.clone())
        .ok()?
        .start_of_day()
        .ok()?;
    Some((start.timestamp().as_second(), end.timestamp().as_second()))
}

fn weekday_name(locale: Locale, weekday: jiff::civil::Weekday) -> String {
    use crate::i18n::gettext;
    // Each literal sits in its own call so xgettext can extract it.
    match weekday {
        jiff::civil::Weekday::Monday => gettext(locale, "Monday"),
        jiff::civil::Weekday::Tuesday => gettext(locale, "Tuesday"),
        jiff::civil::Weekday::Wednesday => gettext(locale, "Wednesday"),
        jiff::civil::Weekday::Thursday => gettext(locale, "Thursday"),
        jiff::civil::Weekday::Friday => gettext(locale, "Friday"),
        jiff::civil::Weekday::Saturday => gettext(locale, "Saturday"),
        jiff::civil::Weekday::Sunday => gettext(locale, "Sunday"),
    }
    .into_owned()
}

fn month_name(locale: Locale, month: i8) -> String {
    use crate::i18n::gettext;
    match month {
        1 => gettext(locale, "January"),
        2 => gettext(locale, "February"),
        3 => gettext(locale, "March"),
        4 => gettext(locale, "April"),
        5 => gettext(locale, "May"),
        6 => gettext(locale, "June"),
        7 => gettext(locale, "July"),
        8 => gettext(locale, "August"),
        9 => gettext(locale, "September"),
        10 => gettext(locale, "October"),
        11 => gettext(locale, "November"),
        _ => gettext(locale, "December"),
    }
    .into_owned()
}

/// A month's name and year, for the day filter's header.
pub fn month_heading(locale: Locale, month: Date) -> String {
    format!("{} {}", month_name(locale, month.month()), month.year())
}

/// The two-letter weekday headings, Monday first, for the day filter's grid.
/// Taken by character, so a name that is not ASCII is not split mid-glyph.
pub fn weekday_headings(locale: Locale) -> [String; 7] {
    use jiff::civil::Weekday;
    [
        Weekday::Monday,
        Weekday::Tuesday,
        Weekday::Wednesday,
        Weekday::Thursday,
        Weekday::Friday,
        Weekday::Saturday,
        Weekday::Sunday,
    ]
    .map(|weekday| weekday_name(locale, weekday).chars().take(2).collect())
}

/// A date as "23 Sep 2026", in the UI language. [`regional_short_date`]
/// follows the OS region instead.
pub fn short_date(locale: Locale, date: Date) -> String {
    short_date_in(None, locale, date)
}

/// A date in the OS region's convention: "23.9.2026" in Finland, "Sep 23,
/// 2026" in the US, and the UI language's "23 Sep 2026" where no region
/// rule applies.
pub fn regional_short_date(locale: Locale, date: Date) -> String {
    short_date_in(region(), locale, date)
}

fn short_date_in(region: Option<Region>, locale: Locale, date: Date) -> String {
    match region {
        Some(Region::Finnish) => format!("{}.{}.{}", date.day(), date.month(), date.year()),
        Some(Region::EnglishUs) => {
            let month: String = month_name(Locale::English, date.month())
                .chars()
                .take(3)
                .collect();
            format!("{month} {}, {}", date.day(), date.year())
        }
        None => {
            let month: String = month_name(locale, date.month()).chars().take(3).collect();
            format!("{} {month} {}", date.day(), date.year())
        }
    }
}

/// A date with its weekday, as "Wednesday, 23 September 2026", in the UI
/// language. [`regional_long_date`] follows the OS region instead.
pub fn long_date(locale: Locale, date: Date) -> String {
    long_date_in(None, locale, date)
}

/// A date with its weekday in the OS region's convention: "keskiviikko 23.
/// syyskuuta 2026" in Finland, "Wednesday, September 23, 2026" in the US.
pub fn regional_long_date(locale: Locale, date: Date) -> String {
    long_date_in(region(), locale, date)
}

fn long_date_in(region: Option<Region>, locale: Locale, date: Date) -> String {
    match region {
        Some(Region::Finnish) => format!(
            "{} {}. {} {}",
            FI_WEEKDAYS[monday_index(date.weekday())],
            date.day(),
            FI_MONTHS_PARTITIVE[(date.month() - 1) as usize],
            date.year()
        ),
        Some(Region::EnglishUs) => format!(
            "{}, {} {}, {}",
            weekday_name(Locale::English, date.weekday()),
            month_name(Locale::English, date.month()),
            date.day(),
            date.year()
        ),
        None => format!(
            "{}, {} {} {}",
            weekday_name(locale, date.weekday()),
            date.day(),
            month_name(locale, date.month()),
            date.year()
        ),
    }
}

/// The current time as a Unix timestamp.
pub fn now() -> i64 {
    #[cfg(test)]
    if let Some(clock) = fixed_clock::get() {
        return clock.now;
    }
    Timestamp::now().as_second()
}

/// A clock for tests whose layout depends on the time: the current time, the
/// time zone, and the clock format, fixed for the calling thread until the
/// returned guard drops. Otherwise a sample built relative to now gains a day
/// separator or wider times depending on when and where the test runs.
#[cfg(test)]
pub mod fixed_clock {
    use std::cell::RefCell;

    #[derive(Clone)]
    pub struct Clock {
        pub now: i64,
        pub zone: jiff::tz::TimeZone,
        pub twelve_hour: bool,
    }

    thread_local! {
        static CLOCK: RefCell<Option<Clock>> = const { RefCell::new(None) };
    }

    pub(super) fn get() -> Option<Clock> {
        CLOCK.with(|clock| clock.borrow().clone())
    }

    /// Restores the thread's previous clock when dropped.
    #[must_use]
    pub struct Guard(Option<Clock>);

    impl Drop for Guard {
        fn drop(&mut self) {
            CLOCK.with(|clock| *clock.borrow_mut() = self.0.take());
        }
    }

    pub fn set(clock: Clock) -> Guard {
        Guard(CLOCK.with(|slot| slot.borrow_mut().replace(clock)))
    }
}

/// Case- and accent-insensitive matching without changing displayed names.
pub fn search_key(text: &str) -> String {
    use icu_normalizer::DecomposingNormalizerBorrowed;
    use icu_properties::{CodePointMapDataBorrowed, props::GeneralCategory};
    use std::sync::OnceLock;

    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    // Performance optimization:
    // 1. In Unicode, GeneralCategory::NonspacingMark characters (accents/diacritics)
    //    begin above ASCII (U+0300 Combining Diacritical Marks). Fast-path check
    //    `c.is_ascii()` inside the iterator filter to avoid table lookups for ASCII
    //    characters produced during NFD decomposition.
    // 2. Cache the static `CodePointMapDataBorrowed` and `DecomposingNormalizerBorrowed`
    //    via `OnceLock` instead of constructing owned ICU tables and structures
    //    on every call.
    static CATEGORIES: OnceLock<CodePointMapDataBorrowed<'static, GeneralCategory>> =
        OnceLock::new();
    let categories = CATEGORIES.get_or_init(CodePointMapDataBorrowed::<GeneralCategory>::new);
    const NORMALIZER: DecomposingNormalizerBorrowed<'static> =
        DecomposingNormalizerBorrowed::new_nfd();

    let mut key = String::with_capacity(text.len());
    let filtered_chars = NORMALIZER
        .normalize_iter(text.chars())
        .filter(|c| c.is_ascii() || categories.get(*c) != GeneralCategory::NonspacingMark)
        .flat_map(char::to_lowercase);
    key.extend(filtered_chars);
    key
}

/// Duration such as "0:12".
pub fn duration(seconds: u32) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// File size such as "1.2 MB".
pub fn bytes(size: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = size as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{size} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Up to two initials for a fallback avatar.
pub fn initials(name: &str) -> String {
    let mut words = name
        .split(|character: char| character.is_whitespace() || character == '-')
        .filter(|word| word.chars().any(char::is_alphanumeric));
    let first = words.next();
    let last = words.next_back();
    let mut initials = String::new();
    for word in [first, last].into_iter().flatten() {
        if let Some(character) = word.chars().find(|character| character.is_alphanumeric()) {
            initials.extend(character.to_uppercase());
        }
    }
    if initials.is_empty() {
        initials.push('#');
    }
    initials
}

/// A finished call's length in words, as WhatsApp's call info gives it:
/// "45 s" under a minute, "12 min 34 s" under an hour, "1 h 2 min" past
/// one. A part that is zero is left out ("12 min", "1 h"), and each form is
/// a message of its own so a language orders and abbreviates the units its
/// own way.
pub fn call_length(locale: Locale, seconds: u64) -> String {
    use crate::i18n::gettext;
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    let text = match (hours, minutes, seconds) {
        (0, 0, _) => gettext(locale, "{seconds} s"),
        (0, _, 0) => gettext(locale, "{minutes} min"),
        (0, _, _) => gettext(locale, "{minutes} min {seconds} s"),
        (_, 0, _) => gettext(locale, "{hours} h"),
        _ => gettext(locale, "{hours} h {minutes} min"),
    };
    text.replace("{hours}", &hours.to_string())
        .replace("{minutes}", &minutes.to_string())
        .replace("{seconds}", &seconds.to_string())
}

/// A running call's clock, as the call screen counts it: `m:ss`, or
/// `h:mm:ss` past an hour.
pub fn call_duration(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Formats a phone number with a plus sign and country-appropriate grouping.
pub fn phone(digits: &str) -> String {
    use std::borrow::Cow;
    // Fast-path: borrow directly when `digits` is already purely ASCII digits
    // (the common case from JIDs and normalized contacts), avoiding a heap
    // allocation of an intermediate String.
    let digits: Cow<'_, str> = if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        Cow::Borrowed(digits)
    } else {
        let filtered: String = digits.chars().filter(char::is_ascii_digit).collect();
        if filtered.is_empty() {
            return String::new();
        }
        Cow::Owned(filtered)
    };
    if let Some(formatted) = brazilian_phone(&digits).or_else(|| french_phone(&digits)) {
        return formatted;
    }
    let (country, national) = digits.split_at(country_code_len(&digits).min(digits.len()));
    // Pre-allocate capacity for "+{country} " + grouped chunks with spaces.
    let estimated_capacity = 1 + country.len() + 1 + national.len() + national.len() / 3;
    let mut out = String::with_capacity(estimated_capacity);
    out.push('+');
    out.push_str(country);
    // Approximate national grouping: threes, with the last four together
    // rather than leaving a lone digit.
    let mut start = 0;
    while start < national.len() {
        let end = match national.len() - start {
            4 => national.len(),
            _ => (start + 3).min(national.len()),
        };
        out.push(' ');
        out.push_str(&national[start..end]);
        start = end;
    }
    out
}

/// Length of the ITU-T E.164 country calling code `digits` starts with.
///
/// The codes are prefix-free: only 1 and 7 have one digit, the two-digit
/// codes are listed here, and every other code has three. Instead of a
/// linear `starts_with` scan over all 44 two-digit codes on every call, match
/// on the first two bytes (O(1), jump-table-friendly). `digits` is ASCII-only
/// and non-empty by the time `phone` calls this.
fn country_code_len(digits: &str) -> usize {
    let mut bytes = digits.bytes();
    match (bytes.next(), bytes.next()) {
        // One-digit country codes: 1 (NANP) and 7 (Russia/Kazakhstan).
        (Some(b'1'), _) | (Some(b'7'), _) => 1,
        // Two-digit country codes, prefix-free with the one-digit codes.
        (Some(b'2'), Some(b'0'))
        | (Some(b'2'), Some(b'7'))
        | (Some(b'3'), Some(b'0'))
        | (Some(b'3'), Some(b'1'))
        | (Some(b'3'), Some(b'2'))
        | (Some(b'3'), Some(b'3'))
        | (Some(b'3'), Some(b'4'))
        | (Some(b'3'), Some(b'6'))
        | (Some(b'3'), Some(b'9'))
        | (Some(b'4'), Some(b'0'))
        | (Some(b'4'), Some(b'1'))
        | (Some(b'4'), Some(b'3'))
        | (Some(b'4'), Some(b'4'))
        | (Some(b'4'), Some(b'5'))
        | (Some(b'4'), Some(b'6'))
        | (Some(b'4'), Some(b'7'))
        | (Some(b'4'), Some(b'8'))
        | (Some(b'4'), Some(b'9'))
        | (Some(b'5'), Some(b'1'))
        | (Some(b'5'), Some(b'2'))
        | (Some(b'5'), Some(b'3'))
        | (Some(b'5'), Some(b'4'))
        | (Some(b'5'), Some(b'5'))
        | (Some(b'5'), Some(b'6'))
        | (Some(b'5'), Some(b'7'))
        | (Some(b'5'), Some(b'8'))
        | (Some(b'6'), Some(b'0'))
        | (Some(b'6'), Some(b'1'))
        | (Some(b'6'), Some(b'2'))
        | (Some(b'6'), Some(b'3'))
        | (Some(b'6'), Some(b'4'))
        | (Some(b'6'), Some(b'5'))
        | (Some(b'6'), Some(b'6'))
        | (Some(b'8'), Some(b'1'))
        | (Some(b'8'), Some(b'2'))
        | (Some(b'8'), Some(b'4'))
        | (Some(b'8'), Some(b'6'))
        | (Some(b'9'), Some(b'0'))
        | (Some(b'9'), Some(b'1'))
        | (Some(b'9'), Some(b'2'))
        | (Some(b'9'), Some(b'3'))
        | (Some(b'9'), Some(b'4'))
        | (Some(b'9'), Some(b'5'))
        | (Some(b'9'), Some(b'8')) => 2,
        _ => 3,
    }
}

/// Formats Brazil's `+55` numbers as `(DDD) XXXX-XXXX` or `(DDD) XXXXX-XXXX`.
///
/// WhatsApp stores direct-chat ids in international form, while Brazilian
/// numbers use a two-digit area code and either eight-digit fixed lines or
/// nine-digit mobile numbers.
fn brazilian_phone(digits: &str) -> Option<String> {
    let national = digits.strip_prefix("55")?;
    let subscriber_len = match national.len() {
        10 | 11 => national.len() - 2,
        _ => return None,
    };
    let area = &national[..2];
    let subscriber = &national[2..2 + subscriber_len];
    let split = subscriber.len() - 4;
    Some(format!(
        "+55 ({area}) {}-{}",
        &subscriber[..split],
        &subscriber[split..]
    ))
}

/// Formats France's `+33` numbers as `+33 6 12 34 56 78`, and the overseas
/// departments that share its nine-digit plan (`+262` Réunion and Mayotte,
/// `+590` Guadeloupe, `+594` French Guiana, `+596` Martinique) as
/// `+262 692 12 34 56`.
///
/// French numbers are written in pairs. A metropolitan number leads with
/// one digit, except the `8` service numbers, which lead with three like the
/// overseas ones; these are libphonenumber's international formats.
fn french_phone(digits: &str) -> Option<String> {
    let (country, national) = if let Some(rest) = digits.strip_prefix("33") {
        ("33", rest)
    } else if let Some(rest) = digits.strip_prefix("262") {
        ("262", rest)
    } else if let Some(rest) = digits.strip_prefix("590") {
        ("590", rest)
    } else if let Some(rest) = digits.strip_prefix("594") {
        ("594", rest)
    } else if let Some(rest) = digits.strip_prefix("596") {
        ("596", rest)
    } else {
        return None;
    };
    if national.len() != 9 {
        return None;
    }
    let lead = if country == "33" && !national.starts_with('8') {
        1
    } else {
        3
    };
    // Pre-allocate capacity: "+{country} " + national digits with spaces.
    let mut out = String::with_capacity(1 + country.len() + 1 + national.len() + 5);
    out.push('+');
    out.push_str(country);
    out.push(' ');
    out.push_str(&national[..lead]);
    for start in (lead..national.len()).step_by(2) {
        out.push(' ');
        out.push_str(&national[start..start + 2]);
    }
    Some(out)
}

/// The machine's hostname, or an empty string if it cannot be read.
pub fn hostname() -> String {
    gethostname::gethostname().to_string_lossy().into_owned()
}

/// Stable id-derived avatar hue.
pub fn hue(seed: &str) -> f32 {
    let mut hash: u32 = 2_166_136_261;
    for byte in seed.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(16_777_619);
    }
    (hash % 360) as f32
}

/// Embedded SVG app logo used across platform surfaces.
const MARK: &[u8] = include_bytes!("../packaging/icons/zapfast.svg");
/// The same mark without its rim and shading, which blur below this size.
const SMALL_MARK: &[u8] = include_bytes!("../packaging/icons/zapfast-small.svg");
const SMALL_BELOW: usize = 40;

/// Rasterizes the logo to straight-alpha RGBA.
pub fn app_icon_rgba(size: usize) -> Vec<u8> {
    let mark = if size < SMALL_BELOW { SMALL_MARK } else { MARK };
    match render_mark(mark, size) {
        Some(rgba) => rgba,
        // Fall back to an accent disc if the embedded SVG cannot render.
        None => plain_disc(size),
    }
}

fn render_mark(mark: &[u8], size: usize) -> Option<Vec<u8>> {
    let side = size.max(1) as u32;
    let tree = resvg::usvg::Tree::from_data(mark, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(side, side)?;
    let scale = side as f32 / tree.size().width();
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Some(
        pixmap
            .pixels()
            .iter()
            .flat_map(|pixel| {
                let color = pixel.demultiply();
                [color.red(), color.green(), color.blue(), color.alpha()]
            })
            .collect(),
    )
}

fn plain_disc(size: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; size * size * 4];
    let center = size as f32 / 2.0;
    let radius = center - 2.0;
    for y in 0..size {
        for x in 0..size {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let distance = ((px - center).powi(2) + (py - center).powi(2)).sqrt();
            let coverage = (radius - distance + 0.5).clamp(0.0, 1.0);
            let index = (y * size + x) * 4;
            rgba[index] = 0;
            rgba[index + 1] = 168;
            rgba[index + 2] = 132;
            rgba[index + 3] = (coverage * 255.0) as u8;
        }
    }
    rgba
}

/// Converts the logo to a monochrome macOS menu-bar template: the disc,
/// with the bubble cut out of it.
pub fn tray_template_rgba(size: usize) -> Vec<u8> {
    // The flat mark at every size: a template has no room for shading.
    let mut rgba = render_mark(SMALL_MARK, size).unwrap_or_else(|| plain_disc(size));
    // The disc's green against the ink's says how much of a pixel is disc.
    const INK: f32 = 14.0;
    const DISC: f32 = 168.0;
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let disc = ((f32::from(pixel[1]) - INK) / (DISC - INK)).clamp(0.0, 1.0);
        pixel[3] = (f32::from(pixel[3]) * disc).round() as u8;
        pixel[0] = 0;
        pixel[1] = 0;
        pixel[2] = 0;
    }
    rgba
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_call_length_reads_the_way_the_phone_shows_it() {
        assert_eq!(super::call_duration(0), "0:00");
        assert_eq!(super::call_duration(65), "1:05");
        assert_eq!(super::call_duration(3600 + 2 * 60 + 3), "1:02:03");
    }

    #[test]
    fn a_finished_call_length_reads_in_words() {
        use crate::i18n::Locale;
        let length = |seconds| super::call_length(Locale::English, seconds);
        assert_eq!(length(45), "45 s");
        assert_eq!(length(754), "12 min 34 s");
        assert_eq!(length(720), "12 min");
        assert_eq!(length(3725), "1 h 2 min");
        assert_eq!(length(3600), "1 h");
        assert_eq!(super::call_length(Locale::German, 754), "12 Min. 34 Sek.");
    }

    #[test]
    fn the_contact_editor_keeps_names_as_they_were_saved() {
        let names = |name, first| super::editor_names(name, first);
        let pair = |first: &str, last: &str| (first.to_owned(), last.to_owned());
        // A first name of several words, alone or with a last name (#314).
        assert_eq!(
            names("first second", Some("first second")),
            pair("first second", "")
        );
        assert_eq!(
            names("first second third", Some("first second")),
            pair("first second", "third")
        );
        assert_eq!(names("My Dih", Some("My Dih")), pair("My Dih", ""));
        // A first name the full name merely starts with, letter for letter,
        // is not the first word.
        assert_eq!(names("Mary Ann", Some("Mar")), pair("Mary Ann", ""));
        // Without a first name nothing is guessed at a space.
        assert_eq!(names(" Mary Ann Evans ", None), pair("Mary Ann Evans", ""));
        assert_eq!(names("Bob", Some("  ")), pair("Bob", ""));
        // Names written without spaces stay whole.
        assert_eq!(names("山田太郎", None), pair("山田太郎", ""));
        assert_eq!(names("山田太郎", Some("太郎")), pair("山田太郎", ""));
        assert_eq!(names("محمد علي", Some("محمد")), pair("محمد", "علي"));
    }

    #[test]
    fn a_day_filter_covers_the_local_day_across_clock_changes() {
        use jiff::civil::date;
        use jiff::tz::TimeZone;
        let second = |text: &str| {
            text.parse::<jiff::Timestamp>()
                .expect("a timestamp")
                .as_second()
        };
        // POSIX rules, so the test needs no time zone database.
        let berlin = TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").expect("a zone");
        assert_eq!(
            super::day_bounds_in(date(2026, 9, 23), &berlin),
            Some((
                second("2026-09-22T22:00:00Z"),
                second("2026-09-23T22:00:00Z")
            )),
            "an ordinary day runs from local midnight to local midnight"
        );
        let (from, until) = super::day_bounds_in(date(2026, 3, 29), &berlin).expect("a range");
        assert_eq!(until - from, 23 * 3_600, "spring forward loses an hour");
        let (from, until) = super::day_bounds_in(date(2026, 10, 25), &berlin).expect("a range");
        assert_eq!(until - from, 25 * 3_600, "fall back gains one");
        // Brazil once sprang forward at midnight: that day began at 01:00.
        let brasilia = TimeZone::posix("<-03>3<-02>,M11.1.0/0,M2.3.0/0").expect("a zone");
        assert_eq!(
            super::day_bounds_in(date(2018, 11, 4), &brasilia),
            Some((
                second("2018-11-04T03:00:00Z"),
                second("2018-11-05T02:00:00Z")
            ))
        );
    }

    /// GNOME's clock format comes from the portal as the bare value, not
    /// the quoted text `gsettings` printed.
    #[cfg(target_os = "linux")]
    #[test]
    fn gnome_clock_formats_are_recognized() {
        use super::clock_preference::clock_format;
        assert_eq!(clock_format(Some("12h")), Some(true));
        assert_eq!(clock_format(Some("24h")), Some(false));
        assert_eq!(clock_format(Some("'12h'")), None);
        assert_eq!(clock_format(Some("")), None);
        assert_eq!(clock_format(None), None);
    }

    #[test]
    fn clock_patterns_from_every_platform_are_recognized() {
        for pattern in [
            "h:mm:ss tt",
            "h a",
            "K:mm a",
            "%r",
            "%I:%M:%S %p",
            "%l:%M %p",
        ] {
            assert!(super::pattern_is_twelve_hour(pattern), "{pattern}");
        }
        for pattern in ["HH:mm:ss", "H:mm", "%T", "%H:%M:%S", "HH 'h' mm", "HH"] {
            assert!(!super::pattern_is_twelve_hour(pattern), "{pattern}");
        }
    }

    #[test]
    fn time_of_day_switches_between_24_and_12_hour() {
        assert_eq!(time_of_day(0, 5, false), "00:05");
        assert_eq!(time_of_day(14, 5, false), "14:05");
        assert_eq!(time_of_day(0, 5, true), "12:05 AM");
        assert_eq!(time_of_day(9, 5, true), "9:05 AM");
        assert_eq!(time_of_day(12, 0, true), "12:00 PM");
        assert_eq!(time_of_day(14, 5, true), "2:05 PM");
        assert_eq!(time_of_day(23, 59, true), "11:59 PM");
    }

    #[test]
    fn image_paths_keep_the_native_path_after_loader_conversion() {
        for path in [
            r"C:\Users\Ada\photo.jpg",
            r"C:\Users\A B\100% #猫.png",
            r"\\server\share\photo.jpg",
            r"\\?\C:\cache\photo.jpg",
        ] {
            let uri = super::image_uri_for_platform(path, true);
            // Mirrors egui_extras' Windows file loader: its first slash
            // selects a native path, otherwise it prepends a UNC prefix.
            assert_eq!(uri.strip_prefix("file:///").unwrap(), path);
        }
        assert_eq!(
            super::image_uri_for_platform("/home/ada/猫 #1.png", false),
            "file:///home/ada/猫 #1.png"
        );
    }

    #[test]
    fn image_loader_reads_native_paths() {
        use egui::load::BytesPoll;
        let dir = std::env::temp_dir().join(format!("solmu-image-paths-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("猫 photo 100% #1.png");
        std::fs::write(&path, b"image bytes").unwrap();
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        let uri = super::image_uri(&path);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match ctx.try_load_bytes(&uri).unwrap() {
                BytesPoll::Ready { bytes, .. } => {
                    assert_eq!(bytes.as_ref(), b"image bytes");
                    break;
                }
                BytesPoll::Pending { .. } => {
                    assert!(std::time::Instant::now() < deadline);
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    use super::*;

    #[test]
    fn initials_take_first_and_last_word() {
        assert_eq!(initials("Ada Lovelace"), "AL");
        assert_eq!(initials("ada"), "A");
        assert_eq!(initials("  "), "#");
        assert_eq!(initials("🎉 Party Planning"), "PP");
    }

    #[test]
    fn the_shipped_icon_draws_without_svg_filters() {
        // Qt's SVG renderer rejects filter primitives such as feFuncA, and a
        // tray that asks for the themed icon then draws a missing-icon
        // marker (#396).
        let svg = std::str::from_utf8(MARK).expect("the icon is UTF-8");
        assert!(!svg.contains("<filter"), "packaging/icons/zapfast.svg");
    }

    #[test]
    fn phone_numbers_are_grouped() {
        assert_eq!(phone("393331234567"), "+39 333 123 4567");
        assert_eq!(phone("15551234567"), "+1 555 123 4567");
        assert_eq!(phone("79161234567"), "+7 916 123 4567");
        assert_eq!(phone("584241234567"), "+58 424 123 4567");
        assert_eq!(phone("2348031234567"), "+234 803 123 4567");
        assert_eq!(phone("97150123456"), "+971 501 234 56");
        assert_eq!(phone("5511999999999"), "+55 (11) 99999-9999");
        assert_eq!(phone("551140028922"), "+55 (11) 4002-8922");
        assert_eq!(phone("551149508333"), "+55 (11) 4950-8333");
        assert_eq!(phone("+55 (11) 99999-9999"), "+55 (11) 99999-9999");
        assert_eq!(phone("33612345678"), "+33 6 12 34 56 78");
        assert_eq!(phone("33142685300"), "+33 1 42 68 53 00");
        assert_eq!(phone("33899123456"), "+33 899 12 34 56");
        assert_eq!(phone("+33 6 12 34 56 78"), "+33 6 12 34 56 78");
        assert_eq!(phone("262692123456"), "+262 692 12 34 56");
        assert_eq!(phone("590690123456"), "+590 690 12 34 56");
        assert_eq!(phone("594694123456"), "+594 694 12 34 56");
        assert_eq!(phone("596696123456"), "+596 696 12 34 56");
        // A French number of the wrong length keeps the general grouping.
        assert_eq!(phone("3361234567"), "+33 612 345 67");
        assert_eq!(phone(""), "");
    }

    #[test]
    fn stamps_fall_back_to_dates() {
        let when = Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC);
        let date = when.date();
        assert_eq!(
            stamp_relative_to(None, Locale::English, date, date, &when),
            time_of_day(22, 13, twelve_hour_clock())
        );
        assert_eq!(
            stamp_relative_to(
                None,
                Locale::English,
                date,
                date.tomorrow().expect("date"),
                &when
            ),
            "Yesterday"
        );
        assert_eq!(
            stamp_relative_to(
                None,
                Locale::English,
                date,
                date.checked_add(jiff::Span::new().days(3)).expect("date"),
                &when
            ),
            "Tuesday"
        );
        assert_eq!(
            stamp_relative_to(
                None,
                Locale::English,
                date,
                date.checked_add(jiff::Span::new().days(30)).expect("date"),
                &when
            ),
            "14 Nov 2023"
        );
    }

    #[test]
    fn bubble_stamps_name_two_days_then_show_dates() {
        let when = Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC);
        let date = when.date(); // 14 Nov 2023, 22:13 UTC
        let plus = |days: i64| {
            date.checked_add(jiff::Span::new().days(days))
                .expect("date")
        };
        let stamp = |today| super::bubble_stamp_relative_to(None, Locale::English, &when, today);
        let time = time_of_day(22, 13, twelve_hour_clock());
        assert_eq!(stamp(date), time);
        assert_eq!(stamp(plus(1)), format!("Yesterday {time}"));
        assert_eq!(stamp(plus(2)), format!("The day before yesterday {time}"));
        // Three days back the date takes over, without the year this year.
        assert_eq!(stamp(plus(3)), format!("14.11. {time}"));
        // Months later but still the same year: still no year.
        assert_eq!(stamp(plus(45)), format!("14.11. {time}"));
        // A previous year carries the full year.
        assert_eq!(stamp(plus(400)), format!("14.11.2023 {time}"));
        // A future timestamp is clock skew, not a named day.
        assert_eq!(stamp(plus(-1)), time);
    }

    /// Last seen reads as on the phone: the time today and yesterday, the
    /// weekday and time within the week, and the date after that.
    #[test]
    fn last_seen_names_the_day_and_the_time() {
        // Tuesday 14 November 2023, 22:13 UTC.
        let when = Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC);
        let date = when.date();
        let time = time_of_day(22, 13, twelve_hour_clock());
        let later = |days: i64| {
            date.checked_add(jiff::Span::new().days(days))
                .expect("date")
        };
        let english = |today| last_seen_relative_to(Locale::English, today, &when);
        assert_eq!(english(date), format!("last seen today at {time}"));
        assert_eq!(
            english(date.yesterday().expect("date")),
            format!("last seen today at {time}"),
            "a timestamp slightly ahead of our clock is still today"
        );
        assert_eq!(english(later(1)), format!("last seen yesterday at {time}"));
        assert_eq!(english(later(3)), format!("last seen Tuesday at {time}"));
        assert_eq!(english(later(6)), format!("last seen Tuesday at {time}"));
        assert_eq!(english(later(7)), "last seen 14 Nov 2023");
        assert_eq!(english(later(30)), "last seen 14 Nov 2023");

        // Mid-sentence weekdays keep their capital only where the language
        // writes one.
        assert_eq!(
            last_seen_relative_to(Locale::Italian, later(3), &when),
            format!("ultimo accesso martedì alle {time}")
        );
        assert_eq!(
            last_seen_relative_to(Locale::German, later(3), &when),
            format!("zuletzt online am Dienstag um {time}")
        );
        for locale in Locale::ALL {
            for days in [0, 1, 3, 30] {
                let line = last_seen_relative_to(locale, later(days), &when);
                assert!(!line.contains('{'), "{locale:?} {days}: {line}");
            }
        }
    }

    #[test]
    fn short_dates_take_whole_characters_in_every_locale() {
        for locale in Locale::ALL {
            for month in 1..=12 {
                let date = jiff::civil::date(2024, month, 5);
                let short = short_date(locale, date);
                assert!(
                    short.starts_with("5 ") && short.ends_with(" 2024"),
                    "{short}"
                );
            }
        }
        assert_eq!(
            short_date(Locale::German, jiff::civil::date(2024, 3, 5)),
            "5 Mär 2024"
        );
    }

    #[test]
    fn brazilian_portuguese_stamps_are_translated() {
        let when = Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC);
        let date = when.date();
        assert_eq!(
            stamp_relative_to(
                None,
                Locale::PortugueseBrazil,
                date,
                date.tomorrow().expect("date"),
                &when
            ),
            "Ontem"
        );
        assert_eq!(
            stamp_relative_to(
                None,
                Locale::PortugueseBrazil,
                date,
                date.checked_add(jiff::Span::new().days(3)).expect("date"),
                &when
            ),
            "Terça-feira"
        );
    }

    #[test]
    fn turkish_stamps_are_translated() {
        let when = Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC);
        let date = when.date();
        assert_eq!(
            stamp_relative_to(
                None,
                Locale::Turkish,
                date,
                date.tomorrow().expect("date"),
                &when
            ),
            "Dün"
        );
        assert_eq!(
            stamp_relative_to(
                None,
                Locale::Turkish,
                date,
                date.checked_add(jiff::Span::new().days(3)).expect("date"),
                &when
            ),
            "Salı"
        );
    }

    #[test]
    fn sizes_and_durations_read_naturally() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(2_048), "2.0 KB");
        assert_eq!(bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(duration(75), "1:15");
    }

    /// Only WhatsApp's own durations get a name, an explicit zero reads as
    /// off, and a chat whose timer nobody has mentioned has no line at all.
    #[test]
    fn disappearing_timers_read_as_whatsapp_words_them() {
        let line = |seconds| {
            crate::util::disappearing_line(Locale::English, seconds)
                .unwrap_or_default()
                .replace("Disappearing messages: ", "")
        };
        assert_eq!(line(Some(0)), "Off");
        assert_eq!(line(Some(86_400)), "24 hours");
        assert_eq!(line(Some(604_800)), "7 days");
        assert_eq!(line(Some(7_776_000)), "90 days");
        assert_eq!(line(Some(3_600)), "3600s");
        assert_eq!(
            crate::util::disappearing_line(Locale::English, None),
            None,
            "an unknown timer says nothing"
        );
    }

    #[test]
    fn icon_is_opaque_in_the_middle_and_clear_at_the_corners() {
        let icon = app_icon_rgba(32);
        assert_eq!(icon[3], 0);
        let middle = (16 * 32 + 16) * 4;
        assert_eq!(icon[middle + 3], 255);
    }

    /// The menu-bar template is the disc with the bubble cut out: solid in
    /// the bubble's middle, clear along its outline and outside the disc.
    #[test]
    fn the_tray_template_cuts_the_bubble_out_of_the_disc() {
        let size = 64;
        let template = tray_template_rgba(size);
        let alpha = |x: usize, y: usize| template[(y * size + x) * 4 + 3];
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(32, 32), 255, "inside the bubble is disc");
        assert_eq!(alpha(32, 4), 255, "so is the rim above it");
        let outline = (0..32).map(|y| alpha(32, y)).min().unwrap();
        assert!(outline < 40, "the outline is cut out: {outline}");
        assert!(
            template
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[..3] == [0, 0, 0])
        );
    }

    /// Both marks render, the large one with its filters.
    #[test]
    fn both_marks_render() {
        assert!(render_mark(MARK, 128).is_some());
        assert!(render_mark(SMALL_MARK, 22).is_some());
    }

    #[test]
    fn search_key_normalizes_ascii_and_unicode_accents() {
        assert_eq!(super::search_key("Hello World"), "hello world");
        assert_eq!(super::search_key("CAFÉ"), "cafe");
        assert_eq!(super::search_key("São Paulo"), "sao paulo");
        assert_eq!(super::search_key("München"), "munchen");
        assert_eq!(super::search_key("Crème Brûlée"), "creme brulee");
    }

    #[test]
    fn os_region_identifiers_resolve() {
        use super::Region;
        let region = Region::from_identifier;
        assert_eq!(region("fi_FI.UTF-8"), Some(Region::Finnish));
        assert_eq!(region("fi-FI"), Some(Region::Finnish));
        assert_eq!(region("fi"), Some(Region::Finnish));
        assert_eq!(region("en-US"), Some(Region::EnglishUs));
        assert_eq!(region("en_US.UTF-8"), Some(Region::EnglishUs));
        assert_eq!(region("en-GB"), None);
        assert_eq!(region("de-DE"), None);
        assert_eq!(region("sv-SE"), None);
        assert_eq!(region("C"), None);
        assert_eq!(region(""), None);
    }

    /// Tuesday 14 November 2023, 22:13:20 UTC.
    fn sample_moment() -> Zoned {
        Timestamp::from_second(1_700_000_000)
            .expect("valid")
            .to_zoned(jiff::tz::TimeZone::UTC)
    }

    #[test]
    fn finnish_region_stamps_read_in_finnish() {
        let fi = Some(Region::Finnish);
        let when = sample_moment();
        let date = when.date();
        let time = time_of_day(22, 13, twelve_hour_clock());
        let plus = |days: i64| {
            date.checked_add(jiff::Span::new().days(days))
                .expect("date")
        };

        // Bubble stamps: the named days are Finnish, the date keeps the
        // Finnish day.month order without leading zeros.
        let stamp = |today| bubble_stamp_relative_to(fi, Locale::English, &when, today);
        assert_eq!(stamp(date), time);
        assert_eq!(stamp(plus(1)), format!("eilen {time}"));
        assert_eq!(stamp(plus(2)), format!("toissa päivänä {time}"));
        assert_eq!(stamp(plus(3)), format!("14.11. {time}"));
        assert_eq!(stamp(plus(400)), format!("14.11.2023 {time}"));

        // Chat-row stamps.
        assert_eq!(
            stamp_relative_to(fi, Locale::English, date, plus(1), &when),
            "eilen"
        );
        assert_eq!(
            stamp_relative_to(fi, Locale::English, date, plus(3), &when),
            "tiistai"
        );
        assert_eq!(
            stamp_relative_to(fi, Locale::English, date, plus(30), &when),
            "14.11.2023"
        );

        // Day-separator labels.
        let label = |today| day_label_relative_to(fi, Locale::English, date, today, "dd.MM.yyyy");
        assert_eq!(label(date), "Tänään");
        assert_eq!(label(plus(1)), "Eilen");
        assert_eq!(label(plus(2)), "Toissapäivänä");
        assert_eq!(label(plus(3)), "14.11.");
        assert_eq!(label(plus(30)), "14.11.");
        assert_eq!(label(plus(400)), "14.11.2023");

        // Message-info moments take the essive weekday with "klo".
        let moment = |today| moment_stamp_relative_to(fi, Locale::English, &when, today);
        assert_eq!(moment(date), time);
        assert_eq!(moment(plus(1)), format!("eilen klo {time}"));
        assert_eq!(moment(plus(3)), format!("tiistaina klo {time}"));
        assert_eq!(moment(plus(30)), format!("14.11.2023 klo {time}"));

        // Dates, and the full and transcript stamps.
        assert_eq!(short_date_in(fi, Locale::English, date), "14.11.2023");
        assert_eq!(
            long_date_in(fi, Locale::English, date),
            "tiistai 14. marraskuuta 2023"
        );
        let seconds = time_with_seconds(22, 13, 20, twelve_hour_clock());
        assert_eq!(
            full_stamp_in(fi, &when),
            format!("14.11.2023 {seconds} UTC")
        );
        assert_eq!(copy_stamp_in(fi, &when), format!("{time}, 14.11.2023"));
    }

    #[test]
    fn day_separators_use_calendar_days_across_year_and_leap_boundaries() {
        use jiff::civil::date;
        let label =
            |date, today| day_label_relative_to(None, Locale::English, date, today, "dd.MM.yyyy");
        let new_year = date(2026, 1, 1);
        assert_eq!(label(new_year, new_year), "Today");
        assert_eq!(label(date(2025, 12, 31), new_year), "Yesterday");
        assert_eq!(label(date(2025, 12, 30), new_year), "Day before yesterday");
        assert_eq!(label(date(2025, 12, 29), new_year), "29.12.2025");
        assert_eq!(
            label(date(2024, 2, 29), date(2024, 3, 2)),
            "Day before yesterday"
        );
        assert_eq!(label(date(2024, 2, 29), date(2024, 3, 3)), "29.02.");
        assert_eq!(label(date(2026, 1, 2), new_year), "02.01.");
        assert_eq!(day_label(Locale::English, i64::MAX), "");
    }

    #[test]
    fn day_before_yesterday_is_translated_in_every_catalog() {
        let date = jiff::civil::date(2026, 3, 5);
        let today = jiff::civil::date(2026, 3, 7);
        for locale in Locale::ALL {
            let label = day_label_relative_to(None, locale, date, today, "dd.MM.yyyy");
            assert!(!label.is_empty());
            if locale != Locale::English {
                assert_ne!(label, "Day before yesterday", "{locale:?}");
            }
        }
    }

    #[test]
    fn us_region_stamps_use_us_date_order() {
        let us = Some(Region::EnglishUs);
        let when = sample_moment();
        let date = when.date();
        let time = time_of_day(22, 13, twelve_hour_clock());
        let plus = |days: i64| {
            date.checked_add(jiff::Span::new().days(days))
                .expect("date")
        };

        // The region, not the UI language, words the stamps: a German UI on
        // a US system still reads "Yesterday".
        assert_eq!(
            bubble_stamp_relative_to(us, Locale::German, &when, plus(1)),
            format!("Yesterday {time}")
        );
        assert_eq!(
            bubble_stamp_relative_to(us, Locale::English, &when, plus(2)),
            format!("The day before yesterday {time}")
        );
        assert_eq!(
            bubble_stamp_relative_to(us, Locale::English, &when, plus(3)),
            format!("11/14 {time}")
        );
        assert_eq!(
            bubble_stamp_relative_to(us, Locale::English, &when, plus(400)),
            format!("11/14/2023 {time}")
        );

        assert_eq!(
            stamp_relative_to(us, Locale::English, date, plus(1), &when),
            "Yesterday"
        );
        assert_eq!(
            stamp_relative_to(us, Locale::English, date, plus(3), &when),
            "Tuesday"
        );
        assert_eq!(
            stamp_relative_to(us, Locale::English, date, plus(30), &when),
            "Nov 14, 2023"
        );

        let label = |today| day_label_relative_to(us, Locale::English, date, today, "M/d/yyyy");
        assert_eq!(label(date), "Today");
        assert_eq!(label(plus(2)), "Day before yesterday");
        assert_eq!(label(plus(30)), "11/14");
        assert_eq!(label(plus(400)), "11/14/2023");

        let moment = |today| moment_stamp_relative_to(us, Locale::English, &when, today);
        assert_eq!(moment(plus(1)), format!("Yesterday at {time}"));
        assert_eq!(moment(plus(3)), format!("Tuesday at {time}"));
        assert_eq!(moment(plus(30)), format!("Nov 14, 2023 at {time}"));

        assert_eq!(short_date_in(us, Locale::English, date), "Nov 14, 2023");
        assert_eq!(
            long_date_in(us, Locale::English, date),
            "Tuesday, November 14, 2023"
        );
        let seconds = time_with_seconds(22, 13, 20, twelve_hour_clock());
        assert_eq!(
            full_stamp_in(us, &when),
            format!("11/14/2023 {seconds} UTC")
        );
        // The transcript stamp keeps WhatsApp's US shape.
        assert_eq!(copy_stamp_in(us, &when), "22:13, 11/14/2023");
    }
}

#[cfg(test)]
mod international_phone_tests {
    /// Supported formatting preserves every recipient digit and rejects extra suffixes.
    #[test]
    fn supported_formatting_never_swallows_extra_recipient_digits() {
        for input in ["+1 (555) 123-4567", "15551234567", "  +55 11 99999-9999  "] {
            assert!(super::international_phone(input).is_some(), "{input}");
        }
        for input in [
            "+1 (555) 123-4567 ext. 89",
            "05512345678",
            "1234567890123456",
            "123456",
            "++15551234567",
            "1+5551234567",
            "+1 (555 1234567",
            "١٥٥٥١٢٣٤٥٦٧",
        ] {
            assert!(super::international_phone(input).is_none(), "{input}");
        }
    }
}
