//! A progressive JPEG whose frame samples a component more finely than its first is refused from
//! its header, not handed to the decoder (zune-jpeg 0.5.15 asserts on it, and in the browser a
//! panic aborts the page): here for the header check itself and for the pictures files carry.

use std::io::Cursor;

use crate::jpeg::finer_than_first;
use crate::thumbnail::{archive_preview, decode_thumbnail};

include!("jpeg_fixtures.rs");

#[test]
fn the_jpeg_as_written_is_not_finer_than_its_first() {
    assert_eq!(TINY_PROGRESSIVE[SECOND_SAMPLING], 0x11);
    assert!(!finer_than_first(TINY_PROGRESSIVE));
    // Chroma sampled coarser than luma (the usual way round) isn't finer either.
    assert!(!finer_than_first(&{
        let mut b = TINY_PROGRESSIVE.to_vec();
        b[SECOND_SAMPLING - 3] = 0x22;
        b
    }));
}

#[test]
fn a_progressive_jpeg_sampling_a_component_finer_than_its_first_is_caught_from_its_header() {
    for hv in [0x22, 0x21, 0x12] {
        assert!(finer_than_first(&with_second_sampled(hv)), "second component {hv:#x}");
    }
    let mut third = TINY_PROGRESSIVE.to_vec();
    third[SECOND_SAMPLING + 3] = 0x22;
    assert!(finer_than_first(&third));
}

#[test]
fn baseline_frames_and_other_files_are_not_caught() {
    // A well-formed baseline frame with its black sampled finer than its cyan: the decoder takes it.
    assert!(!finer_than_first(TINY_BASELINE_CMYK));
    // Not JPEGs, or JPEGs that end before their frame.
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([1, 2, 3, 255])).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    assert!(!finer_than_first(&png));
    assert!(!finer_than_first(&TINY_PROGRESSIVE[..19]));
    assert!(!finer_than_first(&[0xFF, 0xD8]));
    assert!(!finer_than_first(&[]));
}

/// Without the check in `decode_thumbnail` this test panics in the decoder (an assertion in
/// zune-jpeg's upsampler): a crafted file's thumbnail, on the home screen and in recent files.
#[test]
fn a_thumbnail_sampling_a_component_finer_than_its_first_gives_no_picture() {
    for hv in [0x22, 0x21, 0x12] {
        assert_eq!(decode_thumbnail(&with_second_sampled(hv), 64), None, "second component {hv:#x}");
    }
    let mut zip = Cursor::new(Vec::new());
    let mut z = zip::ZipWriter::new(&mut zip);
    z.start_file("thumbnail.png", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut z, &with_second_sampled(0x22)).unwrap();
    z.finish().unwrap();
    assert_eq!(archive_preview(Cursor::new(zip.into_inner()), 64).picture, None);
    // The well-formed ones still give their picture.
    for good in [TINY_PROGRESSIVE, TINY_BASELINE_CMYK] {
        let t = decode_thumbnail(good, 64).expect("decoded");
        assert_eq!((t.width, t.height), (8, 8));
    }
}
