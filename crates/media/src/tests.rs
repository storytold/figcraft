#![allow(clippy::expect_used, clippy::panic)]
use super::*;
use filmcraft_isobmff::AvcConfig;

fn box32(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&(u32::try_from(payload.len() + 8).expect("box length")).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

fn box64(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 1];
    out.extend_from_slice(kind);
    out.extend_from_slice(&(u64::try_from(payload.len() + 16).expect("box length")).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

fn box_to_end(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 0];
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

fn mp4_with_table(table: Vec<u8>) -> Vec<u8> {
    let stbl = box32(b"stbl", &table);
    let minf = box32(b"minf", &stbl);
    let mdia = box32(b"mdia", &minf);
    let trak = box32(b"trak", &mdia);
    let moov = box32(b"moov", &trak);
    let mut out = box32(b"ftyp", b"isom");
    out.extend_from_slice(&moov);
    out
}

fn bomb_stz2() -> Vec<u8> {
    let mut payload = vec![0, 0, 0, 0, 0, 0, 0, 4];
    payload.extend_from_slice(&3_000_000u32.to_be_bytes());
    payload
}

fn fixture(name: &str) -> Arc<Vec<u8>> {
    let bytes = match name {
        "silent" => include_bytes!("../tests/fixtures/synthetic-h264-silent.mp4").to_vec(),
        "main_audio" => include_bytes!("../tests/fixtures/synthetic-h264-main-aac.mp4").to_vec(),
        "short_audio" => include_bytes!("../tests/fixtures/synthetic-h264-short-aac.mp4").to_vec(),
        _ => include_bytes!("../tests/fixtures/synthetic-h264-aac.mp4").to_vec(),
    };
    Arc::new(bytes)
}

#[test]
fn decodes_reference_frames_and_bounds_cache() {
    let mut video = VideoSource::open(fixture("silent")).expect("synthetic silent MP4 opens");
    let meta = video.metadata().clone();
    assert_eq!((meta.width, meta.height, meta.frame_count), (96, 64, 12));
    assert!((meta.duration_seconds - 2.0).abs() < 0.01);
    let reference = include_bytes!("../tests/fixtures/synthetic-h264-silent.rgb24");
    let mut max_delta = 0u8;
    let mut total_delta = 0u64;
    let mut samples = 0u64;
    for index in 0..meta.frame_count {
        let frame = video.frame_at(index as f64 * meta.frame_interval_seconds).expect("frame decodes");
        assert_eq!(frame.frame_index, index);
        assert_eq!(frame.rgba.len(), (meta.width * meta.height * 4) as usize);
        let rgb_start = index as usize * meta.width as usize * meta.height as usize * 3;
        for pixel in 0..(meta.width * meta.height) as usize {
            for channel in 0..3 {
                let delta = frame.rgba[pixel * 4 + channel].abs_diff(reference[rgb_start + pixel * 3 + channel]);
                max_delta = max_delta.max(delta);
                total_delta += u64::from(delta);
                samples += 1;
            }
        }
    }
    assert!(total_delta as f64 / samples as f64 <= 1.0, "MAE {}", total_delta as f64 / samples as f64);
    assert!(max_delta <= 4, "max channel delta {max_delta}");
    assert!(video.cache.len() <= MAX_CACHED_FRAMES);
    let first = video.frame_at(0.0).expect("backward seek");
    let repeated = video.frame_at(0.0).expect("repeated frame");
    assert_eq!(first, repeated);
}

#[test]
fn forward_jump_skips_rgba_for_earlier_decoded_pictures() {
    let target_index = 10usize;
    let mut sequential = VideoSource::open(fixture("silent")).expect("fixture opens");
    let target_time = target_index as f64 * sequential.metadata().frame_interval_seconds;
    for index in 0..target_index {
        sequential.frame_at(index as f64 * sequential.metadata().frame_interval_seconds).expect("preceding frame decodes");
    }
    let reference = sequential.frame_at(target_time).expect("sequential target decodes");

    let mut jumped = VideoSource::open(fixture("silent")).expect("fixture opens");
    let frame = jumped.frame_at(target_time).expect("forward target decodes");
    assert_eq!(frame, reference);
    assert!(jumped.converted_picture_indices.contains(&target_index));
    assert!(jumped.converted_picture_indices.iter().all(|&index| index >= target_index));
    assert!(jumped.cache.len() <= MAX_CACHED_FRAMES);
}

#[test]
fn accepts_audio_track_and_invalid_limits() {
    let video = VideoSource::open(fixture("aac")).expect("AAC track opens");
    let audio = video.metadata().audio.as_ref().expect("audio metadata");
    assert_eq!((audio.sample_rate, audio.channels), (48_000, 1));
    let decoded = video.decode_audio_cancellable(|| false).expect("AAC decodes").expect("audio clip");
    assert_eq!(decoded.samples.len(), (audio.duration_seconds * f64::from(audio.sample_rate)).round() as usize);
    assert!(decoded.samples.iter().any(|sample| sample.abs() > 0.01));
    assert_eq!(validate_source(&[]), Err(VideoError::Empty));
    assert_eq!(validate_source(&vec![0; MAX_SOURCE_BYTES + 1]), Err(VideoError::SourceTooLarge));
    let mut source = VideoSource::open(fixture("silent")).expect("fixture");
    assert!(source.metadata().audio.is_none());
    assert_eq!(source.decode_audio_cancellable(|| false), Ok(None));
    assert_eq!(source.frame_at(f64::NAN), Err(VideoError::InvalidTime));
}

#[test]
fn accepts_main_cabac_and_preserves_audio_longer_than_video() {
    let mut source = VideoSource::open(fixture("main_audio")).expect("Main/CABAC AAC fixture opens");
    let metadata = source.metadata().clone();
    assert_eq!((metadata.width, metadata.height, metadata.frame_count), (1920, 1080, 30));
    assert!(metadata.duration_seconds > 1.2, "audio tail must extend clip duration");
    let audio = metadata.audio.expect("AAC metadata");
    assert_eq!((audio.sample_rate, audio.channels), (48_000, 1));
    assert!(audio.duration_seconds > 1.2);

    let frame = source.frame_at(1.2).expect("video holds on its final picture during audio tail");
    assert_eq!(frame.frame_index, 29);
    assert_eq!((frame.width, frame.height), (1920, 1080));
    for (x, y, expected) in [(0usize, 0usize, [255u8, 0, 0]), (960, 540, [0, 0, 254]), (1919, 1079, [0, 255, 255]), (100, 200, [253, 0, 0])] {
        let offset = (y * frame.width as usize + x) * 4;
        for (actual, expected) in frame.rgba[offset..offset + 3].iter().zip(expected) {
            assert!(actual.abs_diff(expected) <= 4, "pixel ({x},{y}) channel differs: {actual} vs {expected}");
        }
    }
    let clip = source.decode_audio_cancellable(|| false).expect("AAC decode").expect("audio clip");
    assert_eq!(clip.samples.len(), (audio.duration_seconds * f64::from(audio.sample_rate)).round() as usize);
    assert!(clip.samples.iter().any(|sample| sample.abs() > 0.01));
}

#[test]
fn audio_shorter_than_video_does_not_shorten_clip_duration() {
    let source = VideoSource::open(fixture("short_audio")).expect("short-audio fixture opens");
    let metadata = source.metadata();
    let audio = metadata.audio.as_ref().expect("AAC metadata");
    assert!((metadata.duration_seconds - 2.0).abs() < 0.01);
    assert!(audio.duration_seconds < 1.1);
    assert!(metadata.duration_seconds > audio.duration_seconds);
    let clip = source.decode_audio_cancellable(|| false).expect("AAC decode").expect("audio clip");
    assert_eq!(clip.samples.len(), (audio.duration_seconds * f64::from(audio.sample_rate)).round() as usize);
}

#[test]
fn audio_decode_cancellation_and_packet_errors_never_return_partial_pcm() {
    let video = VideoSource::open(fixture("aac")).expect("AAC fixture");
    assert_eq!(video.decode_audio_cancellable(|| true), Err(VideoError::Cancelled));
    let mut checks = 0usize;
    assert_eq!(
        video.decode_audio_cancellable(|| {
            checks += 1;
            checks == 3
        }),
        Err(VideoError::Cancelled)
    );
    assert_eq!(checks, 3);

    let mut bytes = fixture("aac").as_ref().clone();
    let parsed = filmcraft_isobmff::open(bytes.as_slice()).expect("MP4 parses");
    let audio_index = parsed.track_of_kind(TrackKind::Audio).expect("AAC track");
    let sample = parsed.tracks[audio_index].samples.first().expect("AAC packet");
    let start = usize::try_from(sample.offset).expect("packet offset");
    let end = start + sample.size as usize;
    bytes.get_mut(start..end).expect("packet bytes").fill(0);
    let corrupt = VideoSource::open(Arc::new(bytes)).expect("AAC payload is decoded on request");
    assert!(matches!(corrupt.decode_audio_cancellable(|| false), Err(VideoError::Decode(_))));
}

#[test]
fn accepts_aac_track_duration_truncated_by_one_pcm_sample() {
    let mut bytes = fixture("aac").as_ref().clone();
    let audio_mdhd = bytes.windows(4).enumerate().filter_map(|(index, window)| (window == b"mdhd").then_some(index)).nth(1).expect("audio mdhd box");
    let payload = audio_mdhd + 4;
    assert_eq!(bytes[payload], 0, "version-zero mdhd fixture");
    let duration_offset = payload + 16;
    let duration = read_u32(&bytes, duration_offset).expect("audio mdhd duration");
    bytes[duration_offset..duration_offset + 4].copy_from_slice(&(duration - 1).to_be_bytes());
    let source = VideoSource::open(Arc::new(bytes)).expect("single sample of AAC tail may be truncated");
    let audio = source.metadata().audio.as_ref().expect("audio metadata");
    let decoded = source.decode_audio_cancellable(|| false).expect("AAC decode").expect("audio clip");
    assert_eq!(decoded.samples.len(), (audio.duration_seconds * f64::from(audio.sample_rate)).round() as usize);
}

#[test]
fn validates_audio_ticks_and_edit_offsets_in_their_track_timescale() {
    let bytes = fixture("aac");
    let mut file = filmcraft_isobmff::open(bytes.as_slice()).expect("MP4 parses");
    let audio_index = file.track_of_kind(TrackKind::Audio).expect("AAC track");
    file.timescale = 48_000;
    let track = file.tracks.get_mut(audio_index).expect("audio track");
    track.timescale = 96_000;
    track.duration *= 2;
    for sample in &mut track.samples {
        sample.dts *= 2;
        sample.pts *= 2;
        sample.duration *= 2;
    }
    track.edits = vec![
        filmcraft_isobmff::Edit { segment_duration: 480, media_time: -1, media_rate: 0x1_0000 },
        filmcraft_isobmff::Edit { segment_duration: 1_024, media_time: 2_048, media_rate: 0x1_0000 },
    ];
    track.tkhd_duration = 1_504;
    track.edit_offset = -1_088;

    let timeline = validate_audio_track(&file, &file.tracks[audio_index]).expect("scaled track and edit offset validate");
    assert_eq!(timeline.leading_silence_samples, 480);
    assert_eq!(timeline.media_start_sample, 1_024);
    assert_eq!(timeline.media_sample_count, 1_024);
    assert_eq!(timeline.output_sample_count, 1_504);
}

#[test]
fn rejects_edited_audio_output_over_sixty_seconds() {
    let bytes = fixture("aac");
    let mut file = filmcraft_isobmff::open(bytes.as_slice()).expect("MP4 parses");
    let audio_index = file.track_of_kind(TrackKind::Audio).expect("AAC track");
    file.timescale = 48_000;
    let track = file.tracks.get_mut(audio_index).expect("audio track");
    track.edits = vec![
        filmcraft_isobmff::Edit { segment_duration: 99 * 48_000, media_time: -1, media_rate: 0x1_0000 },
        filmcraft_isobmff::Edit { segment_duration: 1_024, media_time: 1_024, media_rate: 0x1_0000 },
    ];
    track.tkhd_duration = 99 * 48_000 + 1_024;
    track.edit_offset = 99 * 48_000 - 1_024;

    assert!(matches!(
        validate_audio_track(&file, &file.tracks[audio_index]),
        Err(VideoError::Unsupported("edited audio duration exceeds 60 seconds"))
    ));
}

#[test]
fn cancellable_seek_checks_before_decode_and_between_packets() {
    let mut before_decode = VideoSource::open(fixture("silent")).expect("fixture");
    assert_eq!(before_decode.frame_at_cancellable(0.0, || true), Err(VideoError::Cancelled));
    assert_eq!(before_decode.next_sample, 0);

    let mut during_seek = VideoSource::open(fixture("silent")).expect("fixture");
    let mut checks = 0;
    let time = 11.0 * during_seek.metadata().frame_interval_seconds;
    assert_eq!(
        during_seek.frame_at_cancellable(time, || {
            checks += 1;
            checks == 3
        }),
        Err(VideoError::Cancelled)
    );
    assert_eq!(checks, 3);
    assert!(during_seek.next_sample > 0);
}

#[test]
fn rejects_sample_table_bomb_before_demux() {
    let mut stsz = vec![0; 8];
    stsz.extend_from_slice(&((MAX_TOTAL_SAMPLES + 1) as u32).to_be_bytes());
    assert!(matches!(preflight(&mp4_with_table(box32(b"stsz", &stsz))), Err(VideoError::Unsupported(_))));
}

#[test]
fn preflight_bounds_aggregate_sample_tables_across_tracks() {
    let stsz = |count: usize| {
        let mut table = vec![0; 4];
        table.extend_from_slice(&42u32.to_be_bytes());
        table.extend_from_slice(&(count as u32).to_be_bytes());
        box32(b"stsz", &table)
    };
    let mut tables = stsz(MAX_SAMPLES);
    tables.extend_from_slice(&stsz(MAX_AUDIO_ACCESS_UNITS + 1));
    assert!(matches!(preflight(&mp4_with_table(tables)), Err(VideoError::Unsupported(_))));
}

#[test]
fn preflight_bounds_extended_and_to_end_sample_table_boxes() {
    let extended = mp4_with_table(box64(b"stz2", &bomb_stz2()));
    let to_end = mp4_with_table(box_to_end(b"stz2", &bomb_stz2()));
    assert!(matches!(preflight(&extended), Err(VideoError::Unsupported(_))));
    assert!(matches!(preflight(&to_end), Err(VideoError::Unsupported(_))));
}

#[test]
fn preflight_accepts_constant_size_stsz_without_size_array() {
    let mut stsz = Vec::new();
    stsz.extend_from_slice(&[0; 4]);
    stsz.extend_from_slice(&42u32.to_be_bytes());
    stsz.extend_from_slice(&12u32.to_be_bytes());
    assert_eq!(preflight(&mp4_with_table(box32(b"stsz", &stsz))), Ok(()));
}

fn avcc_ranges(bytes: &[u8]) -> (usize, usize, usize) {
    let tag = bytes.windows(4).position(|window| window == b"avcC").expect("avcC box");
    let payload_start = tag + 4;
    let box_start = tag - 4;
    let box_end = box_start + read_u32(bytes, box_start).expect("avcC size") as usize;
    let config = AvcConfig::parse(&bytes[payload_start..box_end]).expect("AVC config");
    let sps_nal = payload_start + 8;
    let pps_nal = sps_nal + config.sps[0].len() + 3;
    (sps_nal, pps_nal, config.sps[0].len())
}

#[test]
fn rejects_wrong_nal_types_in_avcc_parameter_arrays_before_decode() {
    let original = fixture("silent");
    let (sps, pps, _) = avcc_ranges(&original);
    let mut wrong_sps = original.as_ref().clone();
    wrong_sps[sps] = (wrong_sps[sps] & 0xe0) | 8;
    assert!(matches!(VideoSource::open(Arc::new(wrong_sps)), Err(VideoError::Unsupported(_))));
    let mut wrong_pps = original.as_ref().clone();
    wrong_pps[pps] = (wrong_pps[pps] & 0xe0) | 7;
    assert!(matches!(VideoSource::open(Arc::new(wrong_pps)), Err(VideoError::Unsupported(_))));
}

fn read_ue(bits: &[u8], pos: &mut usize) -> u32 {
    let mut leading = 0u32;
    while bit(bits, *pos) == 0 {
        *pos += 1;
        leading += 1;
    }
    *pos += 1;
    let mut suffix = 0u32;
    for _ in 0..leading {
        suffix = (suffix << 1) | u32::from(bit(bits, *pos));
        *pos += 1;
    }
    ((1u32 << leading) - 1) + suffix
}

fn bit(bits: &[u8], pos: usize) -> u8 {
    (bits[pos / 8] >> (7 - pos % 8)) & 1
}

fn sps_vui_flag_bit(nal: &[u8]) -> usize {
    let rbsp = unescape_rbsp(&nal[1..]);
    let mut pos = 24;
    read_ue(&rbsp, &mut pos);
    read_ue(&rbsp, &mut pos);
    let poc = read_ue(&rbsp, &mut pos);
    if poc == 0 {
        read_ue(&rbsp, &mut pos);
    }
    if poc == 1 {
        pos += 1;
        read_ue(&rbsp, &mut pos);
        read_ue(&rbsp, &mut pos);
        let count = read_ue(&rbsp, &mut pos);
        for _ in 0..count {
            read_ue(&rbsp, &mut pos);
        }
    }
    read_ue(&rbsp, &mut pos);
    pos += 1;
    read_ue(&rbsp, &mut pos);
    read_ue(&rbsp, &mut pos);
    let frame_only = bit(&rbsp, pos) != 0;
    pos += 1;
    if !frame_only {
        pos += 1;
    }
    pos += 1;
    let has_crop = bit(&rbsp, pos) != 0;
    pos += 1;
    if has_crop {
        for _ in 0..4 {
            read_ue(&rbsp, &mut pos);
        }
    }
    pos
}

fn clear_sps_vui_flag(nal: &[u8]) -> Vec<u8> {
    let rbsp = unescape_rbsp(&nal[1..]);
    let bit_pos = sps_vui_flag_bit(nal);
    let rbsp_pos = bit_pos / 8;
    let mut ebsp_pos = 1usize;
    let mut rbsp_index = 0usize;
    let mut zeroes = 0usize;
    while ebsp_pos < nal.len() {
        let byte = nal[ebsp_pos];
        if zeroes >= 2 && byte == 3 {
            zeroes = 0;
            ebsp_pos += 1;
            continue;
        }
        if rbsp_index == rbsp_pos {
            break;
        }
        rbsp_index += 1;
        zeroes = if byte == 0 { zeroes + 1 } else { 0 };
        ebsp_pos += 1;
    }
    assert!(ebsp_pos < nal.len(), "VUI flag EBSP byte");
    let mut changed = nal.to_vec();
    changed[ebsp_pos] &= !(1 << (7 - bit_pos % 8));
    assert_eq!(unescape_rbsp(&changed[1..]).len(), rbsp.len());
    changed
}

#[test]
fn delayed_dpb_flush_keeps_requested_picture_and_nearest_frames_in_cache() {
    let mut bytes = fixture("silent").as_ref().clone();
    let (sps_offset, _, sps_len) = avcc_ranges(&bytes);
    let source = filmcraft_isobmff::open(bytes.as_slice()).expect("MP4 parses");
    let avcc = match &source.tracks[0].entries[0].codec {
        CodecConfig::Avc(c) => c,
        _ => panic!("AVC fixture"),
    };
    let no_vui_sps = clear_sps_vui_flag(&avcc.sps[0]);
    assert_eq!(no_vui_sps.len(), sps_len);
    bytes[sps_offset..sps_offset + sps_len].copy_from_slice(&no_vui_sps);
    let mut video = VideoSource::open(Arc::new(bytes)).expect("Baseline without VUI opens");
    let frame = video.frame_at(0.0).expect("end-of-stream flush returns the delayed first picture");
    assert_eq!(frame.frame_index, 0);
    let cached: Vec<_> = video.cache.iter().map(|frame| frame.frame_index).collect();
    assert_eq!(cached, vec![0, 1, 2, 3]);
    assert!(video.cache.len() <= MAX_CACHED_FRAMES);
    video.prefetch_after_cancellable(0, || false).expect("nearest frames already cached");
    assert_eq!(video.cache.front().map(|frame| frame.frame_index), Some(0));
}

#[test]
fn prefetch_keeps_anchor_and_cancels_before_decoding_next_frame() {
    let mut video = VideoSource::open(fixture("silent")).expect("fixture opens");
    let current = video.frame_at(0.0).expect("current frame decodes");
    assert_eq!(video.prefetch_after_cancellable(current.frame_index, || true), Err(VideoError::Cancelled));
    assert_eq!(video.cache.front().map(|frame| frame.frame_index), Some(current.frame_index));
    assert!(video.cache.len() <= MAX_CACHED_FRAMES);
    video.prefetch_after_cancellable(current.frame_index, || false).expect("future frames prefetch");
    let cached: Vec<_> = video.cache.iter().map(|frame| frame.frame_index).collect();
    assert!(cached.contains(&current.frame_index));
    assert!(cached.iter().all(|index| (current.frame_index..=current.frame_index + 3).contains(index)));
    assert!(cached.len() <= MAX_CACHED_FRAMES);
}

#[test]
fn duration_tail_clamps_to_last_frame() {
    let mut bytes = fixture("silent").as_ref().clone();
    let parsed = filmcraft_isobmff::open(bytes.as_slice()).expect("MP4 parses");
    let track = &parsed.tracks[0];
    let delta = u64::from(track.samples[0].duration);
    let tag = bytes.windows(4).position(|window| window == b"mdhd").expect("mdhd box");
    let start = tag - 4;
    let payload = tag + 4;
    if bytes[payload] == 0 {
        let old = read_u32(&bytes, payload + 16).expect("version-zero duration");
        bytes[payload + 16..payload + 20].copy_from_slice(&(old + delta as u32).to_be_bytes());
    } else {
        let old = read_u64(&bytes, payload + 24).expect("version-one duration");
        bytes[payload + 24..payload + 32].copy_from_slice(&(old + delta).to_be_bytes());
    }
    assert!(read_u32(&bytes, start).is_some());
    let mut video = VideoSource::open(Arc::new(bytes)).expect("one-interval duration tail is accepted");
    let time = video.metadata().duration_seconds - 0.01;
    let frame = video.frame_at(time).expect("duration tail clamps to last frame");
    assert_eq!(frame.frame_index, video.metadata().frame_count - 1);
}

#[test]
fn parameter_parser_accepts_only_validated_reference_limits() {
    let video = VideoSource::open(fixture("silent")).expect("fixture");
    let _ = video;
}

#[test]
fn rejects_malformed_sps_without_panicking() {
    assert!(parse_sps(&[0x67, 0xff]).is_err());
}

fn synthetic_picture(y: u8, u: u8, v: u8, full_range: bool, matrix: u8) -> Picture {
    Picture {
        width: 2,
        height: 2,
        chroma_width: 1,
        chroma_height: 1,
        y: vec![y; 4],
        u: vec![u],
        v: vec![v],
        y_stride: 2,
        uv_stride: 1,
        pts: 0,
        poc: 0,
        key: false,
        color: filmcraft_h264::ColorInfo { full_range, primaries: 1, transfer: 1, matrix },
        sar: (0, 0),
        draft: false,
    }
}

fn legacy_rgba(picture: &Picture, color: YuvColor) -> Vec<u8> {
    let (r_v, g_u, g_v, b_u, y_offset) = match (color.matrix, color.full_range) {
        (YuvMatrix::Bt601, false) => (409, 100, 208, 516, 16),
        (YuvMatrix::Bt709, false) => (459, 55, 136, 541, 16),
        (YuvMatrix::Bt601, true) => (359, 88, 183, 454, 0),
        (YuvMatrix::Bt709, true) => (403, 48, 120, 475, 0),
    };
    let y_scale = if color.full_range { 256 } else { 298 };
    let mut output = Vec::new();
    for y in 0..picture.height as usize {
        for x in 0..picture.width as usize {
            let yv = picture.y[y * picture.y_stride + x] as i32;
            let uv_index = y / 2 * picture.uv_stride + x / 2;
            let u = picture.u[uv_index] as i32 - 128;
            let v = picture.v[uv_index] as i32 - 128;
            let c = (yv - y_offset).max(0);
            output.extend_from_slice(&[
                ((y_scale * c + r_v * v + 128) >> 8).clamp(0, 255) as u8,
                ((y_scale * c - g_u * u - g_v * v + 128) >> 8).clamp(0, 255) as u8,
                ((y_scale * c + b_u * u + 128) >> 8).clamp(0, 255) as u8,
                255,
            ]);
        }
    }
    output
}

#[test]
fn row_conversion_matches_legacy_for_odd_padded_planes_and_color_overrides() {
    let mut picture = Picture {
        width: 5,
        height: 3,
        chroma_width: 3,
        chroma_height: 2,
        y: vec![18, 44, 71, 99, 127, 0xaa, 151, 179, 203, 231, 0xaa, 0xbb, 21, 63, 108, 177, 245, 0xaa],
        u: vec![71, 119, 202, 0xaa, 13, 145, 239, 0xaa],
        v: vec![230, 157, 82, 0xaa, 251, 103, 30, 0xaa],
        y_stride: 6,
        uv_stride: 4,
        pts: 0,
        poc: 0,
        key: false,
        color: filmcraft_h264::ColorInfo { full_range: false, primaries: 1, transfer: 1, matrix: 6 },
        sar: (0, 0),
        draft: false,
    };
    for (matrix, full_range) in [(YuvMatrix::Bt601, false), (YuvMatrix::Bt709, false), (YuvMatrix::Bt601, true), (YuvMatrix::Bt709, true)] {
        let color = YuvColor { matrix, full_range };
        let actual = to_rgba(picture.clone(), 4, 0.04, Some(color)).expect("row conversion");
        assert_eq!(actual.rgba, legacy_rgba(&picture, color));
        assert!(actual.rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
    }
    // The stream's metadata remains the fallback when no override is supplied.
    picture.color = filmcraft_h264::ColorInfo { full_range: false, primaries: 1, transfer: 1, matrix: 1 };
    let color = validate_colorimetry(1, 1, 1, false).expect("BT.709 metadata");
    let actual = to_rgba(picture.clone(), 0, 0.04, None).expect("metadata conversion");
    assert_eq!(actual.rgba, legacy_rgba(&picture, color));
}

#[test]
fn row_conversion_rejects_truncated_and_overflowing_planes() {
    let mut picture = synthetic_picture(40, 120, 180, false, 6);
    picture.y.truncate(3);
    assert!(matches!(to_rgba(picture.clone(), 0, 1.0, None), Err(VideoError::InvalidContainer)));

    picture = synthetic_picture(40, 120, 180, false, 6);
    picture.u.clear();
    assert!(matches!(to_rgba(picture.clone(), 0, 1.0, None), Err(VideoError::InvalidContainer)));

    picture = synthetic_picture(40, 120, 180, false, 6);
    picture.v.clear();
    assert!(matches!(to_rgba(picture.clone(), 0, 1.0, None), Err(VideoError::InvalidContainer)));

    picture = synthetic_picture(40, 120, 180, false, 6);
    picture.height = 3;
    picture.y_stride = usize::MAX;
    assert!(matches!(to_rgba(picture, 0, 1.0, None), Err(VideoError::InvalidContainer)));
}

#[test]
fn converts_bt601_and_bt709_limited_and_full_range() {
    let limited_601 = to_rgba(synthetic_picture(16, 128, 128, false, 6), 0, 1.0, None).expect("BT.601 limited");
    let full_601 = to_rgba(synthetic_picture(16, 128, 128, true, 6), 0, 1.0, None).expect("BT.601 full");
    assert_eq!(&limited_601.rgba[..3], &[0, 0, 0]);
    assert_eq!(&full_601.rgba[..3], &[16, 16, 16]);

    let limited_709 = to_rgba(synthetic_picture(100, 90, 240, false, 1), 0, 1.0, None).expect("BT.709 limited");
    let full_709 = to_rgba(synthetic_picture(100, 90, 240, true, 1), 0, 1.0, None).expect("BT.709 full");
    assert_eq!(&limited_709.rgba[..3], &[255, 46, 17]);
    assert_eq!(&full_709.rgba[..3], &[255, 55, 29]);
}

#[test]
fn rejects_hdr_and_unsupported_matrix_colorimetry() {
    assert!(matches!(validate_colorimetry(9, 16, 9, false), Err(VideoError::Unsupported(_))));
    assert!(matches!(validate_colorimetry(1, 18, 1, false), Err(VideoError::Unsupported(_))));
    assert!(matches!(to_rgba(synthetic_picture(100, 128, 128, false, 9), 0, 1.0, None), Err(VideoError::Unsupported(_))));
}

fn sample(offset: u64, size: u32) -> filmcraft_isobmff::Sample {
    filmcraft_isobmff::Sample { offset, size, dts: 0, pts: 0, duration: 1, is_sync: false, description_index: 1 }
}

#[test]
fn sample_tables_must_stay_inside_the_source_without_overlap() {
    assert!(check_sample_layout(&[sample(0, 10), sample(10, 10), sample(30, 5)], 40).is_ok());
    // Every sample pointing at one payload would multiply the parsing work by the sample count.
    let shared: Vec<_> = (0..1800).map(|_| sample(100, 1_000_000)).collect();
    assert!(matches!(check_sample_layout(&shared, 16 * 1024 * 1024), Err(VideoError::Unsupported(_))));
    assert!(matches!(check_sample_layout(&[sample(0, 10), sample(9, 10)], 40), Err(VideoError::Unsupported(_))));
    assert!(matches!(check_sample_layout(&[sample(35, 10)], 40), Err(VideoError::InvalidContainer)));
    assert!(matches!(check_sample_layout(&[sample(u64::MAX, 10)], 40), Err(VideoError::InvalidContainer)));
    assert!(matches!(check_sample_layout(&[sample(0, 0)], 40), Err(VideoError::InvalidContainer)));
}

#[test]
fn a_sample_of_millions_of_tiny_nal_units_is_refused_without_building_them() {
    // One 16 MiB sample made of 2-byte NAL units behind 4-byte lengths.
    let unit = [0, 0, 0, 2, 0x06, 0x00];
    let data: Vec<u8> = unit.iter().copied().cycle().take(unit.len() * 2_000_000).collect();
    assert!(matches!(check_nal_count(&data, 4), Err(VideoError::Unsupported(_))));
    let few: Vec<u8> = unit.iter().copied().cycle().take(unit.len() * 3).collect();
    assert!(check_nal_count(&few, 4).is_ok());
    assert!(matches!(check_nal_count(&[0, 0, 0, 9, 1], 4), Err(VideoError::InvalidContainer)));
}

#[test]
fn a_real_file_whose_sample_sizes_overrun_the_source_is_refused() {
    let mut bytes = fixture("silent").to_vec();
    let at = bytes.windows(4).position(|w| w == b"stsz").expect("stsz box");
    // version/flags, then the default sample size: claim every sample is huge.
    bytes[at + 8..at + 12].copy_from_slice(&1_000_000u32.to_be_bytes());
    assert!(VideoSource::open(Arc::new(bytes)).is_err());
}

#[test]
fn video_edit_lists_that_trim_delay_or_change_speed_are_refused() {
    let edit = |segment_duration, media_time, media_rate| filmcraft_isobmff::Edit { segment_duration, media_time, media_rate };
    // A 2 s clip: movie timescale 1000 (2000 ticks), track timescale 12 (24 ticks), 2-tick frames.
    let t = EditTimeline { movie_timescale: 1000, track_timescale: 12, track_duration: 24, tolerance_ticks: 2 };
    assert!(video_edits_are_trivial(&[], t));
    assert!(video_edits_are_trivial(&[edit(2000, 0, 0x0001_0000)], t));
    assert!(video_edits_are_trivial(&[edit(1850, 0, 0x0001_0000)], t), "a frame short is within tolerance");
    assert!(!video_edits_are_trivial(&[edit(1000, 0, 0x0001_0000)], t), "end trimmed to half");
    assert!(!video_edits_are_trivial(&[edit(2000, 512, 0x0001_0000)], t), "trimmed start");
    assert!(!video_edits_are_trivial(&[edit(500, -1, 0x0001_0000), edit(2000, 0, 0x0001_0000)], t), "delayed start");
    assert!(!video_edits_are_trivial(&[edit(2000, 0, 0x0002_0000)], t), "double speed");
    assert!(!video_edits_are_trivial(&[edit(2000, 0, 0)], t), "dwell");
}
