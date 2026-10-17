//! Native prototype audio output. PCM is prepared by the UI's decode worker; this module owns only
//! the device stream and its real-time-safe mixer callback.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use figcraft_media::AudioClip;
use figcraft_render::VideoInstanceId;
use figcraft_ui_egui::{AudioInspect, AudioVoice, AudioVoiceState, PrototypeAudio};

const MAX_VOICES: usize = 16;
const MAX_OUTPUT_CHANNELS: u16 = 32;

struct Voice {
    id: VideoInstanceId,
    clip: Option<Arc<AudioClip>>,
    duration_frames: u64,
    frame: AtomicU64,
    playing: AtomicBool,
    muted: AtomicBool,
    looped: AtomicBool,
    completed_loops: AtomicU64,
}

struct Playback {
    voices: Vec<Voice>,
    output_rate: u32,
    output_channels: u16,
    state_epoch: AtomicU64,
    playing: AtomicBool,
    callbacks: AtomicU64,
    nonzero_samples: AtomicU64,
}

struct ErrorState(std::sync::Mutex<Option<String>>);

pub struct Output {
    playback: Arc<Playback>,
    errors: Arc<ErrorState>,
    _stream: cpal::Stream,
}

#[derive(Clone, Copy, Default)]
struct VoiceSnapshot {
    frame: u64,
    duration_frames: u64,
    playing: bool,
    muted: bool,
    looped: bool,
    completed_loops: u64,
}

impl Output {
    pub fn start(voices: Vec<AudioVoice>, paused: bool) -> Result<Self, String> {
        validate_voices(&voices)?;
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or_else(|| "No default audio output device is available".to_string())?;
        let supported = device.default_output_config().map_err(|e| format!("Could not query audio output format: {e}"))?;
        let channels = supported.channels();
        // Mono is mixed down; stereo and wider layouts get the left and right signal in their first two channels.
        if !(1..=MAX_OUTPUT_CHANNELS).contains(&channels) {
            return Err(format!("Audio output has unsupported channel count {channels}"));
        }
        let rate = supported.sample_rate().0;
        if rate == 0 {
            return Err("Audio output reported an invalid sample rate".into());
        }
        for voice in &voices {
            if let Some(clip) = &voice.clip
                && (clip.sample_rate == 0
                    || !matches!(clip.channels, 1 | 2)
                    || clip.samples.len() < usize::from(clip.channels)
                    || clip.samples.len() % usize::from(clip.channels) != 0)
            {
                return Err("A decoded audio source has invalid PCM format".into());
            }
        }
        let voices = voices
            .into_iter()
            .map(|voice| Voice {
                id: voice.id,
                clip: voice.clip,
                duration_frames: duration_frames(voice.duration_seconds, rate),
                frame: AtomicU64::new(frames(voice.state.position, rate).min(duration_frames(voice.duration_seconds, rate))),
                playing: AtomicBool::new(voice.state.playing),
                muted: AtomicBool::new(voice.state.muted),
                looped: AtomicBool::new(voice.state.looped),
                completed_loops: AtomicU64::new(voice.state.completed_loops),
            })
            .collect();
        let playback = Arc::new(Playback {
            voices,
            output_rate: rate,
            output_channels: channels,
            state_epoch: AtomicU64::new(0),
            playing: AtomicBool::new(!paused),
            callbacks: AtomicU64::new(0),
            nonzero_samples: AtomicU64::new(0),
        });
        let errors = Arc::new(ErrorState(std::sync::Mutex::new(None)));
        let config: cpal::StreamConfig = supported.clone().into();
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, playback.clone(), errors.clone())?,
            cpal::SampleFormat::F64 => build::<f64>(&device, &config, playback.clone(), errors.clone())?,
            cpal::SampleFormat::I8 => build::<i8>(&device, &config, playback.clone(), errors.clone())?,
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, playback.clone(), errors.clone())?,
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, playback.clone(), errors.clone())?,
            cpal::SampleFormat::U8 => build::<u8>(&device, &config, playback.clone(), errors.clone())?,
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, playback.clone(), errors.clone())?,
            cpal::SampleFormat::U32 => build::<u32>(&device, &config, playback.clone(), errors.clone())?,
            format => return Err(format!("Audio output sample format {format:?} is unsupported")),
        };
        stream.play().map_err(|e| format!("Could not start audio output: {e}"))?;
        Ok(Self { playback, errors, _stream: stream })
    }
}

fn validate_voices(voices: &[AudioVoice]) -> Result<(), String> {
    if voices.is_empty() || voices.len() > MAX_VOICES {
        return Err(format!("Audio output requires 1 to {MAX_VOICES} voices"));
    }
    if !voices.iter().any(|v| v.clip.is_some()) {
        return Err("At least one decoded audio source is required".into());
    }
    for (index, voice) in voices.iter().enumerate() {
        if !voice.duration_seconds.is_finite() || voice.duration_seconds <= 0.0 {
            return Err("Audio voice duration must be finite and positive".into());
        }
        if !voice.state.position.is_finite() || voice.state.position < 0.0 {
            return Err("Audio voice position must be finite and nonnegative".into());
        }
        if voices[..index].iter().any(|other| other.id == voice.id) {
            return Err("Audio voice IDs must be unique".into());
        }
    }
    Ok(())
}

fn frames(seconds: f64, rate: u32) -> u64 {
    (seconds * f64::from(rate)).floor().min(u64::MAX as f64) as u64
}
fn duration_frames(seconds: f64, rate: u32) -> u64 {
    (seconds * f64::from(rate)).ceil().max(1.0).min(u64::MAX as f64) as u64
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, playback: Arc<Playback>, errors: Arc<ErrorState>) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| write_output(out, &playback),
            move |error| {
                if let Ok(mut slot) = errors.0.lock() {
                    *slot = Some(format!("Audio output stream failed: {error}"));
                }
            },
            None,
        )
        .map_err(|e| format!("Could not create audio output stream: {e}"))
}

fn write_output<T>(output: &mut [T], playback: &Playback)
where
    T: cpal::FromSample<f32> + cpal::Sample,
{
    playback.callbacks.fetch_add(1, Ordering::Relaxed);
    // A control change (seek, pause, mute) that lands while the buffer is mixed makes its position
    // update fail: mix once more from the new state, so the same stretch is not played twice.
    if mix_buffer(output, playback) {
        mix_buffer(output, playback);
    }
}

/// Mix one buffer from a snapshot of the voices. Returns whether the snapshot went stale (a
/// control change came in) so the positions were not advanced.
fn mix_buffer<T>(output: &mut [T], playback: &Playback) -> bool
where
    T: cpal::FromSample<f32> + cpal::Sample,
{
    let channels = usize::from(playback.output_channels);
    if channels == 0 {
        return false;
    }
    let epoch = playback.state_epoch.load(Ordering::Acquire);
    if epoch & 1 != 0 {
        output.fill(T::from_sample(0.0));
        return false;
    }
    let globally_playing = playback.playing.load(Ordering::Relaxed);
    let mut snapshots = [VoiceSnapshot { frame: 0, duration_frames: 0, playing: false, muted: false, looped: false, completed_loops: 0 }; MAX_VOICES];
    for (slot, voice) in snapshots.iter_mut().zip(&playback.voices) {
        *slot = VoiceSnapshot {
            frame: voice.frame.load(Ordering::Relaxed),
            duration_frames: voice.duration_frames,
            playing: voice.playing.load(Ordering::Relaxed),
            muted: voice.muted.load(Ordering::Relaxed),
            looped: voice.looped.load(Ordering::Relaxed),
            completed_loops: voice.completed_loops.load(Ordering::Relaxed),
        };
    }
    let start = snapshots;
    let frame_count = output.len() / channels;
    let mut nonzero = 0u64;
    let mut chunks = output.chunks_exact_mut(channels);
    for (offset, samples) in chunks.by_ref().enumerate() {
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        if globally_playing {
            for (voice, state) in playback.voices.iter().zip(&start) {
                if !state.playing || (state.frame >= state.duration_frames && !state.looped) || state.muted {
                    continue;
                }
                if let Some(clip) = &voice.clip {
                    let frame = state.frame.saturating_add(offset as u64);
                    let frame = if state.looped { frame % state.duration_frames } else { frame.min(state.duration_frames.saturating_sub(1)) };
                    if !state.looped && state.frame.saturating_add(offset as u64) >= state.duration_frames {
                        continue;
                    }
                    let time = frame as f64 / f64::from(playback.output_rate);
                    left += mix_channel(clip, time, 0, playback.output_channels);
                    right += if channels == 1 { 0.0 } else { mix_channel(clip, time, 1, playback.output_channels) };
                }
            }
        }
        let values = [left.clamp(-1.0, 1.0), right.clamp(-1.0, 1.0)];
        for (i, sample) in samples.iter_mut().enumerate() {
            // Channels past the first two stay silent.
            let value = values.get(i).copied().unwrap_or(0.0);
            if value != 0.0 {
                nonzero = nonzero.saturating_add(1);
            }
            *sample = T::from_sample(value);
        }
    }
    for sample in chunks.into_remainder() {
        *sample = T::from_sample(0.0);
    }
    playback.nonzero_samples.fetch_add(nonzero, Ordering::Relaxed);
    globally_playing && !commit_callback(playback, epoch, &start, frame_count, globally_playing)
}

fn commit_callback(playback: &Playback, epoch: u64, start: &[VoiceSnapshot; MAX_VOICES], frames: usize, globally_playing: bool) -> bool {
    if !globally_playing || playback.state_epoch.compare_exchange(epoch, epoch + 1, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return false;
    }
    for (voice, state) in playback.voices.iter().zip(start) {
        if !state.playing {
            continue;
        }
        let total = state.frame.saturating_add(frames as u64);
        if state.looped {
            let wraps = total / state.duration_frames;
            voice.frame.store(total % state.duration_frames, Ordering::Relaxed);
            voice.completed_loops.store(state.completed_loops.saturating_add(wraps), Ordering::Relaxed);
        } else if total >= state.duration_frames {
            voice.frame.store(state.duration_frames, Ordering::Relaxed);
            voice.playing.store(false, Ordering::Relaxed);
        } else {
            voice.frame.store(total, Ordering::Relaxed);
        }
    }
    playback.state_epoch.store(epoch + 2, Ordering::Release);
    true
}

fn update_playback(playback: &Playback, update: impl FnOnce()) {
    loop {
        let epoch = playback.state_epoch.load(Ordering::Acquire);
        if epoch & 1 != 0 {
            std::hint::spin_loop();
            continue;
        }
        if playback.state_epoch.compare_exchange(epoch, epoch + 1, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            update();
            playback.state_epoch.store(epoch + 2, Ordering::Release);
            return;
        }
    }
}

fn coherent_voice_state(playback: &Playback, voice: &Voice) -> AudioVoiceState {
    loop {
        let epoch = playback.state_epoch.load(Ordering::Acquire);
        if epoch & 1 != 0 {
            std::hint::spin_loop();
            continue;
        }
        let frame = voice.frame.load(Ordering::Relaxed);
        let playing = voice.playing.load(Ordering::Relaxed);
        let muted = voice.muted.load(Ordering::Relaxed);
        let looped = voice.looped.load(Ordering::Relaxed);
        let completed_loops = voice.completed_loops.load(Ordering::Relaxed);
        if playback.state_epoch.load(Ordering::Acquire) == epoch {
            return AudioVoiceState { position: frame as f64 / f64::from(playback.output_rate), playing, muted, looped, completed_loops };
        }
    }
}

fn mix_channel(clip: &AudioClip, time: f64, output_channel: u16, output_channels: u16) -> f32 {
    let channels = usize::from(clip.channels);
    let source_frame = time * f64::from(clip.sample_rate);
    let index = source_frame.floor() as usize;
    let fraction = (source_frame - index as f64) as f32;
    let sample = |frame: usize, channel: usize| {
        clip.samples.get(frame.saturating_mul(channels).saturating_add(channel)).copied().filter(|v| v.is_finite()).unwrap_or(0.0)
    };
    let channel_value = |channel: usize| {
        let a = sample(index, channel);
        let b = sample(index.saturating_add(1), channel);
        a + (b - a) * fraction
    };
    if clip.channels == 1 {
        channel_value(0)
    } else if output_channels == 1 {
        (channel_value(0) + channel_value(1)) * 0.5
    } else {
        channel_value(usize::from(output_channel.min(1)))
    }
}

impl PrototypeAudio for Output {
    fn play(&mut self) {
        update_playback(&self.playback, || self.playback.playing.store(true, Ordering::Relaxed));
    }
    fn pause(&mut self) {
        update_playback(&self.playback, || self.playback.playing.store(false, Ordering::Relaxed));
    }
    fn restart(&mut self) {
        update_playback(&self.playback, || {
            for voice in &self.playback.voices {
                voice.frame.store(0, Ordering::Relaxed);
                voice.completed_loops.store(0, Ordering::Relaxed);
            }
            self.playback.playing.store(true, Ordering::Relaxed);
        });
    }
    fn seek(&mut self, seconds: f64) {
        let frame = frames(if seconds.is_finite() { seconds.max(0.0) } else { 0.0 }, self.playback.output_rate);
        update_playback(&self.playback, || {
            for voice in &self.playback.voices {
                voice.frame.store(frame.min(voice.duration_frames), Ordering::Relaxed);
            }
        });
    }
    fn position_seconds(&self) -> f64 {
        self.playback.voices.iter().map(|v| v.frame.load(Ordering::Relaxed) as f64 / f64::from(self.playback.output_rate)).fold(0.0, f64::max)
    }
    fn inspect(&self) -> AudioInspect {
        AudioInspect {
            playing: self.playback.playing.load(Ordering::Relaxed)
                && self.playback.voices.iter().any(|v| {
                    v.playing.load(Ordering::Relaxed) && (v.looped.load(Ordering::Relaxed) || v.frame.load(Ordering::Relaxed) < v.duration_frames)
                }),
            position: self.position_seconds(),
            callbacks: self.playback.callbacks.load(Ordering::Relaxed),
            nonzero_samples: self.playback.nonzero_samples.load(Ordering::Relaxed),
            error: self.errors.0.lock().ok().and_then(|e| e.clone()),
            source_count: self
                .playback
                .voices
                .iter()
                .filter_map(|v| v.clip.as_ref())
                .map(Arc::as_ptr)
                .collect::<std::collections::HashSet<_>>()
                .len(),
        }
    }
    fn voice_state(&self, id: VideoInstanceId) -> Option<AudioVoiceState> {
        let voice = self.playback.voices.iter().find(|v| v.id == id)?;
        Some(coherent_voice_state(&self.playback, voice))
    }
    fn set_voice_state(&mut self, id: VideoInstanceId, state: AudioVoiceState) -> Result<(), String> {
        if !state.position.is_finite() || state.position < 0.0 {
            return Err("Audio voice position must be finite and nonnegative".into());
        }
        let Some(index) = self.playback.voices.iter().position(|v| v.id == id) else {
            return Err("Audio voice ID is not present".into());
        };
        let frame = frames(state.position, self.playback.output_rate).min(self.playback.voices[index].duration_frames);
        update_playback(&self.playback, || {
            let voice = &self.playback.voices[index];
            voice.frame.store(frame, Ordering::Relaxed);
            voice.playing.store(state.playing, Ordering::Relaxed);
            voice.muted.store(state.muted, Ordering::Relaxed);
            voice.looped.store(state.looped, Ordering::Relaxed);
            voice.completed_loops.store(state.completed_loops, Ordering::Relaxed);
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use figcraft_doc::NodeId;
    use figcraft_render::{PaintDomain, PaintId, VideoOccurrenceId, VideoPatternPath, VideoScopeId};

    fn id(node: u64) -> VideoInstanceId {
        VideoInstanceId {
            scope: VideoScopeId(1),
            occurrence: VideoOccurrenceId {
                paint: PaintId { node: NodeId(node), domain: PaintDomain::Fill, index: 0 },
                patterns: VideoPatternPath::default(),
            },
        }
    }

    fn voice(node: u64, clip: Option<Arc<AudioClip>>, pos: f64, playing: bool, duration: f64) -> AudioVoice {
        AudioVoice {
            id: id(node),
            clip,
            duration_seconds: duration,
            state: AudioVoiceState { position: pos, playing, muted: false, looped: false, completed_loops: 0 },
        }
    }

    fn playback(voices: Vec<AudioVoice>, rate: u32) -> Playback {
        Playback {
            voices: voices
                .into_iter()
                .map(|v| Voice {
                    id: v.id,
                    clip: v.clip,
                    duration_frames: duration_frames(v.duration_seconds, rate),
                    frame: AtomicU64::new(frames(v.state.position, rate)),
                    playing: AtomicBool::new(v.state.playing),
                    muted: AtomicBool::new(v.state.muted),
                    looped: AtomicBool::new(v.state.looped),
                    completed_loops: AtomicU64::new(v.state.completed_loops),
                })
                .collect(),
            output_rate: rate,
            output_channels: 1,
            state_epoch: AtomicU64::new(0),
            playing: AtomicBool::new(true),
            callbacks: AtomicU64::new(0),
            nonzero_samples: AtomicU64::new(0),
        }
    }

    #[test]
    fn mixer_resamples_and_maps_channels() {
        let clip = AudioClip { sample_rate: 2, channels: 1, samples: vec![0.0, 1.0, 0.0] };
        assert_eq!(mix_channel(&clip, 0.25, 0, 2), 0.5);
        assert_eq!(mix_channel(&clip, 0.25, 1, 2), 0.5);
    }

    #[test]
    fn wide_layouts_get_left_and_right_in_their_first_two_channels() {
        let pcm = Arc::new(AudioClip { sample_rate: 1, channels: 2, samples: vec![0.25, -0.5] });
        let mut wide = playback(vec![voice(1, Some(pcm), 0.0, true, 1.0)], 1);
        wide.output_channels = 6;
        let mut output = [1.0f32; 6];
        write_output(&mut output, &wide);
        assert_eq!(output, [0.25, -0.5, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn every_integer_and_float_output_format_is_written() {
        let pcm = Arc::new(AudioClip { sample_rate: 1, channels: 1, samples: vec![0.5, 0.5] });
        let one = || playback(vec![voice(1, Some(pcm.clone()), 0.0, true, 2.0)], 1);
        let mut i32s = [0i32; 1];
        write_output(&mut i32s, &one());
        assert!(i32s[0] > i32::MAX / 4);
        let mut u8s = [0u8; 1];
        write_output(&mut u8s, &one());
        assert!(u8s[0] > 128);
        let mut i8s = [0i8; 1];
        write_output(&mut i8s, &one());
        assert!(i8s[0] > 0);
        let mut f64s = [0f64; 1];
        write_output(&mut f64s, &one());
        assert_eq!(f64s[0], 0.5);
    }

    #[test]
    fn mixer_filters_nonfinite_pcm_and_maps_stereo_to_mono() {
        let bad = AudioClip { sample_rate: 1, channels: 1, samples: vec![f32::NAN, 0.4] };
        assert_eq!(mix_channel(&bad, 0.0, 0, 1), 0.0);
        let stereo = AudioClip { sample_rate: 1, channels: 2, samples: vec![0.8, -0.4] };
        assert_eq!(mix_channel(&stereo, 0.0, 0, 1), 0.2);
    }

    #[test]
    fn output_clips_integer_samples_and_keeps_silent_video_tail() {
        let pcm = Arc::new(AudioClip { sample_rate: 2, channels: 1, samples: vec![2.0; 8] });
        let playback = playback(vec![voice(1, Some(pcm), 0.0, true, 1.0), voice(2, None, 0.0, true, 3.0)], 2);
        let mut output = [0i16; 4];
        write_output(&mut output, &playback);
        assert_eq!(output[0..2], [i16::MAX, i16::MAX]);
        assert_eq!(output[2..], [0, 0]);
        assert!(!playback.voices[0].playing.load(Ordering::Relaxed));
        assert!(playback.voices[1].playing.load(Ordering::Relaxed));
        assert_eq!(playback.voices[1].frame.load(Ordering::Relaxed), 4);
        let mut tail = [1i16; 2];
        write_output(&mut tail, &playback);
        assert_eq!(tail, [0, 0]);
        assert_eq!(playback.voices[1].frame.load(Ordering::Relaxed), 6);
        assert!(!playback.voices[1].playing.load(Ordering::Relaxed));
    }

    #[test]
    fn voices_share_pcm_but_keep_independent_positions_and_pause_state() {
        let pcm = Arc::new(AudioClip { sample_rate: 1, channels: 1, samples: vec![0.1, 0.2, 0.3, 0.4] });
        let voices = vec![voice(1, Some(pcm.clone()), 0.0, true, 4.0), voice(2, Some(pcm.clone()), 2.0, true, 4.0)];
        assert!(validate_voices(&voices).is_ok());
        assert!(voices[0].clip.as_ref().zip(voices[1].clip.as_ref()).is_some_and(|(a, b)| Arc::ptr_eq(a, b)));
        let playback = playback(voices, 1);
        let mut output = [0.0f32; 2];
        write_output(&mut output, &playback);
        assert_eq!(output, [0.4, 0.6]);
        assert_eq!(playback.voices[0].frame.load(Ordering::Relaxed), 2);
        assert_eq!(playback.voices[1].frame.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn voice_end_loop_mute_and_global_pause_are_independent() {
        let pcm = Arc::new(AudioClip { sample_rate: 4, channels: 1, samples: vec![0.4; 16] });
        let mut looping = voice(1, Some(pcm.clone()), 0.5, true, 1.0);
        looping.state.looped = true;
        let stopped = voice(2, Some(pcm), 0.5, true, 1.0);
        let playback = playback(vec![looping, stopped], 4);
        let mut start = [VoiceSnapshot::default(); MAX_VOICES];
        start[0] = VoiceSnapshot { frame: 2, duration_frames: 4, playing: true, muted: true, looped: true, completed_loops: 0 };
        start[1] = VoiceSnapshot { frame: 2, duration_frames: 4, playing: true, muted: false, looped: false, completed_loops: 0 };
        assert!(commit_callback(&playback, 0, &start, 6, true));
        assert_eq!(playback.voices[0].frame.load(Ordering::Relaxed), 0);
        assert_eq!(playback.voices[0].completed_loops.load(Ordering::Relaxed), 2);
        assert_eq!(playback.voices[1].frame.load(Ordering::Relaxed), 4);
        assert!(!playback.voices[1].playing.load(Ordering::Relaxed));
        let held = playback.voices[0].frame.load(Ordering::Relaxed);
        assert!(!commit_callback(&playback, 2, &start, 6, false));
        assert_eq!(playback.voices[0].frame.load(Ordering::Relaxed), held);
    }

    #[test]
    fn muted_voice_advances_silently_and_global_pause_freezes_all_voices() {
        let pcm = Arc::new(AudioClip { sample_rate: 2, channels: 1, samples: vec![0.5; 8] });
        let mut muted = voice(1, Some(pcm.clone()), 0.0, true, 4.0);
        muted.state.muted = true;
        let playback = playback(vec![muted, voice(2, Some(pcm), 0.0, true, 4.0)], 2);
        let mut output = [0.0f32; 2];
        write_output(&mut output, &playback);
        assert_eq!(output, [0.5, 0.5]);
        assert_eq!(playback.voices[0].frame.load(Ordering::Relaxed), 2);
        playback.playing.store(false, Ordering::Relaxed);
        let before = playback.voices[1].frame.load(Ordering::Relaxed);
        write_output(&mut output, &playback);
        assert_eq!(output, [0.0, 0.0]);
        assert_eq!(playback.voices[1].frame.load(Ordering::Relaxed), before);
    }

    #[test]
    fn stale_callback_is_rejected_and_silent_voice_validates_with_shared_output() {
        let pcm = Arc::new(AudioClip { sample_rate: 4, channels: 1, samples: vec![0.1; 8] });
        let voices = vec![voice(1, Some(pcm), 0.0, true, 1.0), voice(2, None, 0.0, true, 2.0)];
        assert!(validate_voices(&voices).is_ok());
        assert!(validate_voices(&[voice(1, None, 0.0, true, 1.0)]).is_err());
        let playback = playback(voices, 4);
        update_playback(&playback, || playback.voices[0].frame.store(1, Ordering::Relaxed));
        let mut stale = [VoiceSnapshot::default(); MAX_VOICES];
        stale[0] = VoiceSnapshot { frame: 0, duration_frames: 4, playing: true, muted: false, looped: false, completed_loops: 0 };
        assert!(!commit_callback(&playback, 0, &stale, 2, true));
        assert_eq!(playback.voices[0].frame.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn sub_sample_duration_never_produces_zero_loop_modulus() {
        assert_eq!(duration_frames(0.001, 2), 1);
        let pcm = Arc::new(AudioClip { sample_rate: 2, channels: 1, samples: vec![0.3; 2] });
        let mut looping = voice(1, Some(pcm), 0.0, true, 0.001);
        looping.state.looped = true;
        let playback = playback(vec![looping], 2);
        let mut start = [VoiceSnapshot::default(); MAX_VOICES];
        start[0] = VoiceSnapshot { frame: 0, duration_frames: 1, playing: true, muted: false, looped: true, completed_loops: 0 };
        assert!(commit_callback(&playback, 0, &start, 4, true));
        assert_eq!(playback.voices[0].completed_loops.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn voice_membership_validation_rejects_duplicate_ids_invalid_times_and_overflow() {
        let pcm = Arc::new(AudioClip { sample_rate: 1, channels: 1, samples: vec![0.1, 0.2] });
        assert!(validate_voices(&[voice(1, Some(pcm.clone()), 0.0, true, 1.0), voice(1, None, 0.0, true, 1.0)]).is_err());
        assert!(validate_voices(&[voice(1, Some(pcm.clone()), f64::NAN, true, 1.0)]).is_err());
        assert!(validate_voices(&[voice(1, Some(pcm.clone()), 0.0, true, f64::INFINITY)]).is_err());
        let too_many = (1..=MAX_VOICES + 1).map(|id| voice(id as u64, Some(pcm.clone()), 0.0, true, 1.0)).collect::<Vec<_>>();
        assert!(validate_voices(&too_many).is_err());
    }
}
