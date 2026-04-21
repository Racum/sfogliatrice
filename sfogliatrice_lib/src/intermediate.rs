use crate::defaults::ROUND_ANGLE;
use crate::geo_utils::is_too_small;
use geo::Buffer;
use geo::algorithm::bool_ops::BooleanOps;
use geo::algorithm::buffer::{BufferStyle, LineCap, LineJoin};
use geo::algorithm::unary_union;
use geo::{
    Area, BoundingRect, Centroid, Coord, CoordsIter, LineString, MultiPolygon, Polygon, Simplify, Validation, Winding,
};

/// Cap on exterior vertex count for `deflate`. The algorithm buffers a bbox-sized polygon with a hole,
/// which scales roughly with vertex count × buffer arcs per vertex — pathological inputs would be
/// prohibitively slow. Inputs exceeding this cap are dropped rather than processed.
const DEFLATE_MAX_EXTERIOR_VERTICES: usize = 50_000;

/// Prepare polygon, making sure small original polygons will not disappear.
fn prepare(polygon: &Polygon, inflation: f64) -> Polygon {
    if is_too_small(polygon, inflation / 4.) {
        let Some(centroid) = polygon.centroid() else {
            return polygon.clone();
        };
        centroid
            .buffer_with_style(
                BufferStyle::new(inflation * 0.25)
                    .line_cap(LineCap::Round(ROUND_ANGLE))
                    .line_join(LineJoin::Round(ROUND_ANGLE)),
            )
            .into_iter()
            .next()
            .unwrap_or_else(|| polygon.clone())
    } else {
        polygon.clone()
    }
}

/// Expand a round buffer around the polygon. Returns None when the buffer produces no polygon
/// (e.g. non-positive or non-finite `inflation`, or a degenerate geo buffer result).
fn inflate(polygon: &Polygon, inflation: f64) -> Option<Polygon> {
    if !inflation.is_finite() || inflation <= 0.0 {
        return None;
    }
    // Simplify before buffering: inflate is only used to detect which polygons should merge,
    // so vertex precision at this stage is irrelevant. The buffer (inflation * 1.5) dominates
    // any edge introduced by simplifying at inflation / 10.
    polygon
        .simplify(inflation / 10.0)
        .buffer_with_style(
            BufferStyle::new(inflation * 1.5)
                .line_cap(LineCap::Round(ROUND_ANGLE))
                .line_join(LineJoin::Round(ROUND_ANGLE)),
        )
        .into_iter()
        .next()
}

/// Normalizes a polygon to CCW exterior winding, which geo::BooleanOps requires.
/// A CW exterior would be treated as a hole and subtracted during union operations.
fn make_ccw(polygon: Polygon) -> Polygon {
    let (mut exterior, interiors) = polygon.into_inner();
    exterior.make_ccw_winding();
    Polygon::new(exterior, interiors)
}

/// Unions a list of polygons, discarding any that are geometrically invalid.
fn combine(polygons: Vec<Polygon>) -> Vec<Polygon> {
    let ccw: Vec<Polygon> = polygons.into_iter().map(make_ccw).collect();
    unary_union(ccw.iter().filter(|g| g.is_valid())).0
}

/// Shrinks a polygon inward by `amount` using the Minkowski complement trick.
///
/// Algorithm (equivalent to Shapely's negative buffer):
///   1. Build a bounding box large enough to fully contain the polygon.
///   2. `outside` = bbox − polygon  (a rectangle with a hole).
///   3. `inflated_outside` = positive-buffer `outside` by `amount`.
///      geo's positive buffer is correct; this eats into the hole by `amount`.
///   4. `eroded` = bbox − inflated_outside  = the surviving interior = deflated polygon.
///
/// Returns None if the deflation collapses the polygon entirely, if `amount` is not a positive
/// finite number, or if the input polygon's exterior exceeds `DEFLATE_MAX_EXTERIOR_VERTICES`.
fn deflate(polygon: &Polygon, amount: f64) -> Option<Polygon> {
    if !amount.is_finite() || amount <= 0.0 {
        return None;
    }
    if polygon.exterior().coords_count() > DEFLATE_MAX_EXTERIOR_VERTICES {
        return None;
    }
    let bbox = polygon.bounding_rect()?;
    let pad = amount * 3.0;
    let big_box = Polygon::new(
        LineString::from(vec![
            Coord {
                x: bbox.min().x - pad,
                y: bbox.min().y - pad,
            },
            Coord {
                x: bbox.max().x + pad,
                y: bbox.min().y - pad,
            },
            Coord {
                x: bbox.max().x + pad,
                y: bbox.max().y + pad,
            },
            Coord {
                x: bbox.min().x - pad,
                y: bbox.max().y + pad,
            },
            Coord {
                x: bbox.min().x - pad,
                y: bbox.min().y - pad,
            },
        ]),
        vec![],
    );
    let outside: MultiPolygon = big_box.difference(polygon);
    let inflated_outside = outside.buffer_with_style(
        BufferStyle::new(amount)
            .line_cap(LineCap::Round(ROUND_ANGLE))
            .line_join(LineJoin::Round(ROUND_ANGLE)),
    );
    let eroded = MultiPolygon::new(vec![big_box]).difference(&inflated_outside);
    eroded
        .0
        .into_iter()
        .filter(|p| p.is_valid() && p.unsigned_area() > 0.0)
        .max_by(|a, b| a.unsigned_area().total_cmp(&b.unsigned_area()))
}

/// Combines a list of polygons into fewer polygons, applying a given inflation distance for the combination.
pub fn combine_polygons(polygons: &[Polygon], inflation: f64) -> Vec<Polygon> {
    let prepared: Vec<Polygon> = polygons.iter().map(|p| prepare(p, inflation)).collect();
    let inflated: Vec<Polygon> = prepared.iter().filter_map(|p| inflate(p, inflation)).collect();
    let deflated: Vec<Polygon> = combine(inflated)
        .into_iter()
        .filter_map(|p| deflate(&p, inflation * 1.5))
        .collect();
    combine(deflated.into_iter().chain(polygons.iter().cloned()).collect())
}

#[cfg(test)]
mod tests {
    use geo::{Area, Centroid, Geometry, GeometryCollection, LineString, algorithm::bool_ops::BooleanOps, polygon};
    use geojson::GeoJson;
    use serde_json::Value;

    use super::*;
    use crate::geo_utils::{coerce_to_polygon, iterate_normalized_geometry};
    use crate::projection::get_projection;

    fn iterate_geometry_from_geojson(geojson: &Value) -> Vec<Geometry> {
        let gj: GeoJson = serde_json::from_value(geojson.clone()).expect("Invalid GeoJSON");
        match gj {
            GeoJson::Geometry(geom) => Geometry::try_from(geom).into_iter().collect(),
            GeoJson::Feature(f) => f
                .geometry
                .and_then(|g| Geometry::try_from(g).ok())
                .into_iter()
                .collect(),
            GeoJson::FeatureCollection(fc) => fc
                .features
                .into_iter()
                .filter_map(|f| f.geometry.and_then(|g| Geometry::try_from(g).ok()))
                .collect(),
        }
    }

    #[test]
    fn test_combine_polygons() {
        let fixtures = ["comunidad_valenciana.geojson", "iberia.geojson"];
        let mut failures: Vec<String> = Vec::new();

        for name in fixtures {
            let path = std::path::Path::new("../fixtures").join(name);
            let geojson_str = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("Failed to read {name}: {e}"));
            let geojson: Value =
                serde_json::from_str(&geojson_str).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"));

            let geodesic_geometries = iterate_geometry_from_geojson(&geojson);
            let gc = GeometryCollection::new_from(geodesic_geometries.clone());
            let centroid = gc
                .centroid()
                .unwrap_or_else(|| panic!("Failed to compute centroid for {name}"));
            let transformer = get_projection(&centroid);

            let cartesian_geometries =
                iterate_normalized_geometry(&geodesic_geometries, |g| transformer.to_cartesian(g).ok());

            let expanded: Vec<Polygon> = cartesian_geometries
                .iter()
                .filter_map(|g| coerce_to_polygon(g, 5_000.0))
                .collect();

            let result = combine_polygons(&expanded, 5_000.0);

            let input_area: f64 = cartesian_geometries.iter().map(|p| p.unsigned_area()).sum();
            let output_area: f64 = result.iter().map(|p| p.unsigned_area()).sum();

            if result.is_empty() {
                failures.push(format!("{name}: returned empty result"));
            } else if output_area < input_area {
                failures.push(format!(
                    "{name}: output area ({output_area:.0}) is smaller than input area ({input_area:.0})"
                ));
            } else if output_area / input_area >= 1.5 {
                failures.push(format!(
                    "{name}: output area ({output_area:.0}) is {:.2}× input area ({input_area:.0}), expected < 1.5×",
                    output_area / input_area
                ));
            } else {
                for (i, input) in expanded.iter().enumerate() {
                    let input_area = input.unsigned_area();
                    let covered: f64 = result.iter().map(|o| o.intersection(input).unsigned_area()).sum();
                    let coverage = covered / input_area;
                    if coverage < 0.99 {
                        failures.push(format!(
                            "{name}: input polygon #{i} is only {:.1}% covered by output polygons, expected >= 99%",
                            coverage * 100.0
                        ));
                    }
                }
            }
        }

        assert!(failures.is_empty(), "Failures:\n  - {}", failures.join("\n  - "));
    }

    fn square(x: f64, y: f64, size: f64) -> Polygon {
        polygon![
            (x: x, y: y),
            (x: x + size, y: y),
            (x: x + size, y: y + size),
            (x: x, y: y + size),
            (x: x, y: y),
        ]
    }

    #[test]
    fn test_inflate_invalid_amount_returns_none() {
        let p = square(0.0, 0.0, 1_000.0);
        assert!(inflate(&p, 0.0).is_none());
        assert!(inflate(&p, -1.0).is_none());
        assert!(inflate(&p, f64::NAN).is_none());
        assert!(inflate(&p, f64::INFINITY).is_none());
    }

    #[test]
    fn test_deflate_invalid_amount_returns_none() {
        let p = square(0.0, 0.0, 10_000.0);
        assert!(deflate(&p, 0.0).is_none());
        assert!(deflate(&p, -1.0).is_none());
        assert!(deflate(&p, f64::NAN).is_none());
        assert!(deflate(&p, f64::INFINITY).is_none());
    }

    #[test]
    fn test_deflate_collapse_returns_none() {
        // amount >> polygon diameter: eroded interior vanishes.
        let p = square(0.0, 0.0, 100.0);
        assert!(deflate(&p, 10_000.0).is_none());
    }

    #[test]
    fn test_deflate_vertex_cap_returns_none() {
        // Build a polygon whose exterior exceeds DEFLATE_MAX_EXTERIOR_VERTICES.
        let n = DEFLATE_MAX_EXTERIOR_VERTICES + 1;
        let coords: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let theta = (i as f64) / (n as f64) * std::f64::consts::TAU;
                (theta.cos() * 1_000.0, theta.sin() * 1_000.0)
            })
            .collect();
        let ls = LineString::from(coords);
        let big = Polygon::new(ls, vec![]);
        assert!(big.exterior().coords_count() > DEFLATE_MAX_EXTERIOR_VERTICES);
        assert!(deflate(&big, 10.0).is_none());
    }

    #[test]
    fn test_combine_polygons_empty() {
        let out = combine_polygons(&[], 5_000.0);
        assert!(out.is_empty());
    }

    #[test]
    fn test_combine_polygons_single() {
        let p = square(0.0, 0.0, 10_000.0);
        let out = combine_polygons(std::slice::from_ref(&p), 1_000.0);
        assert_eq!(out.len(), 1, "single polygon passes through as one");
        // Output area should at least cover the input area.
        assert!(out[0].unsigned_area() >= p.unsigned_area() * 0.99);
    }

    #[test]
    fn test_combine_polygons_far_apart_remain_separate() {
        // Inputs separated by far more than 2 * inflation must not merge.
        let inflation = 1_000.0;
        let a = square(0.0, 0.0, 5_000.0);
        let b = square(100_000.0, 100_000.0, 5_000.0);
        let out = combine_polygons(&[a.clone(), b.clone()], inflation);
        assert_eq!(out.len(), 2, "non-overlapping polygons must remain separate");
    }

    #[test]
    fn test_combine_polygons_overlapping_collapse_to_one() {
        // Two identical squares — combine should yield a single polygon covering their shared area.
        let a = square(0.0, 0.0, 10_000.0);
        let out = combine_polygons(&[a.clone(), a.clone()], 1_000.0);
        assert_eq!(out.len(), 1, "fully-overlapping polygons must collapse to one");
    }
}
