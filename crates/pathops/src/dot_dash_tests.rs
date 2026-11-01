use super::*;

#[test]
fn dense_round_dashes_fall_back_before_expansion() {
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((10_000.0, 0.0));
    let dangerous = StrokeProps {
        weight: 2.0,
        dashes: vec![0.0, 0.02],
        cap_start: StrokeCap::Round,
        cap_end: StrokeCap::Round,
        align: StrokeAlign::Center,
        ..Default::default()
    };
    let outlined = stroke_center(&line, 2.0, &dangerous);
    let solid = stroke_center(&line, 2.0, &StrokeProps { dashes: vec![], ..dangerous });
    assert!((outlined.area().abs() - solid.area().abs()).abs() < 0.01);
    assert!(outlined.elements().len() < 1000, "fallback must not emit hundreds of thousands of dots");
}

#[test]
fn zero_length_round_dashes_draw_dots() {
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((100.0, 0.0));
    let round = StrokeProps {
        weight: 2.0,
        dashes: vec![0.0, 9.0],
        cap_start: StrokeCap::Round,
        cap_end: StrokeCap::Round,
        align: StrokeAlign::Center,
        ..Default::default()
    };
    let dots = stroke_center(&line, 2.0, &round);
    let area = dots.area().abs();
    assert!((area - 12.0 * std::f64::consts::PI).abs() < 1.5, "{area}");
    let butt = StrokeProps { cap_start: StrokeCap::None, cap_end: StrokeCap::None, ..round };
    assert!(stroke_center(&line, 2.0, &butt).area().abs() < 1e-6);
}
