//! Geometry reuse must preserve the exact raster, invalidate subpixel changes and stay bounded.

use super::*;
use figcraft_geom::{BezPath, Rect, Shape};
use tiny_skia::FillRule;

fn key() -> GeometryKey {
    GeometryKey {
        shape: (Rect::new(1.25, 2.5, 29.75, 21.5).to_path(0.1), FillRule::Winding),
        xf: Affine::IDENTITY,
        w: 32,
        h: 24,
        margin: 8,
        smooth: 2,
    }
}

fn small_geometry() -> Arc<Geometry> {
    Arc::new(Geometry { coverage: vec![255], distance: vec![0.5], normals: vec![(1.0, 0.0)] })
}

#[test]
fn cached_geometry_keeps_the_exact_samples_of_the_padded_field() {
    let key = key();
    let a = geometry(key.clone()).unwrap();
    // A local cache makes the hit assertion independent of other parallel render tests evicting
    // entries from the shared cache.
    let mut cache = GeometryCache::default();
    cache.insert(key.clone(), a.clone(), MAX_GEOMETRY_BYTES);
    assert!(Arc::ptr_eq(&a, &cache.get(&key).unwrap()), "the second render must reuse geometry");
    let m = key.margin;
    let cov = spread_alpha(&[key.shape], 0.0, Affine::translate((f64::from(m), f64::from(m))) * key.xf, key.w + 2 * m, key.h + 2 * m).unwrap();
    let dist = inside_distance(&cov);
    let normals = smooth_normals(&dist, cov.width() as usize, cov.height() as usize, key.smooth);
    for y in 0..key.h as usize {
        for x in 0..key.w as usize {
            let dst = y * key.w as usize + x;
            let src = (y + m as usize) * cov.width() as usize + x + m as usize;
            assert_eq!(a.coverage[dst], cov.data()[src * 4 + 3]);
            assert_eq!(a.distance[dst], dist[src]);
            assert_eq!(a.normals[dst], normals[src]);
        }
    }
    assert_eq!(a.coverage.len(), key.w as usize * key.h as usize);
}

#[test]
fn subpixel_transform_shape_and_fill_rule_changes_invalidate_geometry() {
    let original = key();
    let mut cache = GeometryCache::default();
    cache.insert(original.clone(), small_geometry(), MAX_GEOMETRY_BYTES);
    let mut changed = original.clone();
    changed.xf = Affine::translate((0.125, 0.0));
    assert!(cache.get(&changed).is_none());
    changed = original.clone();
    changed.shape.1 = FillRule::EvenOdd;
    assert!(cache.get(&changed).is_none());
    changed = original.clone();
    changed.shape.0 = BezPath::new();
    assert!(cache.get(&changed).is_none());
    changed = original.clone();
    changed.w += 1;
    assert!(cache.get(&changed).is_none());
    changed = original.clone();
    changed.margin += 1;
    assert!(cache.get(&changed).is_none());
    changed = original.clone();
    changed.smooth += 1;
    assert!(cache.get(&changed).is_none());
    assert!(cache.get(&original).is_some());
}

#[test]
fn geometry_cache_evicts_the_least_recent_entry_within_its_byte_budget() {
    let mut cache = GeometryCache::default();
    let a = key();
    let mut b = a.clone();
    b.w += 1;
    let mut c = b.clone();
    c.w += 1;
    cache.insert(a.clone(), small_geometry(), MAX_GEOMETRY_BYTES);
    let entry_bytes = cache.bytes;
    let budget = entry_bytes * 2;
    cache.insert(b.clone(), small_geometry(), budget);
    assert!(cache.get(&a).is_some());
    cache.insert(c.clone(), small_geometry(), budget);
    assert!(cache.get(&a).is_some());
    assert!(cache.get(&b).is_none());
    assert!(cache.get(&c).is_some());
    assert_eq!(cache.bytes, budget);
    // Oversized entries bypass the cache without evicting useful geometry.
    let large = Arc::new(Geometry { coverage: vec![0; budget + 1], distance: vec![], normals: vec![] });
    cache.insert(b.clone(), large, budget);
    assert!(cache.get(&b).is_none());
    assert_eq!(cache.bytes, budget);
}

#[test]
fn cached_geometry_does_not_cache_the_backdrop_or_light() {
    let shape = key().shape;
    let mut back = Pixmap::new(48, 40).unwrap();
    back.fill(tiny_skia::Color::from_rgba8(200, 20, 40, 255));
    let g = Glass { depth: 4.0, frost: 0.0, refraction: 0.0, light_intensity: 0.0, ..Glass::default() };
    let red = glass(&g, &shape, &back, 8, 32, 24, Affine::IDENTITY, 1.0).unwrap();
    back.fill(tiny_skia::Color::from_rgba8(20, 40, 200, 255));
    let blue = glass(&g, &shape, &back, 8, 32, 24, Affine::IDENTITY, 1.0).unwrap();
    let center = (12 * 32 + 16) * 4;
    assert_eq!(&red.data()[center..center + 4], &[200, 20, 40, 255]);
    assert_eq!(&blue.data()[center..center + 4], &[20, 40, 200, 255]);
    let lit = Glass { light_intensity: 0.8, light_angle: -90.0, ..g };
    let left = glass(&lit, &shape, &back, 8, 32, 24, Affine::IDENTITY, 1.0).unwrap();
    let right = glass(&Glass { light_angle: 90.0, ..lit }, &shape, &back, 8, 32, 24, Affine::IDENTITY, 1.0).unwrap();
    assert_ne!(left.data(), right.data());
}
