//! Bounded native MP4 decoding for progressive 8-bit 4:2:0 AVC Baseline/Main and AAC-LC.
//! It rejects unsupported content rather than silently dropping streams or substituting frames.

#![deny(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::VecDeque;
use std::sync::Arc;

use filmcraft_aac::{AudioSpecificConfig, Decoder as AacDecoder};
use filmcraft_bitstream::{length_prefixed_nals, unescape_rbsp};
use filmcraft_h264::{
    Decoder, Picture,
    params::{Pps, Sps},
};
use filmcraft_isobmff::{CodecConfig, TrackKind};

/// Maximum accepted encoded source size.
pub const MAX_SOURCE_BYTES: usize = 16 * 1024 * 1024;
/// Maximum visible width and coded width.
pub const MAX_DIMENSION: u32 = 1920;
/// Maximum visible frame height.
pub const MAX_HEIGHT: u32 = 1080;
/// Maximum coded frame height, including H.264 macroblock padding.
pub const MAX_CODED_HEIGHT: u32 = 1088;
/// Maximum coded frame pixels.
pub const MAX_CODED_PIXELS: u64 = 1920 * 1088;
/// Maximum clip duration.
pub const MAX_DURATION_SECONDS: f64 = 60.0;
/// Maximum video samples.
pub const MAX_SAMPLES: usize = 1800;
/// Maximum AAC access units per audio track.
pub const MAX_AUDIO_ACCESS_UNITS: usize = 3000;
/// Maximum total audio plus video sample entries accepted before demux.
pub const MAX_TOTAL_SAMPLES: usize = MAX_SAMPLES + MAX_AUDIO_ACCESS_UNITS;
/// Maximum nominal frame rate.
pub const MAX_FRAME_RATE: f64 = 30.0;
/// Maximum H.264 reference frames.
pub const MAX_REFERENCE_FRAMES: u32 = 4;
/// Maximum samples between verified IDR frames.
pub const MAX_GOP_FRAMES: usize = 60;
const MAX_CACHED_FRAMES: usize = 4;
const MAX_MOOV_BYTES: usize = 2 * 1024 * 1024;
/// NAL units one sample may hold (a normal sample has a handful).
const MAX_NALS_PER_SAMPLE: usize = 64;
const MAX_DECODER_DPB_FRAMES: usize = 16;
const MAX_REORDER_LOOKAHEAD: usize = MAX_DECODER_DPB_FRAMES;
const MAX_AUDIO_RATE: u32 = 48_000;
const MAX_AUDIO_CHANNELS: usize = 2;
const MAX_AUDIO_PCM_BYTES: usize = 24 * 1024 * 1024;
const AAC_SAMPLES_PER_ACCESS_UNIT: usize = 1024;
const MAX_AUDIO_PRIMING_SAMPLES: usize = 2 * AAC_SAMPLES_PER_ACCESS_UNIT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum YuvMatrix {
    Bt601,
    Bt709,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct YuvColor {
    full_range: bool,
    matrix: YuvMatrix,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VideoError {
    Empty,
    SourceTooLarge,
    InvalidContainer,
    Unsupported(&'static str),
    Decode(String),
    Cancelled,
    InvalidTime,
    FrameUnavailable,
}

impl std::fmt::Display for VideoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("video source is empty"),
            Self::SourceTooLarge => f.write_str("video source exceeds 16 MiB"),
            Self::InvalidContainer => f.write_str("invalid MP4 container"),
            Self::Unsupported(reason) => write!(f, "unsupported video: {reason}"),
            Self::Decode(reason) => write!(f, "video decode failed: {reason}"),
            Self::Cancelled => f.write_str("video frame request was cancelled"),
            Self::InvalidTime => f.write_str("video time must be finite and non-negative"),
            Self::FrameUnavailable => f.write_str("requested video frame is unavailable"),
        }
    }
}

impl std::error::Error for VideoError {}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoMetadata {
    pub width: u32,
    pub height: u32,
    pub duration_seconds: f64,
    pub frame_count: u32,
    pub frame_interval_seconds: f64,
    pub audio: Option<AudioMetadata>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioMetadata {
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_seconds: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioClip {
    pub sample_rate: u32,
    pub channels: u16,
    /// Interleaved finite PCM beginning at presentation time zero.
    pub samples: Vec<f32>,
}

#[derive(Clone, Copy, Debug)]
struct AudioTimeline {
    sample_rate: u32,
    channels: u16,
    leading_silence_samples: usize,
    media_start_sample: usize,
    media_sample_count: usize,
    output_sample_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub frame_index: u32,
    pub next_frame_seconds: f64,
}

/// Validate source byte limits before media parsing or allocation.
pub fn validate_source(bytes: &[u8]) -> Result<(), VideoError> {
    if bytes.is_empty() {
        return Err(VideoError::Empty);
    }
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(VideoError::SourceTooLarge);
    }
    Ok(())
}

fn samples_from_ticks(ticks: u64, timescale: u32, sample_rate: u32) -> Result<usize, VideoError> {
    if timescale == 0 {
        return Err(VideoError::InvalidContainer);
    }
    let scaled = u128::from(ticks) * u128::from(sample_rate);
    if scaled % u128::from(timescale) != 0 {
        return Err(VideoError::Unsupported("audio timeline is not aligned to PCM samples"));
    }
    usize::try_from(scaled / u128::from(timescale)).map_err(|_| VideoError::InvalidContainer)
}

fn ticks_from_samples(samples: usize, timescale: u32, sample_rate: u32) -> Result<i64, VideoError> {
    let scaled =
        u128::try_from(samples).map_err(|_| VideoError::InvalidContainer)?.checked_mul(u128::from(timescale)).ok_or(VideoError::InvalidContainer)?;
    if scaled % u128::from(sample_rate) != 0 {
        return Err(VideoError::Unsupported("audio timeline is not aligned to media ticks"));
    }
    i64::try_from(scaled / u128::from(sample_rate)).map_err(|_| VideoError::InvalidContainer)
}

fn validate_audio_track(file: &filmcraft_isobmff::Mp4File, track: &filmcraft_isobmff::Track) -> Result<AudioTimeline, VideoError> {
    if track.samples.is_empty() || track.samples.len() > MAX_AUDIO_ACCESS_UNITS {
        return Err(VideoError::Unsupported("AAC access-unit count exceeds 3000"));
    }
    if track.duration == 0 || track.timescale == 0 {
        return Err(VideoError::InvalidContainer);
    }
    let media_duration = track.duration as f64 / f64::from(track.timescale);
    if !media_duration.is_finite() || media_duration > MAX_DURATION_SECONDS {
        return Err(VideoError::Unsupported("audio duration exceeds 60 seconds"));
    }
    if track.entries.len() != 1 {
        return Err(VideoError::Unsupported("AAC track must have one sample description"));
    }
    let entry = track.entries.first().ok_or(VideoError::InvalidContainer)?;
    if entry.format.0 != *b"mp4a" {
        return Err(VideoError::Unsupported("AAC mp4a audio required"));
    }
    let config = match &entry.codec {
        CodecConfig::Aac(config) => config,
        _ => return Err(VideoError::Unsupported("AAC-LC audio required")),
    };
    let asc = AudioSpecificConfig::parse(&config.asc).map_err(|e| {
        VideoError::Unsupported(match e {
            filmcraft_aac::Error::Unsupported(reason) => reason,
            _ => "invalid AAC AudioSpecificConfig",
        })
    })?;
    if asc.object_type != 2 || asc.sbr || asc.ps || asc.extension_sample_rate.is_some() || asc.pce.is_some() || asc.frame_length_flag {
        return Err(VideoError::Unsupported("plain AAC-LC 1024-sample frames required"));
    }
    if !(1..=MAX_AUDIO_RATE).contains(&asc.sample_rate) || !matches!(asc.channel_config, 1 | 2) {
        return Err(VideoError::Unsupported("AAC must be mono/stereo at up to 48 kHz"));
    }
    let channels = u16::try_from(asc.channels()).map_err(|_| VideoError::InvalidContainer)?;
    let audio_params = entry.audio.as_ref().ok_or(VideoError::InvalidContainer)?;
    if channels == 0
        || usize::from(channels) > MAX_AUDIO_CHANNELS
        || audio_params.channels != u32::from(channels)
        || audio_params.sample_rate != f64::from(asc.sample_rate)
    {
        return Err(VideoError::Unsupported("AAC sample-entry and AudioSpecificConfig disagree"));
    }
    let first = track.samples.first().ok_or(VideoError::InvalidContainer)?;
    let packet_duration_numerator =
        u64::from(AAC_SAMPLES_PER_ACCESS_UNIT as u32).checked_mul(u64::from(track.timescale)).ok_or(VideoError::InvalidContainer)?;
    if packet_duration_numerator % u64::from(asc.sample_rate) != 0 {
        return Err(VideoError::Unsupported("AAC packet duration is not aligned to its media timescale"));
    }
    let packet_duration_ticks = i64::try_from(packet_duration_numerator / u64::from(asc.sample_rate)).map_err(|_| VideoError::InvalidContainer)?;
    let mut decoded_duration = 0u64;
    for (index, sample) in track.samples.iter().enumerate() {
        let delta =
            i64::try_from(index).map_err(|_| VideoError::InvalidContainer)?.checked_mul(packet_duration_ticks).ok_or(VideoError::InvalidContainer)?;
        let expected_dts = first.dts.checked_add(delta).ok_or(VideoError::InvalidContainer)?;
        let is_last = index + 1 == track.samples.len();
        if sample.duration == 0
            || sample.duration > packet_duration_ticks as u32
            || (!is_last && sample.duration != packet_duration_ticks as u32)
            || sample.dts != expected_dts
            || sample.pts != sample.dts
        {
            return Err(VideoError::Unsupported("AAC access units must use a constant, non-reordered 1024-sample timeline"));
        }
        decoded_duration = decoded_duration.checked_add(u64::from(sample.duration)).ok_or(VideoError::InvalidContainer)?;
    }
    let decoded_samples = track.samples.len().checked_mul(AAC_SAMPLES_PER_ACCESS_UNIT).ok_or(VideoError::InvalidContainer)?;
    let media_duration_samples = samples_from_ticks(track.duration, track.timescale, asc.sample_rate)?;
    let decoded_duration_samples = samples_from_ticks(decoded_duration, track.timescale, asc.sample_rate)?;
    if media_duration_samples > decoded_duration_samples
        || decoded_duration_samples - media_duration_samples > 1
        || media_duration_samples > decoded_samples
        || decoded_samples - media_duration_samples > MAX_AUDIO_PRIMING_SAMPLES
    {
        return Err(VideoError::Unsupported("AAC media duration must match decoded access units within a one-sample tail"));
    }

    let (leading_silence_samples, media_start_sample, media_sample_count, output_sample_count) = if track.edits.is_empty() {
        if track.edit_offset != 0 {
            return Err(VideoError::Unsupported("unsupported AAC edit offset"));
        }
        (0, 0, media_duration_samples, media_duration_samples)
    } else {
        if track.edits.len() > 2 {
            return Err(VideoError::Unsupported("AAC edit list exceeds bounded leading-empty plus media edit"));
        }
        let mut leading_silence = 0usize;
        let mut media_edit = None;
        let mut segment_total = 0u64;
        for (index, edit) in track.edits.iter().enumerate() {
            if edit.media_rate != 0x0001_0000 {
                return Err(VideoError::Unsupported("AAC edit rate must be normal speed"));
            }
            segment_total = segment_total.checked_add(edit.segment_duration).ok_or(VideoError::InvalidContainer)?;
            if edit.media_time == -1 && index == 0 {
                leading_silence = samples_from_ticks(edit.segment_duration, file.timescale, asc.sample_rate)?;
            } else if edit.media_time >= 0 && media_edit.is_none() {
                media_edit = Some((samples_from_ticks(edit.media_time as u64, track.timescale, asc.sample_rate)?, edit.segment_duration));
            } else {
                return Err(VideoError::Unsupported("AAC edit list must have at most one leading empty edit and one media edit"));
            }
        }
        if media_edit.is_none() && track.edits.len() != 1 {
            return Err(VideoError::Unsupported("AAC edit list has no media segment"));
        }
        let duration_from_edits = samples_from_ticks(segment_total, file.timescale, asc.sample_rate)?;
        let tkhd_samples = samples_from_ticks(track.tkhd_duration, file.timescale, asc.sample_rate)?;
        if duration_from_edits != tkhd_samples {
            return Err(VideoError::Unsupported("AAC edit duration and track header disagree"));
        }
        let (media_start, media_count) = match media_edit {
            Some((start, duration)) => (start, samples_from_ticks(duration, file.timescale, asc.sample_rate)?),
            None => (0, 0),
        };
        let media_end = media_start.checked_add(media_count).ok_or(VideoError::InvalidContainer)?;
        let expected_offset = i64::try_from(leading_silence)
            .map_err(|_| VideoError::InvalidContainer)?
            .checked_mul(i64::from(track.timescale))
            .ok_or(VideoError::InvalidContainer)?
            .checked_div(i64::from(asc.sample_rate))
            .ok_or(VideoError::InvalidContainer)?
            .checked_sub(ticks_from_samples(media_start, track.timescale, asc.sample_rate)?)
            .ok_or(VideoError::InvalidContainer)?;
        if expected_offset != track.edit_offset || media_end > decoded_samples || media_start > MAX_AUDIO_PRIMING_SAMPLES {
            return Err(VideoError::Unsupported("AAC edit trim or priming exceeds decoded timeline bounds"));
        }
        let output_samples = leading_silence.checked_add(media_count).ok_or(VideoError::InvalidContainer)?;
        (leading_silence, media_start, media_count, output_samples)
    };
    if output_sample_count
        > usize::try_from(asc.sample_rate)
            .map_err(|_| VideoError::InvalidContainer)?
            .checked_mul(MAX_DURATION_SECONDS as usize)
            .ok_or(VideoError::InvalidContainer)?
    {
        return Err(VideoError::Unsupported("edited audio duration exceeds 60 seconds"));
    }
    let pcm_values = output_sample_count.checked_mul(usize::from(channels)).ok_or(VideoError::InvalidContainer)?;
    if pcm_values.checked_mul(std::mem::size_of::<f32>()).is_none_or(|size| size > MAX_AUDIO_PCM_BYTES) {
        return Err(VideoError::Unsupported("decoded audio exceeds 24 MiB PCM budget"));
    }
    Ok(AudioTimeline { sample_rate: asc.sample_rate, channels, leading_silence_samples, media_start_sample, media_sample_count, output_sample_count })
}

/// Open a bounded progressive AVC MP4 source with optional AAC-LC audio.
///
/// FilmCraft's decoder DPB is limited to 16 frames; this adapter's separate output cache is limited
/// to four RGBA frames. The SPS reference-frame limit remains four.
pub struct VideoSource {
    bytes: Arc<Vec<u8>>,
    file: filmcraft_isobmff::Mp4File,
    track_index: usize,
    decoder: Decoder,
    next_sample: usize,
    cache: VecDeque<VideoFrame>,
    metadata: VideoMetadata,
    color_override: Option<YuvColor>,
    audio_track_index: Option<usize>,
    audio_timeline: Option<AudioTimeline>,
    #[cfg(test)]
    converted_picture_indices: Vec<usize>,
}

impl VideoSource {
    pub fn open(bytes: Arc<Vec<u8>>) -> Result<Self, VideoError> {
        validate_source(bytes.as_slice())?;
        preflight(bytes.as_slice())?;
        let file = filmcraft_isobmff::open(bytes.as_slice()).map_err(|_| VideoError::InvalidContainer)?;
        if file.is_quicktime {
            return Err(VideoError::Unsupported("ISO MP4 brand required"));
        }
        if file.fragmented || file.fragment_count != 0 {
            return Err(VideoError::Unsupported("fragmented MP4"));
        }
        if file.tracks.is_empty() || file.tracks.len() > 2 {
            return Err(VideoError::Unsupported("one video track and at most one AAC audio track required"));
        }
        let mut video_track_index = None;
        let mut audio_track_index = None;
        for (index, track) in file.tracks.iter().enumerate() {
            match track.kind {
                TrackKind::Video if video_track_index.replace(index).is_some() => {
                    return Err(VideoError::Unsupported("multiple video tracks"));
                }
                TrackKind::Video => {}
                TrackKind::Audio if audio_track_index.replace(index).is_some() => {
                    return Err(VideoError::Unsupported("multiple audio tracks"));
                }
                TrackKind::Audio => {}
                _ => return Err(VideoError::Unsupported("unsupported additional MP4 track")),
            }
        }
        let track_index = video_track_index.ok_or(VideoError::InvalidContainer)?;
        let track = file.tracks.get(track_index).ok_or(VideoError::InvalidContainer)?;
        let (audio_timeline, audio_metadata) = match audio_track_index {
            Some(index) => {
                let audio_track = file.tracks.get(index).ok_or(VideoError::InvalidContainer)?;
                let timeline = validate_audio_track(&file, audio_track)?;
                let metadata = AudioMetadata {
                    sample_rate: timeline.sample_rate,
                    channels: timeline.channels,
                    duration_seconds: timeline.output_sample_count as f64 / f64::from(timeline.sample_rate),
                };
                (Some(timeline), Some(metadata))
            }
            None => (None, None),
        };
        if track.samples.is_empty() || track.samples.len() > MAX_SAMPLES {
            return Err(VideoError::Unsupported("sample count outside supported range"));
        }
        if track.timescale == 0 || track.duration == 0 {
            return Err(VideoError::InvalidContainer);
        }
        let video_duration_seconds = track.duration as f64 / track.timescale as f64;
        if !video_duration_seconds.is_finite() || video_duration_seconds <= 0.0 || video_duration_seconds > MAX_DURATION_SECONDS {
            return Err(VideoError::Unsupported("duration exceeds 60 seconds"));
        }
        let entry = track.entries.first().ok_or(VideoError::InvalidContainer)?;
        let avcc = match &entry.codec {
            CodecConfig::Avc(c) if entry.format.0 == *b"avc1" => c.clone(),
            _ => return Err(VideoError::Unsupported("AVC avc1 required")),
        };
        if track.entries.len() != 1 || avcc.length_size == 3 || avcc.length_size == 0 || !avcc.ext.is_empty() {
            return Err(VideoError::Unsupported("unsupported AVC configuration"));
        }
        let sps_table = validate_avcc_parameters(&avcc)?;
        let mut dimensions = None;
        for sps in sps_table.iter().flatten() {
            let (_, _, width, height) = sps.crop_rect();
            let dims = (width, height);
            if dimensions.replace(dims).is_some_and(|prev| prev != dims) {
                return Err(VideoError::Unsupported("multiple SPS dimensions"));
            }
        }
        let (width, height) = dimensions.ok_or(VideoError::InvalidContainer)?;
        if width > MAX_DIMENSION || height > MAX_HEIGHT || width == 0 || height == 0 {
            return Err(VideoError::Unsupported("visible dimensions exceed 1920x1080 limit"));
        }
        let video_params = entry.video.as_ref().ok_or(VideoError::InvalidContainer)?;
        if u32::from(video_params.width) != width || u32::from(video_params.height) != height {
            return Err(VideoError::Unsupported("track and SPS dimensions differ"));
        }
        let color_override = validate_video_color(video_params, &sps_table)?;
        let sample_count = track.samples.len();
        let first_sample = track.samples.first().ok_or(VideoError::InvalidContainer)?;
        let sample_duration = first_sample.duration;
        let covers = EditTimeline {
            movie_timescale: file.timescale,
            track_timescale: track.timescale,
            track_duration: track.duration,
            tolerance_ticks: u64::from(sample_duration),
        };
        if !video_edits_are_trivial(&track.edits, covers) {
            return Err(VideoError::Unsupported("video edit lists that trim, delay or change speed"));
        }
        if sample_duration == 0 {
            return Err(VideoError::InvalidContainer);
        }
        for (index, sample) in track.samples.iter().enumerate() {
            let expected = i64::from(sample_duration)
                .checked_mul(index as i64)
                .and_then(|delta| first_sample.dts.checked_add(delta))
                .ok_or(VideoError::InvalidContainer)?;
            if sample.duration != sample_duration || sample.dts != expected || sample.pts != sample.dts {
                return Err(VideoError::Unsupported("constant-rate, non-reordered video required"));
            }
        }
        let interval_seconds = sample_duration as f64 / track.timescale as f64;
        let expected_duration = interval_seconds * sample_count as f64;
        if (video_duration_seconds - expected_duration).abs() > interval_seconds {
            return Err(VideoError::Unsupported("track duration does not match sample timeline"));
        }
        if interval_seconds <= 0.0 || 1.0 / interval_seconds > MAX_FRAME_RATE + 0.01 {
            return Err(VideoError::Unsupported("frame rate exceeds 30 fps"));
        }
        let sync = track.sync_samples();
        if sync.first().copied() != Some(0) || sync.is_empty() {
            return Err(VideoError::Unsupported("video must start with an IDR sample"));
        }
        let mut previous_sync = 0usize;
        for &index in sync.iter().skip(1) {
            let index = index as usize;
            if index.saturating_sub(previous_sync) > MAX_GOP_FRAMES {
                return Err(VideoError::Unsupported("GOP exceeds 60 frames"));
            }
            previous_sync = index;
        }
        if sample_count.saturating_sub(previous_sync) > MAX_GOP_FRAMES {
            return Err(VideoError::Unsupported("GOP exceeds 60 frames"));
        }
        validate_all_samples(&file, bytes.as_slice(), track_index, &avcc, &sps_table)?;
        let mut decoder = Decoder::with_threads(1);
        decoder.configure_avcc(&avcc.to_bytes()).map_err(|e| VideoError::Decode(e.to_string()))?;
        let source = Self {
            bytes,
            file,
            track_index,
            decoder,
            next_sample: 0,
            cache: VecDeque::new(),
            metadata: VideoMetadata {
                width,
                height,
                duration_seconds: audio_metadata.as_ref().map_or(video_duration_seconds, |audio| video_duration_seconds.max(audio.duration_seconds)),
                frame_count: sample_count as u32,
                frame_interval_seconds: interval_seconds,
                audio: audio_metadata,
            },
            color_override,
            audio_track_index,
            audio_timeline,
            #[cfg(test)]
            converted_picture_indices: Vec::new(),
        };
        Ok(source)
    }

    pub fn metadata(&self) -> &VideoMetadata {
        &self.metadata
    }

    /// Decode the optional AAC-LC track to interleaved presentation-timeline PCM.
    ///
    /// A cancelled or failed decode returns no partial clip. The callback is checked before each
    /// encoded AAC access unit.
    pub fn decode_audio_cancellable(&self, mut is_cancelled: impl FnMut() -> bool) -> Result<Option<AudioClip>, VideoError> {
        let (Some(track_index), Some(timeline)) = (self.audio_track_index, self.audio_timeline) else {
            return Ok(None);
        };
        let track = self.file.tracks.get(track_index).ok_or(VideoError::InvalidContainer)?;
        let entry = track.entries.first().ok_or(VideoError::InvalidContainer)?;
        let config = match &entry.codec {
            CodecConfig::Aac(config) => config,
            _ => return Err(VideoError::InvalidContainer),
        };
        let mut decoder = AacDecoder::new(&config.asc).map_err(|e| VideoError::Decode(e.to_string()))?;
        let channels = usize::from(timeline.channels);
        if decoder.sample_rate() != timeline.sample_rate || decoder.channels() != channels {
            return Err(VideoError::Unsupported("AAC decoder metadata changed"));
        }
        let value_count = timeline.output_sample_count.checked_mul(channels).ok_or(VideoError::InvalidContainer)?;
        if value_count.checked_mul(std::mem::size_of::<f32>()).is_none_or(|bytes| bytes > MAX_AUDIO_PCM_BYTES) {
            return Err(VideoError::Unsupported("decoded audio exceeds 24 MiB PCM budget"));
        }
        let mut samples = vec![0.0f32; value_count];
        if timeline.media_sample_count == 0 {
            return Ok(Some(AudioClip { sample_rate: timeline.sample_rate, channels: timeline.channels, samples }));
        }
        let content_end = timeline.media_start_sample.checked_add(timeline.media_sample_count).ok_or(VideoError::InvalidContainer)?;
        let packet_limit = content_end.div_ceil(AAC_SAMPLES_PER_ACCESS_UNIT).min(track.samples.len());
        let mut written = 0usize;
        for packet_index in 0..packet_limit {
            if is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            let encoded = self.file.read_sample(self.bytes.as_slice(), track_index, packet_index).map_err(|_| VideoError::InvalidContainer)?;
            let planes = decoder.decode(&encoded).map_err(|e| VideoError::Decode(e.to_string()))?;
            if planes.len() != channels || planes.iter().any(|plane| plane.len() != AAC_SAMPLES_PER_ACCESS_UNIT) {
                return Err(VideoError::Decode("AAC decoder returned an incomplete access unit".into()));
            }
            for local_index in 0..AAC_SAMPLES_PER_ACCESS_UNIT {
                let source_index = packet_index * AAC_SAMPLES_PER_ACCESS_UNIT + local_index;
                if source_index < timeline.media_start_sample || source_index >= content_end {
                    continue;
                }
                let output_index = timeline.leading_silence_samples + source_index - timeline.media_start_sample;
                for (channel, plane) in planes.iter().enumerate() {
                    let sample = plane[local_index];
                    if !sample.is_finite() {
                        return Err(VideoError::Decode("AAC decoder returned non-finite PCM".into()));
                    }
                    let slot = output_index.checked_mul(channels).and_then(|v| v.checked_add(channel)).ok_or(VideoError::InvalidContainer)?;
                    *samples.get_mut(slot).ok_or(VideoError::InvalidContainer)? = sample;
                }
                written += 1;
            }
        }
        if written != timeline.media_sample_count {
            return Err(VideoError::FrameUnavailable);
        }
        Ok(Some(AudioClip { sample_rate: timeline.sample_rate, channels: timeline.channels, samples }))
    }

    pub fn frame_at(&mut self, seconds: f64) -> Result<VideoFrame, VideoError> {
        self.frame_at_cancellable(seconds, || false)
    }

    /// Decode one requested frame, checking cancellation before each encoded sample packet.
    pub fn frame_at_cancellable(&mut self, seconds: f64, mut is_cancelled: impl FnMut() -> bool) -> Result<VideoFrame, VideoError> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(VideoError::InvalidTime);
        }
        let last_frame = self.metadata.frame_count.saturating_sub(1) as usize;
        let frame_position = seconds / self.metadata.frame_interval_seconds;
        let boundary_roundoff = frame_position.abs().max(1.0) * f64::EPSILON * 4.0;
        let target = (frame_position + boundary_roundoff).floor() as usize;
        let target = target.min(last_frame);
        self.frame_index_cancellable(target, target, &mut is_cancelled)?;
        self.cache.iter().find(|frame| frame.frame_index as usize == target).cloned().ok_or(VideoError::FrameUnavailable)
    }

    /// Decode up to three frames after `anchor_frame`, keeping the anchor in the bounded cache.
    pub fn prefetch_after_cancellable(&mut self, anchor_frame: u32, mut is_cancelled: impl FnMut() -> bool) -> Result<(), VideoError> {
        let anchor = anchor_frame as usize;
        let frame_count = self.metadata.frame_count as usize;
        if anchor >= frame_count {
            return Err(VideoError::FrameUnavailable);
        }
        let end = anchor.saturating_add(MAX_CACHED_FRAMES - 1).min(frame_count.saturating_sub(1));
        for target in anchor.saturating_add(1)..=end {
            if is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            self.frame_index_cancellable(target, anchor, &mut is_cancelled)?;
        }
        Ok(())
    }

    fn frame_index_cancellable(&mut self, target: usize, cache_anchor: usize, is_cancelled: &mut impl FnMut() -> bool) -> Result<(), VideoError> {
        let cache_end = cache_anchor.saturating_add(MAX_CACHED_FRAMES - 1).min(self.metadata.frame_count.saturating_sub(1) as usize);
        self.cache.retain(|frame| (cache_anchor..=cache_end).contains(&(frame.frame_index as usize)));
        if self.cache.iter().any(|frame| frame.frame_index as usize == target) {
            return Ok(());
        }
        if target < self.next_sample || target.saturating_sub(self.next_sample) >= MAX_GOP_FRAMES {
            self.reset_at_sync(target, cache_anchor)?;
        }
        let lookahead_end = target.saturating_add(MAX_REORDER_LOOKAHEAD).min(self.metadata.frame_count.saturating_sub(1) as usize);
        while self.next_sample <= lookahead_end && !self.cache.iter().any(|frame| frame.frame_index as usize == target) {
            if is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            self.decode_one(self.next_sample, target, cache_anchor)?;
        }
        self.cache.iter().any(|frame| frame.frame_index as usize == target).then_some(()).ok_or(VideoError::FrameUnavailable)
    }
}

#[derive(Clone, Copy)]
struct BoxSpan {
    kind: [u8; 4],
    payload: usize,
    end: usize,
}

impl BoxSpan {
    fn payload_len(self) -> usize {
        self.end.saturating_sub(self.payload)
    }
}

fn box_span(bytes: &[u8], offset: usize, parent_end: usize) -> Result<BoxSpan, VideoError> {
    let base_end = offset.checked_add(8).ok_or(VideoError::InvalidContainer)?;
    if base_end > parent_end || parent_end > bytes.len() {
        return Err(VideoError::InvalidContainer);
    }
    let size32 = read_u32(bytes, offset).ok_or(VideoError::InvalidContainer)?;
    let kind: [u8; 4] = bytes.get(offset + 4..base_end).ok_or(VideoError::InvalidContainer)?.try_into().map_err(|_| VideoError::InvalidContainer)?;
    let (header_len, size) = match size32 {
        0 => (8usize, parent_end - offset),
        1 => {
            let header_end = offset.checked_add(16).ok_or(VideoError::InvalidContainer)?;
            if header_end > parent_end {
                return Err(VideoError::InvalidContainer);
            }
            let extended = read_u64(bytes, base_end).ok_or(VideoError::InvalidContainer)?;
            (16, usize::try_from(extended).map_err(|_| VideoError::InvalidContainer)?)
        }
        n => (8, n as usize),
    };
    let end = offset.checked_add(size).ok_or(VideoError::InvalidContainer)?;
    if size < header_len || end > parent_end {
        return Err(VideoError::InvalidContainer);
    }
    Ok(BoxSpan { kind, payload: offset + header_len, end })
}

#[derive(Default)]
struct PreflightState {
    moov: Option<BoxSpan>,
    fragmented: bool,
    boxes_seen: usize,
    sample_entries: usize,
    chunk_entries: usize,
    sample_to_chunk_entries: usize,
    decode_time_entries: usize,
    composition_time_entries: usize,
    sync_entries: usize,
}

fn preflight(bytes: &[u8]) -> Result<(), VideoError> {
    let mut state = PreflightState::default();
    scan_box_range(bytes, 0, bytes.len(), 0, &mut state)?;
    let moov = state.moov.ok_or(VideoError::InvalidContainer)?;
    if state.fragmented {
        return Err(VideoError::Unsupported("fragmented MP4"));
    }
    if moov.payload_len() > MAX_MOOV_BYTES {
        return Err(VideoError::Unsupported("movie metadata exceeds 2 MiB"));
    }
    Ok(())
}

fn scan_box_range(bytes: &[u8], start: usize, end: usize, depth: usize, state: &mut PreflightState) -> Result<(), VideoError> {
    if depth > 12 {
        return Err(VideoError::Unsupported("MP4 box nesting exceeds limit"));
    }
    let mut offset = start;
    while offset < end {
        state.boxes_seen = state.boxes_seen.checked_add(1).ok_or(VideoError::InvalidContainer)?;
        if state.boxes_seen > 100_000 {
            return Err(VideoError::Unsupported("MP4 box count exceeds limit"));
        }
        let span = box_span(bytes, offset, end)?;
        match &span.kind {
            b"moov" => {
                if state.moov.replace(span).is_some() {
                    return Err(VideoError::Unsupported("multiple movie headers"));
                }
                if span.payload_len() > MAX_MOOV_BYTES {
                    return Err(VideoError::Unsupported("movie metadata exceeds 2 MiB"));
                }
                scan_box_range(bytes, span.payload, span.end, depth + 1, state)?;
            }
            b"moof" | b"mvex" => state.fragmented = true,
            b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"udta" | b"meta" => {
                let child_start = if span.kind == *b"meta" { span.payload.checked_add(4).ok_or(VideoError::InvalidContainer)? } else { span.payload };
                if child_start > span.end {
                    return Err(VideoError::InvalidContainer);
                }
                scan_box_range(bytes, child_start, span.end, depth + 1, state)?;
            }
            b"stsz" | b"stz2" | b"stco" | b"co64" | b"stsc" | b"stts" | b"ctts" | b"stss" => check_sample_table(bytes, span, state)?,
            _ => {}
        }
        offset = span.end;
    }
    Ok(())
}

fn add_bounded_entries(total: &mut usize, count: usize) -> Result<(), VideoError> {
    *total = total.checked_add(count).ok_or(VideoError::InvalidContainer)?;
    if *total > MAX_TOTAL_SAMPLES {
        return Err(VideoError::Unsupported("aggregate sample table entries exceed 4800"));
    }
    Ok(())
}

fn check_sample_table(bytes: &[u8], span: BoxSpan, state: &mut PreflightState) -> Result<(), VideoError> {
    let payload = span.payload;
    let len = span.payload_len();
    let required = |n: usize| if len < n { Err(VideoError::InvalidContainer) } else { Ok(()) };
    let count_and_entries = |count_offset: usize, width: usize| -> Result<(usize, usize), VideoError> {
        required(count_offset)?;
        let count =
            read_u32(bytes, payload.checked_add(count_offset - 4).ok_or(VideoError::InvalidContainer)?).ok_or(VideoError::InvalidContainer)? as usize;
        if count > MAX_TOTAL_SAMPLES {
            return Err(VideoError::Unsupported("sample table exceeds 4800 entries"));
        }
        let entries_offset = payload.checked_add(count_offset).ok_or(VideoError::InvalidContainer)?;
        let entry_bytes = count.checked_mul(width).ok_or(VideoError::InvalidContainer)?;
        if entries_offset.checked_add(entry_bytes).is_none_or(|end| end > span.end) {
            return Err(VideoError::InvalidContainer);
        }
        Ok((count, entries_offset))
    };
    match &span.kind {
        b"stsz" => {
            required(12)?;
            let sample_size = read_u32(bytes, payload + 4).ok_or(VideoError::InvalidContainer)?;
            let count = read_u32(bytes, payload + 8).ok_or(VideoError::InvalidContainer)? as usize;
            if count > MAX_TOTAL_SAMPLES {
                return Err(VideoError::Unsupported("sample table exceeds 4800 entries"));
            }
            add_bounded_entries(&mut state.sample_entries, count)?;
            if sample_size == 0 && 12usize.checked_add(count.checked_mul(4).ok_or(VideoError::InvalidContainer)?).is_none_or(|need| need > len) {
                return Err(VideoError::InvalidContainer);
            }
        }
        b"stz2" => {
            required(12)?;
            let field_size = *bytes.get(payload + 7).ok_or(VideoError::InvalidContainer)?;
            let count = read_u32(bytes, payload + 8).ok_or(VideoError::InvalidContainer)? as usize;
            if count > MAX_TOTAL_SAMPLES {
                return Err(VideoError::Unsupported("sample table exceeds 4800 entries"));
            }
            add_bounded_entries(&mut state.sample_entries, count)?;
            if !matches!(field_size, 4 | 8 | 16) {
                return Err(VideoError::InvalidContainer);
            }
            let needed = count
                .checked_mul(field_size as usize)
                .and_then(|n| n.checked_add(7))
                .map(|n| n / 8)
                .and_then(|n| n.checked_add(12))
                .ok_or(VideoError::InvalidContainer)?;
            if needed > len {
                return Err(VideoError::InvalidContainer);
            }
        }
        b"stco" => {
            let (count, _) = count_and_entries(8, 4)?;
            add_bounded_entries(&mut state.chunk_entries, count)?;
        }
        b"co64" => {
            let (count, _) = count_and_entries(8, 8)?;
            add_bounded_entries(&mut state.chunk_entries, count)?;
        }
        b"stsc" => {
            let (count, _) = count_and_entries(8, 12)?;
            add_bounded_entries(&mut state.sample_to_chunk_entries, count)?;
        }
        b"stts" | b"ctts" => {
            let (count, entries) = count_and_entries(8, 8)?;
            if span.kind == *b"stts" {
                add_bounded_entries(&mut state.decode_time_entries, count)?;
            } else {
                add_bounded_entries(&mut state.composition_time_entries, count)?;
            }
            let mut samples = 0u64;
            for i in 0..count {
                let run = read_u32(bytes, entries + i * 8).ok_or(VideoError::InvalidContainer)?;
                samples = samples.checked_add(u64::from(run)).ok_or(VideoError::InvalidContainer)?;
                if samples > MAX_TOTAL_SAMPLES as u64 {
                    return Err(VideoError::Unsupported("sample timing table exceeds 4800 samples"));
                }
            }
        }
        b"stss" => {
            let (count, _) = count_and_entries(8, 4)?;
            add_bounded_entries(&mut state.sync_entries, count)?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?))
}
fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_be_bytes(bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?))
}

fn parse_sps(nal: &[u8]) -> Result<Sps, VideoError> {
    if nal.first().is_none_or(|byte| byte & 0x1f != 7) {
        return Err(VideoError::Unsupported("invalid SPS NAL"));
    }
    let rbsp = unescape_rbsp(nal.get(1..).ok_or(VideoError::InvalidContainer)?);
    Sps::parse(&rbsp).map_err(|e| VideoError::Decode(e.to_string()))
}

fn check_sps(sps: &Sps) -> Result<(), VideoError> {
    if !matches!(sps.profile_idc, 66 | 77) {
        return Err(VideoError::Unsupported("AVC Baseline or Main profile required"));
    }
    if sps.chroma_format_idc != 1 || sps.bit_depth_luma != 8 || sps.bit_depth_chroma != 8 || sps.separate_colour_plane {
        return Err(VideoError::Unsupported("8-bit 4:2:0 required"));
    }
    if !sps.frame_mbs_only || sps.max_num_ref_frames > MAX_REFERENCE_FRAMES {
        return Err(VideoError::Unsupported("progressive video with at most four references required"));
    }
    if sps.max_dpb_frames() > MAX_DECODER_DPB_FRAMES || sps.max_num_reorder_frames() > MAX_REORDER_LOOKAHEAD {
        return Err(VideoError::Unsupported("decoder DPB and reorder delay must not exceed 16 frames"));
    }
    let (_, _, visible_width, visible_height) = sps.crop_rect();
    let coded_pixels = u64::from(sps.width()) * u64::from(sps.height());
    if sps.width() > MAX_DIMENSION
        || sps.height() > MAX_CODED_HEIGHT
        || coded_pixels > MAX_CODED_PIXELS
        || visible_width > MAX_DIMENSION
        || visible_height > MAX_HEIGHT
    {
        return Err(VideoError::Unsupported("SPS dimensions exceed bounded 1920x1088 coded limit"));
    }
    let vui = sps.vui.as_ref();
    validate_colorimetry(
        vui.map_or(2, |v| u16::from(v.colour_primaries)),
        vui.map_or(2, |v| u16::from(v.transfer_characteristics)),
        vui.map_or(2, |v| u16::from(v.matrix_coefficients)),
        vui.is_some_and(|v| v.full_range),
    )?;
    Ok(())
}

fn validate_colorimetry(primaries: u16, transfer: u16, matrix: u16, full_range: bool) -> Result<YuvColor, VideoError> {
    if !matches!(primaries, 1 | 2 | 4 | 5 | 6 | 7) {
        return Err(VideoError::Unsupported("unsupported color primaries"));
    }
    if !matches!(transfer, 1 | 2 | 4 | 5 | 6 | 13) {
        return Err(VideoError::Unsupported("HDR or unsupported transfer characteristics"));
    }
    let matrix = match matrix {
        1 => YuvMatrix::Bt709,
        2 | 5 | 6 => YuvMatrix::Bt601,
        _ => return Err(VideoError::Unsupported("unsupported YCbCr matrix coefficients")),
    };
    Ok(YuvColor { full_range, matrix })
}

fn validate_video_color(video: &filmcraft_isobmff::VideoParams, sps: &[Option<Sps>]) -> Result<Option<YuvColor>, VideoError> {
    if video.mastering_display.is_some() || video.content_light.is_some() {
        return Err(VideoError::Unsupported("HDR mastering or content-light metadata"));
    }
    let sps_full_range = sps.iter().flatten().find_map(|s| s.vui.as_ref().map(|v| v.full_range)).unwrap_or(false);
    let color = match video.color.as_ref() {
        None => return Ok(None),
        Some(filmcraft_isobmff::ColorInfo::Nclx { primaries, transfer, matrix, full_range }) => {
            validate_colorimetry(*primaries, *transfer, *matrix, *full_range)?
        }
        Some(filmcraft_isobmff::ColorInfo::Nclc { primaries, transfer, matrix }) => {
            validate_colorimetry(*primaries, *transfer, *matrix, sps_full_range)?
        }
        Some(filmcraft_isobmff::ColorInfo::Icc { .. }) => {
            return Err(VideoError::Unsupported("embedded ICC video color profile"));
        }
    };
    Ok(Some(color))
}

fn validate_avcc_parameters(avcc: &filmcraft_isobmff::AvcConfig) -> Result<Vec<Option<Sps>>, VideoError> {
    if avcc.sps.is_empty() || avcc.pps.is_empty() {
        return Err(VideoError::InvalidContainer);
    }
    let mut sps_table = vec![None; 32];
    let mut sps_bytes = vec![None::<Vec<u8>>; 32];
    for nal in &avcc.sps {
        let sps = parse_sps(nal)?;
        check_sps(&sps)?;
        let slot = sps_table.get_mut(sps.id as usize).ok_or(VideoError::InvalidContainer)?;
        let bytes = sps_bytes.get_mut(sps.id as usize).ok_or(VideoError::InvalidContainer)?;
        if bytes.as_ref().is_some_and(|known| known.as_slice() != nal) {
            return Err(VideoError::Unsupported("conflicting SPS parameter-set id"));
        }
        *slot = Some(sps);
        *bytes = Some(nal.clone());
    }
    for nal in &avcc.pps {
        if nal.first().is_none_or(|byte| byte & 0x1f != 8) {
            return Err(VideoError::Unsupported("avcC PPS entry contains a non-PPS NAL"));
        }
        let rbsp = unescape_rbsp(nal.get(1..).ok_or(VideoError::InvalidContainer)?);
        let pps = Pps::parse(&rbsp, &sps_table).map_err(|e| VideoError::Decode(e.to_string()))?;
        let sps = sps_table.get(pps.sps_id as usize).and_then(Option::as_ref).ok_or(VideoError::InvalidContainer)?;
        if (sps.profile_idc == 66 && pps.entropy_coding_mode) || pps.num_slice_groups != 1 {
            return Err(VideoError::Unsupported("AVC Baseline/Main PPS without FMO required"));
        }
    }
    Ok(sps_table)
}

/// Timescales and lengths a video edit has to cover.
#[derive(Clone, Copy)]
struct EditTimeline {
    movie_timescale: u32,
    track_timescale: u32,
    track_duration: u64,
    /// How much shorter than the media the edit may be (one frame), in track ticks.
    tolerance_ticks: u64,
}

/// Video plays its samples from the start at normal speed, all of them: no edit list, or one edit
/// that starts at media time 0 without changing the rate and lasts as long as the media (give or
/// take a frame). (Audio honours its own edit list.)
fn video_edits_are_trivial(edits: &[filmcraft_isobmff::Edit], t: EditTimeline) -> bool {
    match edits {
        [] => true,
        [edit] => {
            if edit.media_time != 0 || edit.media_rate != 0x0001_0000 || t.movie_timescale == 0 || t.track_timescale == 0 {
                return false;
            }
            // Compare in movie ticks: segment >= media - tolerance.
            let segment = u128::from(edit.segment_duration) * u128::from(t.track_timescale);
            let media = u128::from(t.track_duration.saturating_sub(t.tolerance_ticks)) * u128::from(t.movie_timescale);
            segment >= media
        }
        _ => false,
    }
}

/// Samples must lie inside the source and not overlap, so the bytes read and parsed in total
/// never exceed the source's own size (a table pointing every sample at one payload would
/// multiply the work by the sample count).
fn check_sample_layout(samples: &[filmcraft_isobmff::Sample], source_len: usize) -> Result<(), VideoError> {
    let mut spans: Vec<(u64, u64)> = Vec::with_capacity(samples.len());
    for sample in samples {
        let end = sample.offset.checked_add(u64::from(sample.size)).ok_or(VideoError::InvalidContainer)?;
        if sample.size == 0 || end > source_len as u64 {
            return Err(VideoError::InvalidContainer);
        }
        spans.push((sample.offset, end));
    }
    spans.sort_unstable();
    if spans.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(VideoError::Unsupported("overlapping sample data"));
    }
    Ok(())
}

/// Count the length-prefixed NAL units of a sample without building them, refusing more than
/// [`MAX_NALS_PER_SAMPLE`].
fn check_nal_count(data: &[u8], length_size: usize) -> Result<(), VideoError> {
    let mut at = 0usize;
    let mut count = 0usize;
    while at < data.len() {
        let prefix = data.get(at..at + length_size).ok_or(VideoError::InvalidContainer)?;
        let len = prefix.iter().fold(0usize, |n, b| (n << 8) | usize::from(*b));
        at = at.checked_add(length_size).and_then(|a| a.checked_add(len)).ok_or(VideoError::InvalidContainer)?;
        if at > data.len() {
            return Err(VideoError::InvalidContainer);
        }
        count += 1;
        if count > MAX_NALS_PER_SAMPLE {
            return Err(VideoError::Unsupported("too many NAL units in one sample"));
        }
    }
    Ok(())
}

fn validate_all_samples(
    file: &filmcraft_isobmff::Mp4File,
    bytes: &[u8],
    track_index: usize,
    avcc: &filmcraft_isobmff::AvcConfig,
    sps_table: &[Option<Sps>],
) -> Result<(), VideoError> {
    let track = file.tracks.get(track_index).ok_or(VideoError::InvalidContainer)?;
    check_sample_layout(&track.samples, bytes.len())?;
    for (index, sample) in track.samples.iter().enumerate() {
        let data = file.read_sample(bytes, track_index, index).map_err(|_| VideoError::InvalidContainer)?;
        check_nal_count(&data, avcc.length_size as usize)?;
        let nals = length_prefixed_nals(&data, avcc.length_size as usize).map_err(|e| VideoError::Decode(e.to_string()))?;
        if nals.is_empty() {
            return Err(VideoError::InvalidContainer);
        }
        let mut has_idr = false;
        for nal in nals {
            let kind = nal.first().map(|byte| byte & 0x1f).ok_or(VideoError::InvalidContainer)?;
            match kind {
                5 => has_idr = true,
                7 => {
                    let sps = parse_sps(nal)?;
                    check_sps(&sps)?;
                    if !avcc.sps.iter().any(|configured| configured.as_slice() == nal) {
                        return Err(VideoError::Unsupported("in-band SPS change"));
                    }
                }
                8 => {
                    let rbsp = unescape_rbsp(nal.get(1..).ok_or(VideoError::InvalidContainer)?);
                    let pps = Pps::parse(&rbsp, sps_table).map_err(|e| VideoError::Decode(e.to_string()))?;
                    let sps = sps_table.get(pps.sps_id as usize).and_then(Option::as_ref).ok_or(VideoError::InvalidContainer)?;
                    if (sps.profile_idc == 66 && pps.entropy_coding_mode) || pps.num_slice_groups != 1 {
                        return Err(VideoError::Unsupported("AVC Baseline/Main PPS without FMO required"));
                    }
                    if !avcc.pps.iter().any(|configured| configured.as_slice() == nal) {
                        return Err(VideoError::Unsupported("in-band PPS change"));
                    }
                }
                _ => {}
            }
        }
        if sample.is_sync != has_idr {
            return Err(VideoError::Unsupported("sync table does not match IDR samples"));
        }
    }
    Ok(())
}

impl VideoSource {
    fn make_decoder(&self) -> Result<Decoder, VideoError> {
        let track = self.file.tracks.get(self.track_index).ok_or(VideoError::InvalidContainer)?;
        let entry = track.entries.first().ok_or(VideoError::InvalidContainer)?;
        let config = match &entry.codec {
            CodecConfig::Avc(c) => c,
            _ => return Err(VideoError::InvalidContainer),
        };
        let mut decoder = Decoder::with_threads(1);
        decoder.configure_avcc(&config.to_bytes()).map_err(|e| VideoError::Decode(e.to_string()))?;
        Ok(decoder)
    }

    fn reset_at_sync(&mut self, target: usize, cache_anchor: usize) -> Result<(), VideoError> {
        let track = self.file.tracks.get(self.track_index).ok_or(VideoError::InvalidContainer)?;
        let sync = track.sync_sample_before(target);
        if target.saturating_sub(sync) >= MAX_GOP_FRAMES {
            return Err(VideoError::Unsupported("seek exceeds bounded GOP"));
        }
        self.decoder = self.make_decoder()?;
        self.next_sample = sync;
        let cache_end = cache_anchor.saturating_add(MAX_CACHED_FRAMES - 1).min(self.metadata.frame_count.saturating_sub(1) as usize);
        self.cache.retain(|frame| (cache_anchor..=cache_end).contains(&(frame.frame_index as usize)));
        Ok(())
    }

    fn decode_one(&mut self, index: usize, requested_index: usize, cache_anchor: usize) -> Result<(), VideoError> {
        self.file.tracks.get(self.track_index).and_then(|t| t.samples.get(index)).ok_or(VideoError::FrameUnavailable)?;
        let data = self.file.read_sample(self.bytes.as_slice(), self.track_index, index).map_err(|_| VideoError::InvalidContainer)?;
        let pictures = self.decoder.decode(&data, index as i64).map_err(|e| VideoError::Decode(e.to_string()))?;
        if let Some(error) = self.decoder.take_error() {
            return Err(VideoError::Decode(error.to_string()));
        }
        self.accept_pictures(pictures, requested_index, cache_anchor)?;
        self.next_sample = index + 1;
        if self.next_sample == self.file.tracks.get(self.track_index).map_or(0, |t| t.samples.len()) {
            let pictures = self.decoder.flush();
            if let Some(error) = self.decoder.take_error() {
                return Err(VideoError::Decode(error.to_string()));
            }
            self.accept_pictures(pictures, requested_index, cache_anchor)?;
        }
        Ok(())
    }

    fn accept_pictures(&mut self, pictures: Vec<Picture>, requested_index: usize, cache_anchor: usize) -> Result<(), VideoError> {
        let cache_end = cache_anchor.saturating_add(MAX_CACHED_FRAMES - 1).min(self.metadata.frame_count.saturating_sub(1) as usize);
        for picture in pictures {
            if picture.width != self.metadata.width || picture.height != self.metadata.height {
                return Err(VideoError::Unsupported("decoded dimensions changed"));
            }
            let index = usize::try_from(picture.pts).map_err(|_| VideoError::InvalidContainer)?;
            if index >= self.metadata.frame_count as usize {
                return Err(VideoError::InvalidContainer);
            }
            validate_colorimetry(
                u16::from(picture.color.primaries),
                u16::from(picture.color.transfer),
                u16::from(picture.color.matrix),
                picture.color.full_range,
            )?;
            validate_picture_planes(&picture)?;
            if index < requested_index || !(cache_anchor..=cache_end).contains(&index) {
                continue;
            }
            #[cfg(test)]
            self.converted_picture_indices.push(index);
            let frame = to_rgba(picture, index, self.metadata.frame_interval_seconds, self.color_override)?;
            if let Some(existing) = self.cache.iter_mut().find(|f| f.frame_index as usize == index) {
                *existing = frame;
            } else {
                self.cache.push_back(frame);
            }
        }
        Ok(())
    }
}

fn validate_picture_planes(picture: &Picture) -> Result<(), VideoError> {
    let width = usize::try_from(picture.width).map_err(|_| VideoError::InvalidContainer)?;
    let height = usize::try_from(picture.height).map_err(|_| VideoError::InvalidContainer)?;
    let uv_width = width.div_ceil(2);
    let uv_height = height.div_ceil(2);
    let last_index = |plane_width: usize, plane_height: usize, stride: usize| {
        plane_height
            .checked_sub(1)
            .and_then(|row| row.checked_mul(stride))
            .and_then(|offset| offset.checked_add(plane_width.checked_sub(1)?))
            .ok_or(VideoError::InvalidContainer)
    };
    if picture.y.get(last_index(width, height, picture.y_stride)?).is_none()
        || picture.u.get(last_index(uv_width, uv_height, picture.uv_stride)?).is_none()
        || picture.v.get(last_index(uv_width, uv_height, picture.uv_stride)?).is_none()
    {
        return Err(VideoError::InvalidContainer);
    }
    Ok(())
}

fn to_rgba(picture: Picture, index: usize, interval: f64, override_color: Option<YuvColor>) -> Result<VideoFrame, VideoError> {
    let picture_color = validate_colorimetry(
        u16::from(picture.color.primaries),
        u16::from(picture.color.transfer),
        u16::from(picture.color.matrix),
        picture.color.full_range,
    )?;
    let color = override_color.unwrap_or(picture_color);
    let width = usize::try_from(picture.width).map_err(|_| VideoError::InvalidContainer)?;
    let height = usize::try_from(picture.height).map_err(|_| VideoError::InvalidContainer)?;
    let pixels = width.checked_mul(height).ok_or(VideoError::InvalidContainer)?;
    let bytes = pixels.checked_mul(4).ok_or(VideoError::InvalidContainer)?;
    validate_picture_planes(&picture)?;
    let (r_v, g_u, g_v, b_u, y_offset) = match (color.matrix, color.full_range) {
        (YuvMatrix::Bt601, false) => (409, 100, 208, 516, 16),
        (YuvMatrix::Bt709, false) => (459, 55, 136, 541, 16),
        (YuvMatrix::Bt601, true) => (359, 88, 183, 454, 0),
        (YuvMatrix::Bt709, true) => (403, 48, 120, 475, 0),
    };
    let y_scale = if color.full_range { 256 } else { 298 };
    let chroma_width = width.div_ceil(2);
    let mut rgba = vec![0; bytes];
    for y in 0..height {
        let y_start = y.checked_mul(picture.y_stride).ok_or(VideoError::InvalidContainer)?;
        let y_row = picture.y.get(y_start..y_start.checked_add(width).ok_or(VideoError::InvalidContainer)?).ok_or(VideoError::InvalidContainer)?;
        let uv_start = (y / 2).checked_mul(picture.uv_stride).ok_or(VideoError::InvalidContainer)?;
        let u_row =
            picture.u.get(uv_start..uv_start.checked_add(chroma_width).ok_or(VideoError::InvalidContainer)?).ok_or(VideoError::InvalidContainer)?;
        let v_row =
            picture.v.get(uv_start..uv_start.checked_add(chroma_width).ok_or(VideoError::InvalidContainer)?).ok_or(VideoError::InvalidContainer)?;
        let rgba_start = y.checked_mul(width).and_then(|row| row.checked_mul(4)).ok_or(VideoError::InvalidContainer)?;
        let rgba_row_end = rgba_start.checked_add(width.checked_mul(4).ok_or(VideoError::InvalidContainer)?).ok_or(VideoError::InvalidContainer)?;
        let rgba_row = rgba.get_mut(rgba_start..rgba_row_end).ok_or(VideoError::InvalidContainer)?;
        let chroma_pairs = u_row.iter().zip(v_row).flat_map(|(&u, &v)| std::iter::repeat_n((u, v), 2));
        for ((yv, (u, v)), pixel) in y_row.iter().zip(chroma_pairs).zip(rgba_row.chunks_exact_mut(4)) {
            let yv = *yv as i32;
            let u = u as i32 - 128;
            let v = v as i32 - 128;
            let c = (yv - y_offset).max(0);
            pixel.copy_from_slice(&[
                ((y_scale * c + r_v * v + 128) >> 8).clamp(0, 255) as u8,
                ((y_scale * c - g_u * u - g_v * v + 128) >> 8).clamp(0, 255) as u8,
                ((y_scale * c + b_u * u + 128) >> 8).clamp(0, 255) as u8,
                255,
            ]);
        }
    }
    Ok(VideoFrame {
        width: picture.width,
        height: picture.height,
        rgba,
        frame_index: index as u32,
        next_frame_seconds: (index + 1) as f64 * interval,
    })
}
