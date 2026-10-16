//! Frame-accurate video decoding: "the frame shown at time t", fast when time
//! moves forward (playback, export) and correct after any jump (scrubbing).
//!
//! Heavy formats decode on the GPU when the system has a decoder for them
//! (D3D11VA on Windows, VideoToolbox on macOS): HEVC, AV1, VP9, and H.264
//! above 1080p or deeper than 8 bits. Plain H.264 stays on the CPU, where it
//! is faster. When the GPU fails, decoding continues on the CPU from the
//! frame that failed.

use std::path::Path;
use std::sync::OnceLock;

use ffmpeg::software::scaling::{self, flag::Flags};
use ffmpeg_next as ffmpeg;
use vf_core::time::Rational;
use vf_core::{Error, Result};

use crate::{av, fit_size, RgbaImage};

/// Forward distance up to which decoding on is cheaper than seeking.
const FORWARD_WINDOW: f64 = 2.5;
/// GPU errors in a row after which decoding moves to the CPU.
const HARDWARE_FAILURES: u32 = 3;

struct Decoded {
    pts: i64,
    time: f64,
    frame: ffmpeg::frame::Video,
}

/// The system's video decoding device, created once and shared by every decoder.
struct Device(*mut ffmpeg::ffi::AVBufferRef);

// libavutil hardware device contexts lock internally.
unsafe impl Send for Device {}
unsafe impl Sync for Device {}

fn device() -> Option<&'static Device> {
    static DEVICE: OnceLock<Option<Device>> = OnceLock::new();
    DEVICE
        .get_or_init(|| {
            use ffmpeg::ffi::AVHWDeviceType::*;
            let kind = if cfg!(windows) {
                AV_HWDEVICE_TYPE_D3D11VA
            } else if cfg!(target_os = "macos") {
                AV_HWDEVICE_TYPE_VIDEOTOOLBOX
            } else if std::env::var("VF_VAAPI").as_deref() == Ok("1") {
                // Opt-in: VAAPI has hung some Intel drivers.
                AV_HWDEVICE_TYPE_VAAPI
            } else {
                return None;
            };
            let mut context = std::ptr::null_mut();
            // SAFETY: plain FFI call; the context lives for the whole process.
            let status = unsafe { ffmpeg::ffi::av_hwdevice_ctx_create(&mut context, kind, std::ptr::null(), std::ptr::null_mut(), 0) };
            if status < 0 || context.is_null() {
                log::info!("no hardware video decoding ({kind:?}: error {status})");
                return None;
            }
            log::info!("hardware video decoding: {kind:?}");
            Some(Device(context))
        })
        .as_ref()
}

/// Whether this stream decodes better on the GPU. `VF_HWDECODE=off` turns
/// hardware decoding off, `VF_HWDECODE=always` also uses it for small H.264.
fn hardware_wins(parameters: &ffmpeg::codec::Parameters) -> bool {
    use ffmpeg::codec::Id;
    let setting = std::env::var("VF_HWDECODE").unwrap_or_default();
    if setting == "off" {
        return false;
    }
    // SAFETY: reading plain fields of valid codec parameters.
    let (width, height, format) = unsafe {
        let raw = &*parameters.as_ptr();
        (raw.width as i64, raw.height as i64, raw.format)
    };
    // SAFETY: the descriptor table is static; unknown formats give null.
    let deep = unsafe {
        let descriptor = ffmpeg::ffi::av_pix_fmt_desc_get(std::mem::transmute::<i32, ffmpeg::ffi::AVPixelFormat>(format));
        !descriptor.is_null() && (*descriptor).comp[0].depth > 8
    };
    match parameters.id() {
        Id::HEVC | Id::AV1 | Id::VP9 => true,
        Id::H264 => setting == "always" || deep || width * height > 1920 * 1088,
        _ => false,
    }
}

/// Threads per software decoder: half the cores, two to eight (4K HEVC
/// still decodes faster than real time; more only cost memory).
fn decode_threads() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    (cores / 2).clamp(2, 8)
}

/// A decoder for the stream, on the GPU when `hardware` and it pays off.
fn open_decoder(parameters: ffmpeg::codec::Parameters, hardware: bool, keyframes: bool) -> Result<(ffmpeg::decoder::Video, bool)> {
    let device = if hardware && hardware_wins(&parameters) { device() } else { None };
    let mut context = ffmpeg::codec::context::Context::from_parameters(parameters).map_err(av)?;
    match device {
        // SAFETY: the context is not opened yet; it takes its own reference to the device.
        Some(device) => unsafe { (*context.as_mut_ptr()).hw_device_ctx = ffmpeg::ffi::av_buffer_ref(device.0) },
        // Frame threads hold back output until each has a frame: right for
        // playback, slow for one picture after each seek.
        None if keyframes => context.set_threading(ffmpeg::threading::Config { kind: ffmpeg::threading::Type::Slice, count: 0 }),
        // Each frame thread holds a picture in flight: a few are plenty.
        None => context.set_threading(ffmpeg::threading::Config { kind: ffmpeg::threading::Type::Frame, count: decode_threads() }),
    }
    let mut decoder = context.decoder().video().map_err(av)?;
    if keyframes {
        decoder.skip_frame(ffmpeg::Discard::NonKey);
    }
    Ok((decoder, device.is_some()))
}

pub struct VideoDecoder {
    input: ffmpeg::format::context::Input,
    stream: usize,
    decoder: ffmpeg::decoder::Video,
    time_base: Rational,
    /// First timestamp of the stream (time 0).
    start: i64,
    rotation: i32,
    /// Output size after rotation.
    width: u32,
    height: u32,
    scaler: Option<(scaling::Context, ffmpeg::format::Pixel, u32, u32)>,
    current: Option<Decoded>,
    next: Option<Decoded>,
    eof: bool,
    /// Average frame length; used to pick the frame on screen.
    frame_length: f64,
    /// Decoding on the GPU (until it fails).
    hardware: bool,
    failures: u32,
    /// Frames decoded since the last seek.
    produced: u32,
    keyframes: bool,
    /// Last keyframe returned by `keyframe_at`.
    last_key: Option<(i64, RgbaImage)>,
}

// The decoder is only ever used from one thread at a time.
unsafe impl Send for VideoDecoder {}

impl VideoDecoder {
    /// Opens the first video stream; frames come out at most `max_w`×`max_h`.
    pub fn open(path: &Path, max_w: u32, max_h: u32) -> Result<Self> {
        Self::open_with(path, max_w, max_h, true, false)
    }

    /// A decoder that only decodes keyframes (see [`VideoDecoder::keyframe_at`]):
    /// a picture costs one decoded frame, for filmstrips and thumbnails.
    pub fn open_keyframes(path: &Path, max_w: u32, max_h: u32) -> Result<Self> {
        Self::open_with(path, max_w, max_h, false, true)
    }

    fn open_with(path: &Path, max_w: u32, max_h: u32, hardware: bool, keyframes: bool) -> Result<Self> {
        crate::init();
        let input = ffmpeg::format::input(&path).map_err(|error| Error::msg(format!("{} cannot be opened: {error}", path.display())))?;
        let stream = input
            .streams()
            .find(|s| s.parameters().medium() == ffmpeg::media::Type::Video && !s.disposition().contains(ffmpeg::format::stream::Disposition::ATTACHED_PIC))
            .or_else(|| input.streams().best(ffmpeg::media::Type::Video))
            .ok_or_else(|| Error::msg("This file has no video."))?;
        let index = stream.index();
        let base = stream.time_base();
        let time_base = Rational::new(base.numerator() as i64, base.denominator() as i64);
        let start = if stream.start_time() != ffmpeg::ffi::AV_NOPTS_VALUE { stream.start_time() } else { 0 };
        let rotation = crate::probe::stream_rotation(&stream);
        let fps = stream.avg_frame_rate();
        let frame_length = if fps.numerator() > 0 { fps.denominator() as f64 / fps.numerator() as f64 } else { 1.0 / 30.0 };
        let (decoder, hardware) = match open_decoder(stream.parameters(), hardware, keyframes) {
            Ok(opened) => opened,
            Err(error) if hardware => {
                log::warn!("{}: no GPU decoding ({error}); using the CPU", path.display());
                open_decoder(stream.parameters(), false, keyframes)?
            }
            Err(error) => return Err(error),
        };
        let (sw, sh) = if rotation % 180 == 90 { (decoder.height(), decoder.width()) } else { (decoder.width(), decoder.height()) };
        let (width, height) = fit_size(sw, sh, max_w, max_h);
        Ok(VideoDecoder {
            input,
            stream: index,
            decoder,
            time_base,
            start,
            rotation,
            width,
            height,
            scaler: None,
            current: None,
            next: None,
            eof: false,
            frame_length: frame_length.clamp(1.0 / 240.0, 1.0),
            hardware,
            failures: 0,
            produced: 0,
            keyframes,
            last_key: None,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Whether frames come from the GPU decoder.
    pub fn is_hardware(&self) -> bool {
        self.hardware
    }

    /// Continues on the CPU from `time` after the GPU decoder failed.
    fn fall_back(&mut self, time: f64, why: &Error) -> Result<()> {
        log::warn!("GPU video decoding failed ({why}); continuing on the CPU");
        let parameters = self.input.stream(self.stream).ok_or_else(|| Error::msg("The video stream is gone."))?.parameters();
        self.decoder = open_decoder(parameters, false, self.keyframes)?.0;
        self.hardware = false;
        self.failures = 0;
        self.scaler = None;
        self.seek(time)
    }

    fn seek(&mut self, time: f64) -> Result<()> {
        let target = ((time + self.time_base.seconds(self.start)) * ffmpeg::ffi::AV_TIME_BASE as f64) as i64;
        // Any keyframe at or before the target; decoding then runs forward to it.
        if self.input.seek(target, ..target).is_err() {
            self.input.seek(0, ..).map_err(av)?;
        }
        self.decoder.flush();
        self.current = None;
        self.next = None;
        self.eof = false;
        self.produced = 0;
        Ok(())
    }

    /// Decodes the next frame in stream order.
    fn decode_next(&mut self) -> Result<Option<Decoded>> {
        let mut frame = ffmpeg::frame::Video::empty();
        loop {
            match self.decoder.receive_frame(&mut frame) {
                Ok(()) => {
                    self.failures = 0;
                    self.produced += 1;
                    let pts = frame.timestamp().or(frame.pts()).unwrap_or(self.start);
                    return Ok(Some(Decoded { pts, time: self.time_base.seconds(pts - self.start), frame }));
                }
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::ffi::EAGAIN => {}
                Err(ffmpeg::Error::Eof) => {}
                Err(error) => self.failed(error)?,
            }
            if self.eof {
                return Ok(None);
            }
            match self.input.packets().next() {
                Some((stream, packet)) => {
                    if stream.index() == self.stream {
                        // A broken packet must not stop playback: skip it (but a
                        // GPU decoder that keeps failing gives way to the CPU).
                        if let Err(error) = self.decoder.send_packet(&packet) {
                            self.failed(error)?;
                        }
                    }
                }
                None => {
                    self.eof = true;
                    let _ = self.decoder.send_eof();
                }
            }
        }
    }

    /// Counts a decoding error; on the GPU, several in a row are a failure.
    fn failed(&mut self, error: ffmpeg::Error) -> Result<()> {
        if !self.hardware {
            return Ok(());
        }
        self.failures += 1;
        if self.failures >= HARDWARE_FAILURES {
            return Err(Error::msg(format!("GPU decoder: {error}")));
        }
        Ok(())
    }

    /// The frame on screen at `time` seconds (from the start of the file).
    pub fn frame_at(&mut self, time: f64) -> Result<Option<RgbaImage>> {
        match self.try_frame_at(time) {
            Err(error) if self.hardware => {
                self.fall_back(time, &error)?;
                self.try_frame_at(time)
            }
            result => result,
        }
    }

    fn try_frame_at(&mut self, time: f64) -> Result<Option<RgbaImage>> {
        let time = time.max(0.0);
        let behind = self.current.as_ref().is_some_and(|c| time + self.frame_length * 0.5 < c.time);
        let far = self.current.as_ref().is_none_or(|c| time > c.time + FORWARD_WINDOW);
        if behind || (far && self.next.as_ref().is_none_or(|n| time > n.time + FORWARD_WINDOW)) {
            self.seek(time)?;
        }
        // Advance while the following frame starts at or before `time`.
        loop {
            if self.next.is_none() {
                self.next = self.decode_next()?;
            }
            let Some(next) = &self.next else { break };
            if self.current.is_some() && next.time > time + self.frame_length * 0.5 {
                break;
            }
            self.current = self.next.take();
        }
        if self.hardware && self.current.is_none() && self.produced == 0 {
            return Err(Error::msg("GPU decoder gave no picture"));
        }
        match self.current.take() {
            Some(current) => {
                let image = self.convert(&current.frame);
                self.current = Some(current);
                image.map(Some)
            }
            None => Ok(None),
        }
    }

    /// The keyframe at or before `time`: with a decoder from
    /// [`VideoDecoder::open_keyframes`] this decodes a single frame, and none
    /// when it is the keyframe returned last.
    pub fn keyframe_at(&mut self, time: f64) -> Result<Option<RgbaImage>> {
        self.seek(time.max(0.0))?;
        // The packet the seek landed on tells which keyframe comes next.
        let mut landed = None;
        while let Some((stream, packet)) = self.input.packets().next() {
            if stream.index() != self.stream {
                continue;
            }
            landed = packet.pts().or(packet.dts());
            if let (Some(at), Some((key, image))) = (landed, &self.last_key) {
                if at == *key {
                    return Ok(Some(image.clone()));
                }
            }
            if let Err(error) = self.decoder.send_packet(&packet) {
                self.failed(error)?;
            }
            break;
        }
        let decoded = match self.decode_next()? {
            Some(decoded) => decoded,
            // A stream without flagged keyframes: decode every frame instead.
            None if self.keyframes && self.last_key.is_none() => {
                self.decoder.skip_frame(ffmpeg::Discard::Default);
                self.keyframes = false;
                return self.frame_at(time);
            }
            None => return Ok(self.last_key.as_ref().map(|(_, image)| image.clone())),
        };
        let image = self.convert(&decoded.frame)?;
        self.last_key = Some((landed.unwrap_or(decoded.pts), image.clone()));
        Ok(Some(image))
    }

    /// Timestamp of the frame last returned by `frame_at`.
    pub fn current_time(&self) -> Option<f64> {
        self.current.as_ref().map(|c| c.time)
    }

    /// Timestamp of the frame after the one `frame_at` returned last (decoding
    /// it if needed), or None at the end of the stream: until then that frame
    /// is the one on screen.
    pub fn next_time(&mut self) -> Result<Option<f64>> {
        if self.next.is_none() && self.current.is_some() {
            self.next = self.decode_next()?;
        }
        Ok(self.next.as_ref().map(|n| n.time))
    }

    fn convert(&mut self, frame: &ffmpeg::frame::Video) -> Result<RgbaImage> {
        // GPU frames are copied to memory first (as NV12 or P010).
        let mut downloaded = None;
        // SAFETY: reading a field of a valid frame.
        if unsafe { !(*frame.as_ptr()).hw_frames_ctx.is_null() } {
            let mut memory = ffmpeg::frame::Video::empty();
            // SAFETY: both frames are valid; the copy allocates its own buffers.
            let status = unsafe { ffmpeg::ffi::av_hwframe_transfer_data(memory.as_mut_ptr(), frame.as_ptr(), 0) };
            if status < 0 {
                return Err(Error::msg(format!("GPU frame download failed ({status})")));
            }
            downloaded = Some(memory);
        }
        let frame = downloaded.as_ref().unwrap_or(frame);
        let (out_w, out_h) = if self.rotation % 180 == 90 { (self.height, self.width) } else { (self.width, self.height) };
        let needs_new = self.scaler.as_ref().is_none_or(|(_, format, w, h)| *format != frame.format() || *w != frame.width() || *h != frame.height());
        if needs_new {
            let context = scaling::Context::get(
                frame.format(),
                frame.width(),
                frame.height(),
                ffmpeg::format::Pixel::RGBA,
                out_w,
                out_h,
                Flags::BILINEAR | Flags::FULL_CHR_H_INT | Flags::ACCURATE_RND,
            )
            .map_err(av)?;
            self.scaler = Some((context, frame.format(), frame.width(), frame.height()));
        }
        let (scaler, ..) = self.scaler.as_mut().expect("scaler");
        let mut rgba = ffmpeg::frame::Video::empty();
        scaler.run(frame, &mut rgba).map_err(av)?;
        let stride = rgba.stride(0);
        let row = out_w as usize * 4;
        let mut data = Vec::with_capacity(row * out_h as usize);
        for y in 0..out_h as usize {
            data.extend_from_slice(&rgba.data(0)[y * stride..y * stride + row]);
        }
        let image = RgbaImage { width: out_w, height: out_h, data };
        Ok(if self.rotation != 0 { image.rotated(self.rotation) } else { image })
    }
}

/// Decodes one picture (an image file, or the first frame of a video).
pub fn load_image(path: &Path, max_w: u32, max_h: u32) -> Result<RgbaImage> {
    let mut decoder = VideoDecoder::open_keyframes(path, max_w, max_h)?;
    decoder.frame_at(0.0)?.ok_or_else(|| Error::msg(format!("{} has no picture.", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GPU failure in the middle of a clip continues on the CPU at the same frame.
    #[test]
    fn decoding_continues_on_the_cpu_after_a_gpu_failure() {
        let dir = std::env::temp_dir().join("vf-media-fixtures");
        let path = dir.join("counter-hevc.mp4");
        if !path.is_file() {
            return;
        }
        let red = |decoder: &mut VideoDecoder, time: f64| {
            let frame = decoder.frame_at(time).unwrap().expect("frame");
            (frame.data[((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize] as f64 / 8.0).round() as i32
        };
        let mut decoder = VideoDecoder::open(&path, 128, 96).unwrap();
        assert_eq!(red(&mut decoder, 0.3), 9);
        decoder.fall_back(0.4, &Error::msg("simulated")).unwrap();
        assert!(!decoder.is_hardware());
        assert_eq!(red(&mut decoder, 0.4), 12);
        assert_eq!(red(&mut decoder, 0.5), 15);
    }
}
