//! How dates and times are shown in the file lists and dialogs: the system's
//! formats (its locale's, or Windows' region settings), or the user's own
//! date pattern and clock (Settings > Date). Fields people type dates into
//! keep the ISO form, which they are read back in.

use std::sync::RwLock;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// 12 or 24 hours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Clock {
    #[default]
    System,
    H24,
    H12,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DateSettings {
    /// A pattern as "yyyy-MM-dd" (`from_pattern`); the system's when none.
    pub custom: Option<String>,
    pub clock: Clock,
}

/// strftime formats: the date, the time with seconds and without.
#[derive(Clone, Debug, PartialEq)]
struct Formats {
    date: String,
    seconds: String,
    time: String,
}

static FORMATS: RwLock<Option<Formats>> = RwLock::new(None);

/// What the front end's settings say; until called, the system's.
pub fn set(settings: &DateSettings) {
    *FORMATS.write().unwrap_or_else(|err| err.into_inner()) = Some(formats(settings));
}

fn current() -> Formats {
    if let Some(formats) = FORMATS.read().unwrap_or_else(|err| err.into_inner()).as_ref() {
        return formats.clone();
    }
    let formats = formats(&DateSettings::default());
    *FORMATS.write().unwrap_or_else(|err| err.into_inner()) = Some(formats.clone());
    formats
}

/// The file lists' Date column: date and time, no seconds.
pub fn date_time(time: SystemTime) -> String {
    let formats = current();
    local(time, &format!("{} {}", formats.date, formats.time))
}

/// With seconds, as the Properties dialog shows times.
pub fn date_time_seconds(time: SystemTime) -> String {
    let formats = current();
    local(time, &format!("{} {}", formats.date, formats.seconds))
}

/// The longest the Date column shows (28 December, 22:58), for its width.
pub fn widest() -> String {
    let formats = current();
    let time = chrono::NaiveDate::from_ymd_opt(2026, 12, 28).and_then(|day| day.and_hms_opt(22, 58, 0)).unwrap_or_default();
    render(&time, &format!("{} {}", formats.date, formats.time))
}

/// `time` as `settings` would show it, for the settings' preview.
pub fn preview(settings: &DateSettings, time: SystemTime) -> String {
    let formats = formats(settings);
    local(time, &format!("{} {}", formats.date, formats.time))
}

/// The system's date pattern, for the settings' page: "%Y. %m. %d.".
pub fn system_date(time: SystemTime) -> String {
    local(time, &system().0)
}

fn local(time: SystemTime, format: &str) -> String {
    let time = chrono::DateTime::<chrono::Local>::from(time);
    render(&time.naive_local(), format)
}

/// A pattern chrono cannot read shows as it was typed, not as a panic.
fn render(time: &chrono::NaiveDateTime, format: &str) -> String {
    use std::fmt::Write;
    let mut text = String::new();
    match write!(text, "{}", time.format(format)) {
        Ok(()) => text,
        Err(_) => format.to_owned(),
    }
}

fn formats(settings: &DateSettings) -> Formats {
    let (date, seconds) = system();
    let date = match settings.custom.as_deref().map(str::trim) {
        Some(pattern) if !pattern.is_empty() => from_pattern(pattern),
        _ => date,
    };
    let seconds = match settings.clock {
        Clock::System => seconds,
        Clock::H24 => "%H:%M:%S".to_owned(),
        Clock::H12 => "%-I:%M:%S %p".to_owned(),
    };
    Formats { date, time: without_seconds(&seconds), seconds }
}

/// ISO's, where the system tells none (the C locale, macOS for now).
const ISO: (&str, &str) = ("%Y-%m-%d", "%H:%M:%S");

/// The system's date and time (with seconds) formats.
fn system() -> (String, String) {
    platform().unwrap_or_else(|| (ISO.0.to_owned(), ISO.1.to_owned()))
}

/// The locale's LC_TIME, read without changing the process's locale.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn platform() -> Option<(String, String)> {
    let named = ["LC_ALL", "LC_TIME", "LANG"].iter().find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()));
    if named.as_deref().is_none_or(|name| name == "C" || name == "POSIX" || name.starts_with("C.")) {
        return None;
    }
    // SAFETY: a locale of its own, freed after; nl_langinfo_l's text is copied before.
    unsafe {
        let locale = libc::newlocale(libc::LC_TIME_MASK, c"".as_ptr(), std::ptr::null_mut());
        if locale.is_null() {
            return None;
        }
        let item = |item| {
            let text = libc::nl_langinfo_l(item, locale);
            (!text.is_null()).then(|| std::ffi::CStr::from_ptr(text).to_string_lossy().into_owned())
        };
        let date = item(libc::D_FMT).filter(|f| !f.is_empty());
        let time = item(libc::T_FMT).filter(|f| !f.is_empty()).or_else(|| item(libc::T_FMT_AMPM));
        libc::freelocale(locale);
        Some((date?, time.filter(|f| !f.is_empty())?))
    }
}

/// The region settings' short date and time.
#[cfg(windows)]
fn platform() -> Option<(String, String)> {
    use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_SSHORTDATE, LOCALE_STIMEFORMAT};
    let info = |kind| {
        let mut buffer = [0u16; 128];
        // SAFETY: the user's locale (no name), into a buffer of its length.
        let written = unsafe { GetLocaleInfoEx(windows::core::PCWSTR::null(), kind, Some(&mut buffer)) };
        let text = String::from_utf16_lossy(&buffer[..(written as usize).saturating_sub(1)]);
        (!text.is_empty()).then(|| from_pattern(&text))
    };
    Some((info(LOCALE_SSHORTDATE)?, info(LOCALE_STIMEFORMAT)?))
}

// shortcut: macOS shows ISO dates; read NSDateFormatter's if its users ask for theirs.
#[cfg(not(any(target_os = "linux", target_os = "freebsd", windows)))]
fn platform() -> Option<(String, String)> {
    None
}

/// Windows' and Qt's date patterns ("yyyy. MM. dd.", "h:mm tt") as strftime:
/// y year, M month (MMM its short name, MMMM its name), d day (ddd, dddd
/// the weekday's), H 24 hours, h 12, m minutes, s seconds, tt or AP AM/PM;
/// 'quoted' text as it is.
pub fn from_pattern(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        let run = chars[at..].iter().take_while(|&&next| next == c).count();
        let token = match (c, run) {
            ('\'', _) => {
                let end = chars[at + 1..].iter().position(|&next| next == '\'').map_or(chars.len(), |end| at + 1 + end);
                out.extend(chars[at + 1..end].iter().map(|&c| if c == '%' { "%%".into() } else { c.to_string() }));
                at = end + 1;
                continue;
            }
            ('y', 1 | 2) => "%y",
            ('y', _) => "%Y",
            ('M', 1) => "%-m",
            ('M', 2) => "%m",
            ('M', 3) => "%b",
            ('M', _) => "%B",
            ('d', 1) => "%-d",
            ('d', 2) => "%d",
            ('d', 3) => "%a",
            ('d', _) => "%A",
            ('H', 1) => "%-H",
            ('H', _) => "%H",
            ('h', 1) => "%-I",
            ('h', _) => "%I",
            ('m', 1) => "%-M",
            ('m', _) => "%M",
            ('s', 1) => "%-S",
            ('s', _) => "%S",
            ('t', _) | ('A', _) if c == 't' || chars.get(at + 1) == Some(&'P') => {
                at += if c == 't' { run } else { 2 };
                out += "%p";
                continue;
            }
            ('a', _) if chars.get(at + 1) == Some(&'p') => {
                at += 2;
                out += "%P";
                continue;
            }
            _ => {
                match c {
                    '%' => out += "%%",
                    _ => out.push(c),
                }
                at += 1;
                continue;
            }
        };
        out += token;
        at += run;
    }
    out
}

/// A time format without its seconds, and the separator before them.
fn without_seconds(format: &str) -> String {
    let expanded = format.replace("%T", "%H:%M:%S").replace("%r", "%I:%M:%S %p").replace("%X", "%H:%M:%S");
    let mut out = expanded.clone();
    for seconds in ["%-S", "%S"] {
        if let Some(at) = out.find(seconds) {
            let before = out[..at].chars().next_back().filter(|c| matches!(c, ':' | '.'));
            let start = at - before.map_or(0, char::len_utf8);
            out.replace_range(start..at + seconds.len(), "");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(format: &str) -> String {
        let time = chrono::NaiveDate::from_ymd_opt(2026, 3, 7).unwrap().and_hms_opt(14, 5, 9).unwrap();
        render(&time, format)
    }

    #[test]
    fn reads_windows_and_qt_patterns() {
        assert_eq!(from_pattern("yyyy-MM-dd"), "%Y-%m-%d");
        assert_eq!(at(&from_pattern("yyyy. MM. dd.")), "2026. 03. 07.");
        assert_eq!(at(&from_pattern("d.M.yy")), "7.3.26");
        assert_eq!(at(&from_pattern("dddd, MMMM d")), "Saturday, March 7");
        assert_eq!(at(&from_pattern("h:mm tt")), "2:05 PM");
        assert_eq!(at(&from_pattern("HH:mm:ss")), "14:05:09");
        assert_eq!(at(&from_pattern("hh:mm AP")), "02:05 PM");
        assert_eq!(at(&from_pattern("yyyy 'at' HH'h' 100%")), "2026 at 14h 100%");
    }

    #[test]
    fn drops_seconds() {
        assert_eq!(without_seconds("%H:%M:%S"), "%H:%M");
        assert_eq!(without_seconds("%r"), "%I:%M %p");
        assert_eq!(without_seconds("%-I:%M:%S %p"), "%-I:%M %p");
        assert_eq!(without_seconds("%H.%M.%S"), "%H.%M");
        assert_eq!(without_seconds("%H:%M"), "%H:%M");
    }

    #[test]
    fn clocks_and_patterns() {
        let shown = |custom: Option<&str>, clock| {
            let formats = formats(&DateSettings { custom: custom.map(Into::into), clock });
            at(&format!("{} {}", formats.date, formats.time))
        };
        assert_eq!(shown(Some("dd/MM/yyyy"), Clock::H24), "07/03/2026 14:05");
        assert_eq!(shown(Some("yyyy-MM-dd"), Clock::H12), "2026-03-07 2:05 PM");
        // An empty pattern is the system's date.
        assert!(shown(Some("  "), Clock::H24).ends_with(" 14:05"));
    }
}
