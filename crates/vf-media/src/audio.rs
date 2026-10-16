//! Audio decoding to 48 kHz interleaved stereo f32, with sample-accurate seeking.

use std::collections::VecDeque;
use std::path::Path;

use ffmpeg::format::sample::Type as SampleType;
use ffmpeg::format::Sample;
use ffmpeg::software::resampling;
use ffmpeg::ChannelLayout;
use ffmpeg_next as ffmpeg;
use vf_core::{Error, Result};

use crate::av;

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

pub struct AudioDecoder {
    input: ffmpeg::format::context::Input,
    stream: usize,
    decoder: ffmpeg::decoder::Audio,
    resampler: Option<resampling::Context>,
    time_base: f64,
    start: f64,
    /// Interleaved stereo samples not handed out yet.
    buffer: VecDeque<f32>,
    /// Time (s) of the first sample in `buffer`.
    position: f64,
    /// Samples before this time are dropped (after a seek).
    skip_until: f64,
    eof: bool,
    drained: bool,
}

unsafe impl Send for AudioDecoder {}

impl AudioDecoder {
    pub fn open(path: &Path) -> Result<Self> {
        crate::init();
        let input = ffmpeg::format::input(&path).map_err(|error| Error::msg(format!("{} cannot be opened: {error}", path.display())))?;
        let stream = input.streams().best(ffmpeg::media::Type::Audio).ok_or_else(|| Error::msg("This file has no sound."))?;
        let index = stream.index();
        let time_base = f64::from(stream.time_base());
        let start = if stream.start_time() != ffmpeg::ffi::AV_NOPTS_VALUE { stream.start_time() as f64 * time_base } else { 0.0 };
        let context = ffmpeg::codec::context::Context::from_parameters(stream.parameters()).map_err(av)?;
        let decoder = context.decoder().audio().map_err(av)?;
        Ok(AudioDecoder {
            input,
            stream: index,
            decoder,
            resampler: None,
            time_base,
            start,
            buffer: VecDeque::new(),
            position: 0.0,
            skip_until: 0.0,
            eof: false,
            drained: false,
        })
    }

    /// Continues reading from `time` seconds into the file.
    pub fn seek(&mut self, time: f64) -> Result<()> {
        let time = time.max(0.0);
        let target = ((time + self.start) * ffmpeg::ffi::AV_TIME_BASE as f64) as i64;
        if self.input.seek(target, ..target).is_err() {
            self.input.seek(0, ..).map_err(av)?;
        }
        self.decoder.flush();
        self.resampler = None;
        self.buffer.clear();
        self.eof = false;
        self.drained = false;
        self.position = time;
        self.skip_until = time;
        Ok(())
    }

    /// Time of the next sample `read` returns.
    pub fn position(&self) -> f64 {
        self.position
    }

    fn resample(&mut self, frame: &mut ffmpeg::frame::Audio) -> Result<Vec<f32>> {
        // Files without a channel layout (UNSPEC order) get the default one for their channel count.
        if frame.channel_layout().is_empty() {
            frame.set_channel_layout(ChannelLayout::default(frame.channels() as i32));
        }
        let layout = frame.channel_layout();
        if self.resampler.is_none() {
            let context = resampling::Context::get(frame.format(), layout, frame.rate(), Sample::F32(SampleType::Packed), ChannelLayout::STEREO, SAMPLE_RATE)
                .map_err(av)?;
            self.resampler = Some(context);
        }
        let resampler = self.resampler.as_mut().expect("resampler");
        // Room for the whole converted frame plus what swr buffered earlier;
        // the crate's default (input length) would silently drop samples.
        let capacity = (frame.samples() as u64 * SAMPLE_RATE as u64 / frame.rate().max(1) as u64) as usize + 512;
        let mut out = output_frame(capacity);
        resampler.run(frame, &mut out).map_err(av)?;
        Ok(packed(&out))
    }

    fn fill(&mut self) -> Result<bool> {
        let mut frame = ffmpeg::frame::Audio::empty();
        loop {
            if self.decoder.receive_frame(&mut frame).is_ok() {
                let stamp = frame.timestamp().or(frame.pts()).map(|t| t as f64 * self.time_base - self.start);
                let samples = self.resample(&mut frame)?;
                let mut samples = &samples[..];
                if let Some(stamp) = stamp {
                    if self.buffer.is_empty() {
                        self.position = stamp;
                    }
                }
                // After a seek, drop what lies before the requested time.
                if self.position < self.skip_until {
                    let drop_frames = (((self.skip_until - self.position) * SAMPLE_RATE as f64).round() as usize).min(samples.len() / CHANNELS);
                    samples = &samples[drop_frames * CHANNELS..];
                    self.position += drop_frames as f64 / SAMPLE_RATE as f64;
                    if !samples.is_empty() {
                        self.skip_until = self.position;
                    }
                }
                self.buffer.extend(samples);
                return Ok(true);
            }
            if self.eof {
                if !self.drained {
                    self.drained = true;
                    if let Some(resampler) = self.resampler.as_mut() {
                        let mut out = output_frame(4096);
                        if resampler.flush(&mut out).is_ok() && out.samples() > 0 {
                            self.buffer.extend(packed(&out));
                            return Ok(true);
                        }
                    }
                }
                return Ok(false);
            }
            match self.input.packets().next() {
                Some((stream, packet)) => {
                    if stream.index() == self.stream {
                        let _ = self.decoder.send_packet(&packet);
                    }
                }
                None => {
                    self.eof = true;
                    let _ = self.decoder.send_eof();
                }
            }
        }
    }

    /// Fills `out` (interleaved stereo); returns how many values are real
    /// samples. Past the end of the file the rest is silence.
    pub fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let mut written = 0;
        while written < out.len() {
            if self.buffer.is_empty() && !self.fill()? {
                break;
            }
            let take = (out.len() - written).min(self.buffer.len());
            for (slot, sample) in out[written..written + take].iter_mut().zip(self.buffer.drain(..take)) {
                *slot = sample;
            }
            written += take;
            self.position += (take / CHANNELS) as f64 / SAMPLE_RATE as f64;
        }
        out[written..].fill(0.0);
        Ok(written)
    }
}

fn output_frame(capacity: usize) -> ffmpeg::frame::Audio {
    let mut frame = ffmpeg::frame::Audio::new(Sample::F32(SampleType::Packed), capacity, ChannelLayout::STEREO);
    frame.set_rate(SAMPLE_RATE);
    frame
}

fn packed(frame: &ffmpeg::frame::Audio) -> Vec<f32> {
    let count = frame.samples() * CHANNELS;
    if count == 0 {
        return vec![];
    }
    let bytes = frame.data(0);
    bytes[..count * 4].chunks_exact(4).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])).collect()
}
