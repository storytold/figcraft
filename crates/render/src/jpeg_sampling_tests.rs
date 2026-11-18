//! A progressive JPEG whose frame samples a component more finely than its first is refused from
//! its header, not handed to the decoder: zune-jpeg 0.5.15, the decoder behind `image`, asserts
//! in its upsampler on such a frame (etemesi254/zune-image#236), and in the browser a panic aborts
//! the page. Baseline frames of the same shape are decoded by the decoder itself and left to it.

use crate::images;

// The fixtures live with the header check in `figcraft-format` (`jpeg.rs`), which tests it there.
include!("../../format/src/jpeg_fixtures.rs");

#[test]
fn the_jpeg_as_written_decodes() {
    let p = images::decode(TINY_PROGRESSIVE).expect("decoded");
    assert_eq!((p.width(), p.height()), (8, 8));
}

/// Without the check this test panics in the decoder (an assertion in zune-jpeg's upsampler).
#[test]
fn a_progressive_jpeg_sampling_a_component_finer_than_its_first_is_refused_from_its_header() {
    for hv in [0x22, 0x21, 0x12] {
        assert!(images::decode(&with_second_sampled(hv)).is_none(), "second component {hv:#x}");
    }
    let mut third = TINY_PROGRESSIVE.to_vec();
    third[SECOND_SAMPLING + 3] = 0x22;
    assert!(images::decode(&third).is_none());
}

#[test]
fn baseline_frames_and_other_files_are_left_to_the_decoder() {
    // A well-formed baseline frame with its black sampled finer than its cyan: the decoder takes it.
    let p = images::decode(TINY_BASELINE_CMYK).expect("decoded");
    assert_eq!((p.width(), p.height()), (8, 8));
    assert!(images::decode(&TINY_PROGRESSIVE[..19]).is_none());
}
