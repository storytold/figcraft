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
fn rejects_audio_track_and_invalid_limits() {
    assert!(matches!(VideoSource::open(fixture("aac")), Err(VideoError::Unsupported(_))));
    assert_eq!(validate_source(&[]), Err(VideoError::Empty));
    assert_eq!(validate_source(&vec![0; MAX_SOURCE_BYTES + 1]), Err(VideoError::SourceTooLarge));
    let mut source = VideoSource::open(fixture("silent")).expect("fixture");
    assert_eq!(source.frame_at(f64::NAN), Err(VideoError::InvalidTime));
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
    stsz.extend_from_slice(&1801u32.to_be_bytes());
    assert!(matches!(preflight(&mp4_with_table(box32(b"stsz", &stsz))), Err(VideoError::Unsupported(_))));
}

#[test]
fn preflight_bounds_aggregate_sample_tables_across_tracks() {
    let mut stsz = vec![0; 4];
    stsz.extend_from_slice(&1u32.to_be_bytes());
    stsz.extend_from_slice(&1000u32.to_be_bytes());
    let mut tables = box32(b"stsz", &stsz);
    tables.extend_from_slice(&box32(b"stsz", &stsz));
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
fn delayed_dpb_flush_preserves_requested_picture_outside_four_frame_cache() {
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
    assert!(video.cache.len() <= MAX_CACHED_FRAMES);
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
