//! libavfilter graphs over 48 kHz interleaved stereo f32, run in process (the
//! export's voice cleanup, tempo changes and loudness normalisation).

use ffmpeg::format::sample::Type as SampleType;
use ffmpeg::format::Sample;
use ffmpeg::{filter, ChannelLayout};
use ffmpeg_next as ffmpeg;
use vf_core::{Error, Result};

use crate::audio::{CHANNELS, SAMPLE_RATE};
use crate::av;

/// Frames pushed into the graph at a time.
const BLOCK: usize = 4096;

pub struct AudioFilter {
    graph: filter::Graph,
    pts: i64,
}

// The graph is only used from the thread that owns the filter.
unsafe impl Send for AudioFilter {}

impl AudioFilter {
    /// `chain` is a filter chain such as `"atempo=1.5,highpass=f=80"`; empty passes audio through.
    pub fn new(chain: &str) -> Result<Self> {
        crate::init();
        let mut graph = filter::Graph::new();
        let abuffer = filter::find("abuffer").ok_or_else(|| Error::msg("libavfilter has no abuffer."))?;
        let sink = filter::find("abuffersink").ok_or_else(|| Error::msg("libavfilter has no abuffersink."))?;
        let args = format!("time_base=1/{SAMPLE_RATE}:sample_rate={SAMPLE_RATE}:sample_fmt=flt:channel_layout=stereo");
        graph.add(&abuffer, "in", &args).map_err(av)?;
        graph.add(&sink, "out", "").map_err(av)?;
        // The sink's own format options changed between FFmpeg versions; aformat pins the output instead.
        let body = if chain.trim().is_empty() { "anull" } else { chain };
        let spec = format!("{body},aformat=sample_fmts=flt:sample_rates={SAMPLE_RATE}:channel_layouts=stereo");
        graph
            .output("in", 0)
            .and_then(|parser| parser.input("out", 0))
            .and_then(|parser| parser.parse(&spec))
            .map_err(|error| Error::msg(format!("Audio filter “{spec}” failed: {error}")))?;
        graph.validate().map_err(av)?;
        Ok(AudioFilter { graph, pts: 0 })
    }

    /// Pushes interleaved samples and appends whatever the graph has ready to `out`.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) -> Result<()> {
        for block in input.chunks(BLOCK * CHANNELS) {
            let frames = block.len() / CHANNELS;
            if frames == 0 {
                continue;
            }
            let mut frame = ffmpeg::frame::Audio::new(Sample::F32(SampleType::Packed), frames, ChannelLayout::STEREO);
            frame.set_rate(SAMPLE_RATE);
            frame.set_pts(Some(self.pts));
            self.pts += frames as i64;
            let bytes = frame.data_mut(0);
            for (dst, sample) in bytes.chunks_exact_mut(4).zip(block) {
                dst.copy_from_slice(&sample.to_ne_bytes());
            }
            self.send(Some(&frame))?;
            self.pull(out);
        }
        Ok(())
    }

    /// Flushes the graph (filters like loudnorm hold audio back) into `out`.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> Result<()> {
        self.send(None)?;
        self.pull(out);
        Ok(())
    }

    fn send(&mut self, frame: Option<&ffmpeg::frame::Audio>) -> Result<()> {
        let mut context = self.graph.get("in").ok_or_else(|| Error::msg("The audio filter lost its input."))?;
        let mut source = context.source();
        match frame {
            Some(frame) => source.add(frame).map_err(av),
            None => source.flush().map_err(av),
        }
    }

    fn pull(&mut self, out: &mut Vec<f32>) {
        let Some(mut context) = self.graph.get("out") else { return };
        let mut sink = context.sink();
        let mut frame = ffmpeg::frame::Audio::empty();
        while sink.frame(&mut frame).is_ok() {
            let count = frame.samples() * CHANNELS;
            let bytes = frame.data(0);
            out.extend(bytes[..count * 4].chunks_exact(4).map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])));
        }
    }
}

/// Runs `input` through `chain` in one go.
pub fn apply(chain: &str, input: &[f32]) -> Result<Vec<f32>> {
    if chain.trim().is_empty() {
        return Ok(input.to_vec());
    }
    let mut filter = AudioFilter::new(chain)?;
    let mut out = Vec::with_capacity(input.len() + 4096);
    filter.push(input, &mut out)?;
    filter.finish(&mut out)?;
    Ok(out)
}
