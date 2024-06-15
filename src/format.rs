//! Logging and formatting utilites.

use std::{
    io::Write as _,
    sync::atomic::{AtomicBool, Ordering},
};

use env_logger::fmt::Formatter;
use itertools::Itertools;
use log::Record;

use crate::state::{get_tracker_global, DownloadTracker};

const ANSI_LINE_UP: &[u8] = b"\x1b[1A";
const ANSI_LINE_START: &[u8] = b"\x1b[9999D";
const ANSI_CLEAR_LINE: &str = "\x1b[0K";

const ANSI_LIME: &[u8] = b"\x1b[92m";
const ANSI_RESET: &[u8] = b"\x1b[39m";

pub fn left_pad(text: impl AsRef<str>, size: usize) -> String {
    let text = text.as_ref();
    " ".repeat(size - text.len()) + text
}

pub fn format_bytes(bytes: usize) -> String {
    let kb = bytes as f32 / 1024.;
    if kb < 1. {
        format!("{} B", bytes);
    }
    let mb = kb / 1024.;
    if kb < 100. {
        format!("{:.2} KiB", kb);
    }
    let gb = mb / 1024.;
    if mb < 100. {
        format!("{:.2} MiB", mb);
    }

    format!("{:.2} GiB", gb)
}

pub fn format_seconds(seconds: usize) -> String {
    if seconds <= 60 {
        return format!("{}s", seconds);
    }
    let minutes = seconds / 60;
    if minutes <= 60 {
        let seconds = seconds % 60;
        if seconds > 2 {
            return format!("{}min {}s", minutes, seconds);
        }

        return format!("{}min", minutes);
    }
    let hours = minutes / 60;
    if hours <= 24 {
        let rem_m = minutes % 60;
        if rem_m > 5 {
            return format!("{}h {}min", hours, rem_m);
        }
        return format!("{}h", hours);
    }
    format!("{}d {}h", hours / 24, hours % 24)
}

pub fn percent_pad(percent: f32, precision: usize) -> String {
    left_pad(
        format!("{:3.precision$}%", percent * 100., precision = precision),
        5 + precision,
    )
}

fn format_bar(out: &mut Vec<u8>, percent: f32, max_width: usize) -> std::io::Result<()> {
    let fill_width = (max_width as f32 * percent) as usize;
    let key = (((max_width as f32 * percent) - fill_width as f32) * 8.0).round() as u32;

    let (partial, rest) = if key > 0 {
        let c = char::from_u32(0x2590 - key).unwrap(); // Unicode from 2588 (full) to 258F (1/8)
        (c, max_width - fill_width - 1)
    } else {
        (' ', max_width - fill_width)
    };

    out.write_all(ANSI_LIME)?;
    for _ in 0..fill_width {
        out.write_all("█".as_bytes())?;
    }

    if key > 0 && fill_width < max_width {
        out.write_all(partial.to_string().as_bytes())?;
    }
    out.write_all(ANSI_RESET)?;

    for _ in 0..rest {
        out.write_all(" ".as_bytes())?;
    }

    Ok(())
}

fn print_progress_bar(tracker: &DownloadTracker) -> std::io::Result<Vec<u8>> {
    let current_file = match tracker.current_file() {
        Some(it) => it,
        None => return Ok(b"\n\n".to_vec()),
    };

    let total_width = termsize::get()
        .map(|it| it.cols.min(120) as usize)
        .unwrap_or(40);
    let percent = tracker.download_percent();
    let left_display = if total_width > 40 {
        format!("[{}|", left_pad(format_bytes(tracker.downloaded()), 9))
    } else {
        "[".to_string()
    };
    let right_display = if total_width > 60 {
        format!(
            "|{}] {} ETA: {}",
            left_pad(format_bytes(tracker.total_size()), 9),
            percent_pad(percent, 2),
            format_seconds(tracker.eta())
        )
    } else if total_width > 40 {
        format!(
            "] {} ETA: {}",
            percent_pad(percent, 2),
            format_seconds(tracker.eta())
        )
    } else {
        format!(
            "] {} ETA: {}",
            percent_pad(percent, 0),
            format_seconds(tracker.eta())
        )
    };
    let bar_width = total_width - 45;

    let mut out = Vec::with_capacity(total_width + 128);
    out.write_all(ANSI_CLEAR_LINE.as_bytes())?;
    out.write_all(ANSI_LINE_START)?;
    out.write_all(ANSI_RESET)?;
    out.write_all(left_display.as_bytes())?;
    format_bar(&mut out, percent, bar_width)?;
    out.write_all(right_display.as_bytes())?;
    out.write_all(b"\n")?;
    out.write_all(b" > ")?;
    out.write_all(current_file.as_ref().as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()?;

    Ok(out)
}

pub fn format(buf: &mut Formatter, record: &Record) -> std::io::Result<()> {
    static HAS_BAR: AtomicBool = AtomicBool::new(false);
    let tracker = unsafe { get_tracker_global() };

    if HAS_BAR.load(Ordering::Acquire) {
        buf.write_all(ANSI_LINE_UP)?;
        buf.write_all(ANSI_CLEAR_LINE.as_bytes())?;
        buf.write_all(ANSI_LINE_UP)?;
        buf.write_all(ANSI_CLEAR_LINE.as_bytes())?;
    }

    if let Some(tracker) = tracker {
        let message = record.args().to_string().split('\n').join("\n\x1b[0K");
        let progress = print_progress_bar(tracker)?;

        writeln!(buf, "[{}]: {}\x1b[0K", record.level(), message)?;
        buf.write_all(&progress)?;
        HAS_BAR.store(true, Ordering::Release);
    } else {
        writeln!(buf, "[{}]: {}", record.level(), record.args())?;
        HAS_BAR.store(false, Ordering::Release);
    }

    Ok(())
}
