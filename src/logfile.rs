//! Release log: `logs/overlay_input-YYYY-MM-DD.log`, one file per local day, appended; old days deleted.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};

const PREFIX: &str = "overlay_input-";
const SUFFIX: &str = ".log";
/// Daily files kept; older ones are deleted at start.
pub const KEEP_DAYS: usize = 30;

pub struct DailyLog {
    dir: PathBuf,
    /// Today's date and its open file.
    file: Mutex<Option<(String, File)>>,
}

impl DailyLog {
    /// Creates `dir` and deletes all but the newest `KEEP_DAYS` log files.
    pub fn new(dir: PathBuf) -> io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        prune(&dir, KEEP_DAYS);
        Ok(Self {
            dir,
            file: Mutex::new(None),
        })
    }

    fn write_event(&self, buf: &[u8]) -> io::Result<()> {
        let today = date(&local_now());
        let mut slot = self.file.lock().unwrap_or_else(|e| e.into_inner());
        // Past midnight: switch to the new day's file.
        if slot.as_ref().is_none_or(|(day, _)| *day != today) {
            let path = self.dir.join(format!("{PREFIX}{today}{SUFFIX}"));
            let file = OpenOptions::new().create(true).append(true).open(path)?;
            *slot = Some((today, file));
        }
        match slot.as_mut() {
            Some((_, file)) => file.write_all(buf),
            None => Ok(()),
        }
    }
}

impl<'a> MakeWriter<'a> for DailyLog {
    type Writer = DailyWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        DailyWriter(self)
    }
}

pub struct DailyWriter<'a>(&'a DailyLog);

impl Write for DailyWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_event(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Log timestamps in local time with the UTC offset, e.g. `2026-10-14 21:05:09.123 +03:00`.
pub struct LocalTime;

impl FormatTime for LocalTime {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        let t = local_now();
        let utc = unsafe { GetSystemTime() };
        write!(
            w,
            "{} {:02}:{:02}:{:02}.{:03} {}",
            date(&t),
            t.wHour,
            t.wMinute,
            t.wSecond,
            t.wMilliseconds,
            utc_offset(&t, &utc)
        )
    }
}

/// `local - utc` as `+03:00` / `-04:30`, rounded to 15 minutes (every time zone is a multiple of that).
fn utc_offset(local: &SYSTEMTIME, utc: &SYSTEMTIME) -> String {
    let minutes = |t: &SYSTEMTIME| i32::from(t.wHour) * 60 + i32::from(t.wMinute);
    let day = |t: &SYSTEMTIME| (t.wYear, t.wMonth, t.wDay);
    // The two dates differ by at most one day.
    let day_shift = match day(local).cmp(&day(utc)) {
        std::cmp::Ordering::Greater => 24 * 60,
        std::cmp::Ordering::Less => -24 * 60,
        std::cmp::Ordering::Equal => 0,
    };
    let raw = minutes(local) - minutes(utc) + day_shift;
    let offset = ((raw as f32 / 15.0).round() as i32) * 15;
    let sign = if offset < 0 { '-' } else { '+' };
    format!("{sign}{:02}:{:02}", offset.abs() / 60, offset.abs() % 60)
}

fn local_now() -> SYSTEMTIME {
    unsafe { GetLocalTime() }
}

fn date(t: &SYSTEMTIME) -> String {
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

/// Deletes the oldest daily files beyond `keep` (names sort by date).
fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(PREFIX) && n.ends_with(SUFFIX))
        })
        .collect();
    logs.sort();
    let excess = logs.len().saturating_sub(keep);
    for old in &logs[..excess] {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(y: u16, mo: u16, d: u16, h: u16, mi: u16) -> SYSTEMTIME {
        SYSTEMTIME {
            wYear: y,
            wMonth: mo,
            wDay: d,
            wHour: h,
            wMinute: mi,
            ..Default::default()
        }
    }

    #[test]
    fn utc_offsets() {
        let utc = at(2026, 10, 14, 22, 30);
        assert_eq!(
            utc_offset(&at(2026, 10, 15, 1, 30), &utc),
            "+03:00",
            "next day"
        );
        assert_eq!(utc_offset(&at(2026, 10, 14, 18, 0), &utc), "-04:30");
        assert_eq!(utc_offset(&utc, &utc), "+00:00");
        // Year boundary, and a minute ticking over between the two clock reads.
        assert_eq!(
            utc_offset(&at(2027, 1, 1, 3, 1), &at(2026, 12, 31, 23, 0)),
            "+04:00"
        );
    }

    #[test]
    fn writes_todays_file_and_prunes_old_days() {
        let dir = std::env::temp_dir().join(format!("overlay_input_logs_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for day in 1..=(KEEP_DAYS + 2) {
            std::fs::write(dir.join(format!("{PREFIX}2020-01-{day:02}{SUFFIX}")), "").unwrap();
        }
        std::fs::write(dir.join("notes.txt"), "not a log").unwrap();

        let log = DailyLog::new(dir.clone()).unwrap();
        log.make_writer().write_all(b"first\n").unwrap();
        log.make_writer().write_all(b"second\n").unwrap();

        let today = dir.join(format!("{PREFIX}{}{SUFFIX}", date(&local_now())));
        assert_eq!(std::fs::read_to_string(today).unwrap(), "first\nsecond\n");
        assert!(
            !dir.join(format!("{PREFIX}2020-01-01{SUFFIX}")).exists(),
            "oldest pruned"
        );
        assert!(!dir.join(format!("{PREFIX}2020-01-02{SUFFIX}")).exists());
        assert!(dir.join(format!("{PREFIX}2020-01-03{SUFFIX}")).exists());
        assert!(dir.join("notes.txt").exists(), "other files untouched");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
