//! Each page keeps its view in a session: going back to a page shows it where it was left, not
//! fitted again (a new page is still fitted); going to the page shown changes nothing.

use serde_json::{Value, json};

use crate::Session;

fn view(s: &mut Session) -> (f64, f64, f64) {
    let v: Value = s.execute("view.get", &json!({})).unwrap();
    (v["cx"].as_f64().unwrap(), v["cy"].as_f64().unwrap(), v["zoom"].as_f64().unwrap())
}

#[test]
fn going_back_to_a_page_shows_it_where_it_was_left() {
    let mut s = Session::new();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap();
    let first = s.page().unwrap();
    s.execute("view.set", &json!({"zoom": 2.5, "cx": 1000, "cy": 500})).unwrap();
    let second = crate::params::id_of(&s.execute("page.add", &json!({})).unwrap()["id"]).unwrap();
    assert_ne!(view(&mut s), (1000.0, 500.0, 2.5), "a new page is fitted, not shown at the last page's view");
    s.execute("shape.ellipse", &json!({"x": 5000, "y": 5000, "width": 100, "height": 100})).unwrap();
    s.execute("view.set", &json!({"zoom": 0.5, "cx": -200, "cy": 40})).unwrap();

    s.execute("page.select", &json!({"id": first})).unwrap();
    assert_eq!(view(&mut s), (1000.0, 500.0, 2.5), "the first page as it was left");
    s.execute("view.nextPage", &json!({})).unwrap();
    assert_eq!(s.page().unwrap(), second);
    assert_eq!(view(&mut s), (-200.0, 40.0, 0.5), "the second page as it was left");
    // The page shown: nothing moves.
    s.execute("view.set", &json!({"zoom": 3.0, "cx": 7, "cy": 8})).unwrap();
    s.execute("page.select", &json!({"id": second})).unwrap();
    assert_eq!(view(&mut s), (7.0, 8.0, 3.0));
    s.execute("view.prevPage", &json!({})).unwrap();
    assert_eq!(view(&mut s), (1000.0, 500.0, 2.5));
}
