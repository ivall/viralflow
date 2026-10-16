//! Encoding the export: RGBA frames and 48 kHz stereo f32 into H.264, H.265 or
//! VP9 with AAC or Opus, muxed into MP4, MOV or WebM, all in process.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ffmpeg::format::sample::Type as SampleType;
use ffmpeg::format::{Pixel, Sample};
use ffmpeg::software::scaling;
use ffmpeg::{codec, encoder, ChannelLayout, Dictionary, Rational};
use ffmpeg_next as ffmpeg;
use vf_core::{Error, Result};

use crate::audio::{CHANNELS, SAMPLE_RATE};
use crate::av;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoCodec {
    H264,
    H265,
    Vp9,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Container {
    Mp4,
    Mov,
    Webm,
}

#[derive(Debug, Clone)]
pub struct EncodeSettings {
    pub path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: VideoCodec,
    pub container: Container,
    /// 0 draft, 1 standard, 2 high, 3 ultra.
    pub quality: usize,
    /// Use a GPU encoder when one works (H.264/H.265 only).
    pub hardware: bool,
    pub audio: bool,
}

struct AudioOut {
    encoder: encoder::Audio,
    stream: usize,
    time_base: Rational,
    frame_size: usize,
    format: Sample,
    pending: Vec<f32>,
    pts: i64,
}

pub struct Encoder {
    output: ffmpeg::format::context::Output,
    video: encoder::Video,
    video_stream: usize,
    video_time_base: Rational,
    scaler: scaling::Context,
    rgba: ffmpeg::frame::Video,
    yuv: ffmpeg::frame::Video,
    frame: i64,
    audio: Option<AudioOut>,
    /// Name of the video encoder in use (e.g. "h264_amf").
    pub encoder_name: String,
}

// The encoder is created and used on the export thread only.
unsafe impl Send for Encoder {}

fn software_name(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "libx264",
        VideoCodec::H265 => "libx265",
        VideoCodec::Vp9 => "libvpx-vp9",
    }
}

fn hardware_names(codec: VideoCodec) -> &'static [&'static str] {
    match codec {
        VideoCodec::H264 => &["h264_nvenc", "h264_amf", "h264_qsv", "h264_videotoolbox", "h264_mf"],
        VideoCodec::H265 => &["hevc_nvenc", "hevc_amf", "hevc_qsv", "hevc_videotoolbox", "hevc_mf"],
        VideoCodec::Vp9 => &[],
    }
}

/// Pixel format each encoder takes from us.
fn pixel_format(name: &str) -> Pixel {
    if name.contains("qsv") || name.contains("_mf") || name.contains("videotoolbox") {
        Pixel::NV12
    } else {
        Pixel::YUV420P
    }
}

/// Average bits per second a quality level aims at (also the size estimate).
pub fn target_bitrate(width: u32, height: u32, fps: u32, codec: VideoCodec, quality: usize) -> u64 {
    let bits_per_pixel = [0.05, 0.08, 0.12, 0.2][quality.min(3)] * if codec == VideoCodec::H264 { 1.0 } else { 0.65 };
    (width as f64 * height as f64 * fps as f64 * bits_per_pixel) as u64
}

/// Encoder options for a quality level (0..=3). Quality-based rate control
/// like the old CLI export, but with a bitrate ceiling: grainy footage (VHS,
/// noise) otherwise made files of hundreds of megabytes.
fn video_options(name: &str, quality: usize, target: u64) -> Dictionary<'static> {
    let q = quality.min(3);
    let ceiling = |factor: u64| (target * factor).max(1_000_000).to_string();
    let mut options = Dictionary::new();
    let mut set = |key: &str, value: String| options.set(key, &value);
    match name {
        "libvpx-vp9" => {
            set("crf", [38, 33, 30, 24][q].to_string());
            set("b", "0".into());
            set("row-mt", "1".into());
            set("deadline", "good".into());
            set("cpu-used", ["4", "3", "2", "1"][q].into());
        }
        "libx265" => {
            set("crf", [30, 26, 22, 19][q].to_string());
            set("preset", ["veryfast", "fast", "medium", "slow"][q].into());
            let (max, buffer) = (target * 3 / 1000, target * 6 / 1000);
            set("x265-params", format!("log-level=error:vbv-maxrate={max}:vbv-bufsize={buffer}"));
        }
        "libx264" => {
            set("crf", [28, 23, 19, 16][q].to_string());
            set("preset", ["veryfast", "fast", "medium", "slow"][q].into());
            set("profile", "high".into());
            set("maxrate", ceiling(3));
            set("bufsize", ceiling(6));
        }
        _ => {
            let level = [30, 26, 22, 18][q].to_string();
            if name.contains("nvenc") {
                set("preset", "p5".into());
                set("rc", "vbr".into());
                set("cq", level);
                set("maxrate", ceiling(2));
                set("bufsize", ceiling(4));
            } else if name.contains("qsv") {
                set("maxrate", ceiling(2));
            } else if name.contains("amf") {
                set("rc", "vbr_peak".into());
                set("quality", ["speed", "balanced", "quality", "quality"][q].into());
                set("maxrate", ceiling(2));
                set("bufsize", ceiling(4));
            }
        }
    }
    options
}

/// GPU encoders that open on this machine, probed once (first one wins).
fn working_hardware(codec: VideoCodec) -> Option<&'static str> {
    static H264: OnceLock<Option<&'static str>> = OnceLock::new();
    static H265: OnceLock<Option<&'static str>> = OnceLock::new();
    let cell = match codec {
        VideoCodec::H264 => &H264,
        VideoCodec::H265 => &H265,
        VideoCodec::Vp9 => return None,
    };
    *cell.get_or_init(|| hardware_names(codec).iter().copied().find(|name| probe_encoder(name)))
}

/// Opens an encoder at a small size to see whether the hardware behind it exists.
fn probe_encoder(name: &str) -> bool {
    let Some(found) = encoder::find_by_name(name) else { return false };
    let Ok(mut video) = codec::context::Context::new_with_codec(found).encoder().video() else { return false };
    video.set_width(256);
    video.set_height(256);
    video.set_format(pixel_format(name));
    video.set_time_base((1, 30));
    video.set_frame_rate(Some((30, 1)));
    video.set_bit_rate(2_000_000);
    let ok = video.open_with(video_options(name, 2, 2_000_000)).is_ok();
    log::info!("encoder {name}: {}", if ok { "available" } else { "not available" });
    ok
}

/// Name of the encoder an export with these settings would use.
pub fn encoder_for(codec: VideoCodec, hardware: bool) -> &'static str {
    if hardware {
        if let Some(name) = working_hardware(codec) {
            return name;
        }
    }
    software_name(codec)
}

impl Encoder {
    pub fn create(settings: &EncodeSettings) -> Result<Self> {
        crate::init();
        if settings.width % 2 != 0 || settings.height % 2 != 0 {
            return Err(Error::msg("The export size must be even."));
        }
        let format_name = match settings.container {
            Container::Mp4 => "mp4",
            Container::Mov => "mov",
            Container::Webm => "webm",
        };
        let mut output = ffmpeg::format::output_as(&settings.path, format_name)
            .map_err(|error| Error::msg(format!("{} cannot be written: {error}", settings.path.display())))?;
        let global_header = output.format().flags().contains(ffmpeg::format::Flags::GLOBAL_HEADER);

        // ------------------------------------------------ video
        let preferred = encoder_for(settings.codec, settings.hardware);
        let (video, name) = match open_video(preferred, settings, global_header) {
            Ok(video) => (video, preferred),
            Err(error) if preferred != software_name(settings.codec) => {
                log::warn!("{preferred} failed ({error}); using {}", software_name(settings.codec));
                let name = software_name(settings.codec);
                (open_video(name, settings, global_header)?, name)
            }
            Err(error) => return Err(error),
        };
        let found = encoder::find_by_name(name).ok_or_else(|| Error::msg(format!("{name} is missing")))?;
        let video_time_base = Rational::new(1, settings.fps as i32);
        let video_stream = {
            let mut stream = output.add_stream(found).map_err(av)?;
            stream.set_parameters(&video);
            stream.set_time_base(video_time_base);
            if settings.codec == VideoCodec::H265 && settings.container != Container::Webm {
                // Apple players need the hvc1 tag for HEVC in MP4/MOV.
                unsafe {
                    (*(*stream.as_mut_ptr()).codecpar).codec_tag = u32::from_le_bytes(*b"hvc1");
                }
            }
            stream.index()
        };

        // ------------------------------------------------ audio
        let audio = if settings.audio {
            let opus = settings.container == Container::Webm;
            let audio_name = if opus { "libopus" } else { "aac" };
            let found = encoder::find_by_name(audio_name).ok_or_else(|| Error::msg(format!("The {audio_name} encoder is missing.")))?;
            let mut context = codec::context::Context::new_with_codec(found).encoder().audio().map_err(av)?;
            let format = if opus { Sample::F32(SampleType::Packed) } else { Sample::F32(SampleType::Planar) };
            context.set_rate(SAMPLE_RATE as i32);
            context.set_format(format);
            context.set_channel_layout(ChannelLayout::STEREO);
            context.set_bit_rate(if opus { 160_000 } else { 192_000 });
            context.set_time_base((1, SAMPLE_RATE as i32));
            if global_header {
                context.set_flags(codec::Flags::GLOBAL_HEADER);
            }
            let encoder = context.open_with(Dictionary::new()).map_err(|error| Error::msg(format!("The {audio_name} encoder failed: {error}")))?;
            let frame_size = (encoder.frame_size() as usize).max(if opus { 960 } else { 1024 });
            let mut stream = output.add_stream(found).map_err(av)?;
            stream.set_parameters(&encoder);
            stream.set_time_base((1, SAMPLE_RATE as i32));
            Some(AudioOut { encoder, stream: stream.index(), time_base: Rational::new(1, SAMPLE_RATE as i32), frame_size, format, pending: Vec::new(), pts: 0 })
        } else {
            None
        };

        let mut options = Dictionary::new();
        if settings.container != Container::Webm {
            options.set("movflags", "+faststart");
        }
        output.write_header_with(options).map_err(av)?;

        let pixel = pixel_format(name);
        let mut scaler = scaling::Context::get(
            Pixel::RGBA,
            settings.width,
            settings.height,
            pixel,
            settings.width,
            settings.height,
            scaling::Flags::BICUBIC | scaling::Flags::ACCURATE_RND | scaling::Flags::FULL_CHR_H_INT,
        )
        .map_err(av)?;
        // Full-range RGB in, BT.709 limited range out (what players expect).
        unsafe {
            let bt709 = ffmpeg::ffi::sws_getCoefficients(ffmpeg::ffi::SWS_CS_ITU709);
            let source = ffmpeg::ffi::sws_getCoefficients(ffmpeg::ffi::SWS_CS_DEFAULT);
            ffmpeg::ffi::sws_setColorspaceDetails(scaler.as_mut_ptr(), source, 1, bt709, 0, 0, 1 << 16, 1 << 16);
        }
        Ok(Encoder {
            output,
            video,
            video_stream,
            video_time_base,
            scaler,
            rgba: ffmpeg::frame::Video::new(Pixel::RGBA, settings.width, settings.height),
            yuv: ffmpeg::frame::Video::new(pixel, settings.width, settings.height),
            frame: 0,
            audio,
            encoder_name: name.to_string(),
        })
    }

    /// Adds the next frame (tightly packed RGBA, opaque).
    pub fn push_video(&mut self, rgba: &[u8]) -> Result<()> {
        let (width, height) = (self.rgba.width() as usize, self.rgba.height() as usize);
        if rgba.len() < width * height * 4 {
            return Err(Error::msg("The rendered frame has the wrong size."));
        }
        let stride = self.rgba.stride(0);
        let data = self.rgba.data_mut(0);
        for y in 0..height {
            data[y * stride..y * stride + width * 4].copy_from_slice(&rgba[y * width * 4..(y + 1) * width * 4]);
        }
        self.scaler.run(&self.rgba, &mut self.yuv).map_err(av)?;
        self.yuv.set_pts(Some(self.frame));
        self.frame += 1;
        self.video.send_frame(&self.yuv).map_err(av)?;
        self.drain_video()
    }

    /// Adds interleaved stereo samples (any amount; they are cut into encoder frames).
    pub fn push_audio(&mut self, samples: &[f32]) -> Result<()> {
        let Some(audio) = self.audio.as_mut() else { return Ok(()) };
        audio.pending.extend_from_slice(samples);
        while audio.pending.len() >= audio.frame_size * CHANNELS {
            let chunk: Vec<f32> = audio.pending.drain(..audio.frame_size * CHANNELS).collect();
            send_audio(audio, &chunk)?;
            drain_audio(audio, &mut self.output)?;
        }
        Ok(())
    }

    /// Flushes both encoders and writes the file's index.
    pub fn finish(mut self) -> Result<()> {
        if let Some(audio) = self.audio.as_mut() {
            if !audio.pending.is_empty() {
                let mut chunk = std::mem::take(&mut audio.pending);
                chunk.resize(audio.frame_size * CHANNELS, 0.0);
                send_audio(audio, &chunk)?;
            }
            audio.encoder.send_eof().map_err(av)?;
            drain_audio(audio, &mut self.output)?;
        }
        self.video.send_eof().map_err(av)?;
        self.drain_video()?;
        self.output.write_trailer().map_err(av)?;
        Ok(())
    }

    fn drain_video(&mut self) -> Result<()> {
        let mut packet = ffmpeg::Packet::empty();
        while self.video.receive_packet(&mut packet).is_ok() {
            packet.set_stream(self.video_stream);
            let stream_base = self.output.stream(self.video_stream).map(|s| s.time_base()).unwrap_or(self.video_time_base);
            packet.rescale_ts(self.video_time_base, stream_base);
            packet.write_interleaved(&mut self.output).map_err(av)?;
        }
        Ok(())
    }
}

fn open_video(name: &str, settings: &EncodeSettings, global_header: bool) -> Result<encoder::Video> {
    let found = encoder::find_by_name(name).ok_or_else(|| Error::msg(format!("The {name} encoder is missing.")))?;
    let mut video = codec::context::Context::new_with_codec(found).encoder().video().map_err(av)?;
    video.set_width(settings.width);
    video.set_height(settings.height);
    video.set_format(pixel_format(name));
    video.set_time_base((1, settings.fps as i32));
    video.set_frame_rate(Some((settings.fps as i32, 1)));
    video.set_gop(settings.fps * 2);
    if global_header {
        video.set_flags(codec::Flags::GLOBAL_HEADER);
    }
    unsafe {
        let context = video.as_mut_ptr();
        (*context).color_primaries = ffmpeg::ffi::AVColorPrimaries::AVCOL_PRI_BT709;
        (*context).color_trc = ffmpeg::ffi::AVColorTransferCharacteristic::AVCOL_TRC_BT709;
        (*context).colorspace = ffmpeg::ffi::AVColorSpace::AVCOL_SPC_BT709;
        (*context).color_range = ffmpeg::ffi::AVColorRange::AVCOL_RANGE_MPEG;
    }
    let target = target_bitrate(settings.width, settings.height, settings.fps, settings.codec, settings.quality);
    // Hardware encoders aim at the target; software ones use it as a ceiling only.
    if !name.starts_with("lib") {
        video.set_bit_rate(target as usize);
    }
    video.open_with(video_options(name, settings.quality, target)).map_err(|error| Error::msg(format!("The {name} encoder failed: {error}")))
}

fn send_audio(audio: &mut AudioOut, chunk: &[f32]) -> Result<()> {
    let frames = chunk.len() / CHANNELS;
    let mut frame = ffmpeg::frame::Audio::new(audio.format, frames, ChannelLayout::STEREO);
    frame.set_rate(SAMPLE_RATE);
    frame.set_pts(Some(audio.pts));
    audio.pts += frames as i64;
    match audio.format {
        Sample::F32(SampleType::Planar) => {
            for channel in 0..CHANNELS {
                let plane = frame.plane_mut::<f32>(channel);
                for (i, slot) in plane.iter_mut().take(frames).enumerate() {
                    *slot = chunk[i * CHANNELS + channel];
                }
            }
        }
        _ => {
            let bytes = frame.data_mut(0);
            for (dst, sample) in bytes.chunks_exact_mut(4).zip(chunk) {
                dst.copy_from_slice(&sample.to_ne_bytes());
            }
        }
    }
    audio.encoder.send_frame(&frame).map_err(av)
}

fn drain_audio(audio: &mut AudioOut, output: &mut ffmpeg::format::context::Output) -> Result<()> {
    let mut packet = ffmpeg::Packet::empty();
    while audio.encoder.receive_packet(&mut packet).is_ok() {
        packet.set_stream(audio.stream);
        let stream_base = output.stream(audio.stream).map(|s| s.time_base()).unwrap_or(audio.time_base);
        packet.rescale_ts(audio.time_base, stream_base);
        packet.write_interleaved(output).map_err(av)?;
    }
    Ok(())
}

/// The partial file an export writes before it is moved into place.
pub fn partial_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "export".into());
    path.with_file_name(format!(".{name}.partial"))
}
