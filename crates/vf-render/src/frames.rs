//! Video frames for the preview: one cache of decoded pictures shared by every
//! clip, bounded by a share of the machine's memory, and one worker per clip
//! that decodes the frame under the playhead first and then the frames ahead
//! of it (a second while playing, a few frames while paused), so playback
//! shows every frame on time and going back over watched footage is instant.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};

use vf_media::video::VideoDecoder;
use vf_media::RgbaImage;

/// Decoded ahead of the playhead while playing.
const PLAY_AHEAD: f64 = 1.0;
/// Decoded ahead while paused (stepping frame by frame).
const PAUSE_AHEAD: f64 = 0.2;

/// A file decoded at one size.
pub(crate) type SourceKey = (PathBuf, u32, u32);

struct Entry {
    /// Media time range during which this frame is on screen.
    from: f64,
    until: f64,
    image: Arc<RgbaImage>,
    used: u64,
    id: u64,
}

/// Frame timestamps as map keys (microseconds).
fn stamp(time: f64) -> i64 {
    (time * 1e6).round() as i64
}

pub(crate) struct FrameCache {
    /// Frames of each source by timestamp.
    sources: HashMap<SourceKey, BTreeMap<i64, Entry>>,
    bytes: usize,
    budget: usize,
    tick: u64,
    next_id: u64,
}

impl FrameCache {
    pub(crate) fn new(budget: usize) -> Self {
        FrameCache { sources: HashMap::new(), bytes: 0, budget, tick: 0, next_id: 1 }
    }

    /// The cache every renderer shares, so the budget holds for the whole app.
    pub(crate) fn shared() -> Arc<Mutex<FrameCache>> {
        static SHARED: OnceLock<Arc<Mutex<FrameCache>>> = OnceLock::new();
        SHARED.get_or_init(|| Arc::new(Mutex::new(FrameCache::new(budget())))).clone()
    }

    fn covering(&self, key: &SourceKey, time: f64) -> Option<i64> {
        let frames = self.sources.get(key)?;
        let at = stamp(time);
        // The frames around `time` (two before, as the key rounds; one after,
        // which may be stretched back to cover the start).
        let mut candidates = frames.range(..=at).rev().take(2).chain(frames.range(at + 1..).take(1));
        candidates.find(|(_, e)| e.from <= time && time < e.until).map(|(k, _)| *k)
    }

    /// The frame on screen at `time`, with an id that changes with the picture.
    pub(crate) fn get(&mut self, key: &SourceKey, time: f64) -> Option<(u64, Arc<RgbaImage>)> {
        let found = self.covering(key, time)?;
        self.tick += 1;
        let entry = self.sources.get_mut(key)?.get_mut(&found)?;
        entry.used = self.tick;
        Some((entry.id, entry.image.clone()))
    }

    /// First moment in `from..to` without a cached frame.
    pub(crate) fn first_gap(&self, key: &SourceKey, from: f64, to: f64) -> Option<f64> {
        let mut time = from;
        while time < to {
            match self.covering(key, time) {
                Some(found) => time = self.sources[key][&found].until,
                None => return Some(time),
            }
        }
        None
    }

    /// Adds the frame with timestamp `time`, on screen over `from..until`.
    pub(crate) fn insert(&mut self, key: &SourceKey, from: f64, time: f64, until: f64, image: RgbaImage) {
        self.tick += 1;
        let frames = self.sources.entry(key.clone()).or_default();
        let (tick, id) = (self.tick, self.next_id);
        match frames.get_mut(&stamp(time)) {
            Some(entry) => {
                entry.from = entry.from.min(from);
                entry.until = entry.until.max(until);
                entry.used = tick;
                return;
            }
            None => {
                self.bytes += image.data.len();
                self.next_id += 1;
                frames.insert(stamp(time), Entry { from: from.min(time), until, image: Arc::new(image), used: tick, id });
            }
        }
        if self.bytes > self.budget {
            self.evict();
        }
    }

    /// Drops the least recently used frames down to 90 % of the budget.
    fn evict(&mut self) {
        let mut all: Vec<(u64, SourceKey, i64, usize)> = Vec::new();
        for (key, frames) in &self.sources {
            for (time, entry) in frames {
                all.push((entry.used, key.clone(), *time, entry.image.data.len()));
            }
        }
        all.sort_unstable_by_key(|(used, ..)| *used);
        let goal = self.budget / 10 * 9;
        for (_, key, time, size) in all {
            if self.bytes <= goal {
                break;
            }
            if let Some(frames) = self.sources.get_mut(&key) {
                frames.remove(&time);
                self.bytes -= size;
            }
        }
        self.sources.retain(|_, frames| !frames.is_empty());
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn clear(&mut self) {
        self.sources.clear();
        self.bytes = 0;
    }
}

/// A thirty-second of the memory, between 128 and 512 MB (256 MB when
/// unknown): a second ahead of the playhead is all playback needs; the rest
/// keeps a few seconds behind it instant to scrub back over.
fn budget() -> usize {
    const MB: u64 = 1 << 20;
    let total = total_memory().unwrap_or(8192 * MB);
    (total / 32).clamp(128 * MB, 512 * MB) as usize
}

#[cfg(windows)]
fn total_memory() -> Option<u64> {
    #[repr(C)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total_phys: u64,
        avail_phys: u64,
        total_page_file: u64,
        avail_page_file: u64,
        total_virtual: u64,
        avail_virtual: u64,
        avail_extended_virtual: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(buffer: *mut MemoryStatus) -> i32;
    }
    let mut status = MemoryStatus {
        length: std::mem::size_of::<MemoryStatus>() as u32,
        load: 0,
        total_phys: 0,
        avail_phys: 0,
        total_page_file: 0,
        avail_page_file: 0,
        total_virtual: 0,
        avail_virtual: 0,
        avail_extended_virtual: 0,
    };
    // SAFETY: the struct has the layout of MEMORYSTATUSEX with its length set.
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status.total_phys)
}

#[cfg(target_os = "linux")]
fn total_memory() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = info.lines().find(|l| l.starts_with("MemTotal:"))?;
    line.split_whitespace().nth(1)?.parse::<u64>().ok().map(|kb| kb * 1024)
}

#[cfg(target_os = "macos")]
fn total_memory() -> Option<u64> {
    extern "C" {
        fn sysctlbyname(name: *const std::ffi::c_char, old: *mut std::ffi::c_void, old_len: *mut usize, new: *mut std::ffi::c_void, new_len: usize) -> i32;
    }
    let mut bytes: u64 = 0;
    let mut length = std::mem::size_of::<u64>();
    // SAFETY: a NUL-terminated name and an out buffer of the stated length.
    let status = unsafe { sysctlbyname(c"hw.memsize".as_ptr(), (&mut bytes as *mut u64).cast(), &mut length, std::ptr::null_mut(), 0) };
    (status == 0 && bytes > 0).then_some(bytes)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn total_memory() -> Option<u64> {
    None
}

#[derive(Clone, Copy)]
struct Cursor {
    time: f64,
    playing: bool,
}

/// The worker decoding one clip's file into the cache.
pub(crate) struct FrameSource {
    requests: Sender<Cursor>,
    failed: Arc<AtomicBool>,
    last: Option<(f64, bool)>,
}

impl FrameSource {
    pub(crate) fn spawn(key: SourceKey, cache: Arc<Mutex<FrameCache>>) -> Self {
        let (requests, receiver) = channel::<Cursor>();
        let failed = Arc::new(AtomicBool::new(false));
        let flag = failed.clone();
        std::thread::Builder::new()
            .name("video-decoder".into())
            .spawn(move || {
                let mut decoder = match VideoDecoder::open(&key.0, key.1, key.2) {
                    Ok(decoder) => decoder,
                    Err(error) => {
                        log::warn!("{error}");
                        flag.store(true, Ordering::Release);
                        return;
                    }
                };
                let Ok(mut cursor) = receiver.recv() else { return };
                loop {
                    // Only the newest position matters.
                    loop {
                        match receiver.try_recv() {
                            Ok(newer) => cursor = newer,
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    let ahead = if cursor.playing { PLAY_AHEAD } else { PAUSE_AHEAD };
                    let gap = cache.lock().ok().and_then(|c| c.first_gap(&key, cursor.time, cursor.time + ahead));
                    let decoded = match gap {
                        Some(time) => decode(&mut decoder, &key, time, &cache),
                        None => false,
                    };
                    if !decoded {
                        // Everything wanted is there (or cannot be decoded): wait for a move.
                        match receiver.recv() {
                            Ok(newer) => cursor = newer,
                            Err(_) => return,
                        }
                    }
                }
            })
            .expect("video decoder thread");
        FrameSource { requests, failed, last: None }
    }

    /// Tells the worker where the playhead is (in media time).
    pub(crate) fn request(&mut self, time: f64, playing: bool) {
        if self.last != Some((time, playing)) {
            self.last = Some((time, playing));
            let _ = self.requests.send(Cursor { time, playing });
        }
    }

    pub(crate) fn failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
}

/// Decodes the frame on screen at `time` into the cache; false when nothing came out.
fn decode(decoder: &mut VideoDecoder, key: &SourceKey, time: f64, cache: &Mutex<FrameCache>) -> bool {
    let image = match decoder.frame_at(time) {
        Ok(Some(image)) => image,
        Ok(None) => return false,
        Err(error) => {
            log::warn!("{}: {error}", key.0.display());
            return false;
        }
    };
    let Some(shown) = decoder.current_time() else { return false };
    // After the last frame it stays on screen.
    let until = decoder.next_time().ok().flatten().unwrap_or(f64::INFINITY);
    if until <= time {
        return false;
    }
    if let Ok(mut cache) = cache.lock() {
        cache.insert(key, time.min(shown), shown, until, image);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(value: u8, size: usize) -> RgbaImage {
        RgbaImage { width: 1, height: 1, data: vec![value; size] }
    }

    fn key(name: &str) -> SourceKey {
        (PathBuf::from(name), 100, 100)
    }

    #[test]
    fn finds_the_frame_on_screen_and_the_first_gap() {
        let mut cache = FrameCache::new(1 << 20);
        let a = key("a.mp4");
        // The first frame starts at 0.02 but covers the start.
        cache.insert(&a, 0.0, 0.02, 0.06, picture(1, 4));
        cache.insert(&a, 0.06, 0.06, 0.1, picture(2, 4));
        cache.insert(&a, 0.2, 0.2, 0.24, picture(3, 4));
        assert_eq!(cache.get(&a, 0.0).unwrap().1.data[0], 1);
        assert_eq!(cache.get(&a, 0.059).unwrap().1.data[0], 1);
        assert_eq!(cache.get(&a, 0.06).unwrap().1.data[0], 2);
        // Just before a frame starts (rounds to its key): still the previous one.
        assert_eq!(cache.get(&a, 0.0599999999999).unwrap().1.data[0], 1);
        assert!(cache.get(&a, 0.15).is_none());
        assert!(cache.get(&key("b.mp4"), 0.0).is_none());
        assert_eq!(cache.first_gap(&a, 0.0, 0.3), Some(0.1));
        assert_eq!(cache.first_gap(&a, 0.0, 0.1), None);
        assert_eq!(cache.first_gap(&a, 0.2, 0.24), None);
        assert_eq!(cache.first_gap(&a, 0.21, 0.5), Some(0.24));
        // Same frame again: no second copy.
        cache.insert(&a, 0.06, 0.06, 0.1, picture(2, 4));
        assert_eq!(cache.bytes(), 12);
        // The last frame of a file stays on screen.
        cache.insert(&a, 0.3, 0.3, f64::INFINITY, picture(4, 4));
        assert_eq!(cache.get(&a, 99.0).unwrap().1.data[0], 4);
        assert_eq!(cache.first_gap(&a, 0.3, 99.0), None);
    }

    #[test]
    fn stays_within_the_budget_dropping_the_oldest_frames() {
        let mut cache = FrameCache::new(1000);
        let a = key("a.mp4");
        for i in 0..30 {
            let t = i as f64 * 0.1;
            cache.insert(&a, t, t, t + 0.1, picture(i as u8, 100));
            // Frame 0 stays in use.
            assert!(cache.get(&a, 0.05).is_some(), "frame 0 kept at {i}");
        }
        assert!(cache.bytes() <= 1000, "{}", cache.bytes());
        assert!(cache.get(&a, 2.95).is_some(), "newest kept");
        assert!(cache.get(&a, 1.05).is_none(), "old frames dropped");
    }
}
