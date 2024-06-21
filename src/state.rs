use std::{
    collections::BTreeMap,
    ptr::addr_of,
    sync::atomic::{AtomicUsize, Ordering},
};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::input::data::{FileDescriptor, FileName};

/// A global pointer address of the download tracker.
static TRACKER: AtomicUsize = AtomicUsize::new(0);
pub unsafe fn set_tracker_global(tracker: &DownloadTracker) {
    TRACKER
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |it| {
            if it != 0 {
                panic!("download tracker already set");
            }
            Some(addr_of!(*tracker) as usize)
        })
        .expect("can't set download tracker global");
}
pub unsafe fn get_tracker_global() -> Option<&'static DownloadTracker> {
    let addr = TRACKER.load(Ordering::SeqCst);
    if addr == 0 {
        return None;
    }
    Some((addr as *const DownloadTracker).as_ref().unwrap_unchecked())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTracker {
    start_time: DateTime<Utc>,
    file_names: Vec<FileName>,
    file_sizes: Vec<usize>,
    total_size: usize,
    current_file: usize,
    passive_offset: usize,
    current_offset: usize,
}

impl DownloadTracker {
    pub fn new(files: &BTreeMap<FileName, FileDescriptor>) -> DownloadTracker {
        let total_size: usize = files.values().map(|it| it.size).sum();
        DownloadTracker {
            start_time: Utc::now(),
            file_names: files.keys().cloned().collect(),
            file_sizes: files.values().map(|it| it.size).collect(),
            total_size,
            current_file: 0,
            passive_offset: 0,
            current_offset: 0,
        }
    }

    pub fn set_current_position(&mut self, buffer_position: usize) {
        self.current_offset = buffer_position;
    }

    pub fn total_size(&self) -> usize {
        self.total_size
    }

    pub fn downloaded(&self) -> usize {
        self.passive_offset + self.current_offset
    }

    pub fn download_percent(&self) -> f32 {
        self.downloaded() as f32 / self.total_size as f32
    }

    pub fn elapsed_time(&self) -> Duration {
        Utc::now() - self.start_time
    }

    pub fn advance_file(&mut self) {
        self.passive_offset += self.file_sizes[self.current_file];
        self.current_offset = 0;
        self.current_file += 1;
    }

    pub fn current_file(&self) -> Option<&FileName> {
        self.file_names.get(self.current_file)
    }

    pub fn eta(&self) -> usize {
        (self.elapsed_time().num_seconds() as f64 / self.download_percent() as f64
            * (1. - self.download_percent()) as f64
            + 1.) as usize
    }
}
