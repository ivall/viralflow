//! What a media file contains.

use std::path::Path;

use ffmpeg_next as ffmpeg;
use vf_core::model::MediaKind;
use vf_core::{Error, Result};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Probe {
    pub kind: Option<MediaKind>,
    pub duration: Option<f64>,
    /// Displayed size (rotation already applied).
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub has_audio: bool,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    /// Clockwise rotation the frames must be displayed with.
    pub rotation: i32,
    pub size: u64,
}

/// Clockwise display rotation of a video stream (phones record rotated frames).
pub(crate) fn stream_rotation(stream: &ffmpeg::format::stream::Stream) -> i32 {
    unsafe {
        let parameters = (*stream.as_ptr()).codecpar;
        if parameters.is_null() {
            return 0;
        }
        let side = ffmpeg::ffi::av_packet_side_data_get(
            (*parameters).coded_side_data,
            (*parameters).nb_coded_side_data,
            ffmpeg::ffi::AVPacketSideDataType::AV_PKT_DATA_DISPLAYMATRIX,
        );
        if side.is_null() || (*side).data.is_null() || (*side).size < 36 {
            return 0;
        }
        matrix_rotation((*side).data as *const i32)
    }
}

/// Clockwise display rotation of a decoded picture: photos keep their
/// orientation (EXIF) there rather than on the stream.
pub(crate) fn frame_rotation(frame: &ffmpeg::frame::Video) -> i32 {
    match frame.side_data(ffmpeg::frame::side_data::Type::DisplayMatrix) {
        // SAFETY: a display matrix is nine 32-bit values.
        Some(side) if side.data().len() >= 36 => unsafe { matrix_rotation(side.data().as_ptr() as *const i32) },
        _ => 0,
    }
}

/// # Safety
/// `matrix` points to a 3×3 display matrix.
unsafe fn matrix_rotation(matrix: *const i32) -> i32 {
    let angle = ffmpeg::ffi::av_display_rotation_get(matrix);
    if !angle.is_finite() {
        return 0;
    }
    // The matrix stores the counter-clockwise angle; snap to quarter turns.
    ((-angle / 90.0).round() as i32 * 90).rem_euclid(360)
}

fn rate(value: ffmpeg::Rational) -> Option<f64> {
    (value.numerator() > 0 && value.denominator() > 0).then(|| value.numerator() as f64 / value.denominator() as f64)
}

pub fn probe(path: &Path) -> Result<Probe> {
    crate::init();
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let size = std::fs::metadata(path).map(|m| m.len()).map_err(|_| Error::msg(format!("{name} no longer exists.")))?;
    let input = ffmpeg::format::input(&path).map_err(|error| Error::msg(format!("{name} cannot be read: {error}")))?;
    let format_name = input.format().name().to_string();
    let mut info = Probe { size, ..Default::default() };
    let duration = input.duration();
    if duration > 0 {
        info.duration = Some(duration as f64 / ffmpeg::ffi::AV_TIME_BASE as f64);
    }
    let mut frames = None;
    for stream in input.streams() {
        let parameters = stream.parameters();
        let medium = parameters.medium();
        let attached = stream.disposition().contains(ffmpeg::format::stream::Disposition::ATTACHED_PIC);
        if medium == ffmpeg::media::Type::Video && !attached && info.video_codec.is_none() {
            let codec = parameters.id();
            info.video_codec = Some(format!("{codec:?}").to_lowercase());
            let (mut width, mut height) = unsafe { ((*parameters.as_ptr()).width as u32, (*parameters.as_ptr()).height as u32) };
            info.rotation = stream_rotation(&stream);
            if info.rotation % 180 == 90 {
                std::mem::swap(&mut width, &mut height);
            }
            if width > 0 && height > 0 {
                info.width = Some(width);
                info.height = Some(height);
            }
            info.fps = rate(stream.avg_frame_rate()).or_else(|| rate(stream.rate())).filter(|fps| *fps < 1000.0);
            if info.duration.is_none() && stream.duration() > 0 {
                info.duration = Some(stream.duration() as f64 * f64::from(stream.time_base()));
            }
            frames = (stream.frames() > 0).then_some(stream.frames());
        }
        if medium == ffmpeg::media::Type::Audio && info.audio_codec.is_none() {
            info.has_audio = true;
            info.audio_codec = Some(format!("{:?}", parameters.id()).to_lowercase());
        }
    }
    let image_codec = matches!(info.video_codec.as_deref(), Some("png" | "mjpeg" | "webp" | "bmp" | "tiff" | "jpegxl" | "jpeg2000" | "qoi" | "gif"));
    let still = info.video_codec.is_some()
        && (format_name.contains("image2")
            || format_name.ends_with("_pipe")
            || (image_codec && frames.unwrap_or(1) <= 1 && info.duration.unwrap_or(0.0) < 0.5));
    if still && info.rotation == 0 {
        // A photo's orientation only shows on its decoded picture.
        info.rotation = crate::video::picture_rotation(path).unwrap_or(0);
        if info.rotation % 180 == 90 {
            (info.width, info.height) = (info.height, info.width);
        }
    }
    info.kind = if still {
        info.duration = None;
        info.has_audio = false;
        Some(MediaKind::Image)
    } else if info.video_codec.is_some() {
        Some(MediaKind::Video)
    } else if info.has_audio {
        Some(MediaKind::Audio)
    } else {
        None
    };
    if info.kind.is_none() {
        return Err(Error::msg(format!("{name} has no picture or sound ViralFlow can use.")));
    }
    Ok(info)
}
