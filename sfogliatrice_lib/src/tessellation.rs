use geo::algorithm::buffer::{BufferStyle, LineCap, LineJoin};
use geo::{
    BoundingRect, Buffer, Coord, Euclidean, Geometry, Length, Line, LineString, Point, Polygon, Rect, Simplify, coord,
};

use crate::defaults::ROUND_ANGLE;
use crate::geo_utils::{
    coerce_to_polygon, count_lines, distribute_points, ensure_line_length, intersect_line, is_too_small,
    iterate_normalized_geometry, iterate_shards, polygon_ccw_no_holes, project_strip, roll_lines, segment_lines,
};
use crate::intermediate::combine_polygons;
use crate::projection::get_projection;
use crate::types::{Config, Target, TessellationResult, TessellationTuple};

/// Tessellates a single cartesian polygon according to the given config.
fn tessellate_block(geometry: &Polygon, config: &Config) -> TessellationTuple {
    if !config.force_line_targets && is_too_small(geometry, config.min_strip_length) {
        let Some(bbox) = geometry.bounding_rect() else {
            return (vec![], vec![]);
        };
        let center = Point::new((bbox.min().x + bbox.max().x) / 2.0, (bbox.min().y + bbox.max().y) / 2.0);
        let coverage = if config.force_square_coverages {
            // Square buffer: axis-aligned square at strip_width / 2 radius.
            let r = config.strip_width / 2.0;
            Polygon::new(
                LineString::from(vec![
                    coord! {x: center.x() - r, y: center.y() - r},
                    coord! {x: center.x() + r, y: center.y() - r},
                    coord! {x: center.x() + r, y: center.y() + r},
                    coord! {x: center.x() - r, y: center.y() + r},
                    coord! {x: center.x() - r, y: center.y() - r},
                ]),
                vec![],
            )
        } else {
            let Some(c) = center
                .buffer_with_style(
                    BufferStyle::new(config.strip_width / 2.0)
                        .line_cap(LineCap::Round(ROUND_ANGLE))
                        .line_join(LineJoin::Round(ROUND_ANGLE)),
                )
                .into_iter()
                .next()
            else {
                return (vec![Target::Point(center)], vec![]);
            };
            c
        };
        return (vec![Target::Point(center)], vec![coverage]);
    }

    let Some([roll_from, roll_to]) = roll_lines(&Geometry::Polygon(geometry.clone())) else {
        return (vec![], vec![]);
    };
    let roll_from_length = Euclidean.length(&roll_from);
    // count_lines returns None when Config invariants are violated (min_overlap >= strip_width/2).
    // Config::new enforces this, but a hand-built Config can bypass it — skip this block rather than panic.
    let Some(number_of_lines) = count_lines(roll_from_length, config.strip_width, config.min_overlap) else {
        return (vec![], vec![]);
    };

    let points_from = distribute_points(&roll_from, number_of_lines);
    let points_to = distribute_points(&roll_to, number_of_lines);

    let full_lines: Vec<Line> = points_from
        .iter()
        .zip(points_to.iter())
        .map(|(from, to)| Line::new(Coord::from(*from), Coord::from(*to)))
        .collect();

    let mut lines: Vec<Line> = full_lines
        .iter()
        .filter_map(|l| intersect_line(geometry, l, config.strip_width, config.min_strip_length))
        .collect();

    lines = segment_lines(&lines, config.max_strip_length, config.strip_width)
        .into_iter()
        .filter_map(|l| intersect_line(geometry, &l, config.strip_width, config.min_strip_length))
        .collect();

    if number_of_lines == 1 && lines.is_empty() {
        lines = segment_lines(&full_lines, config.max_strip_length, config.strip_width);
    }

    lines = lines
        .iter()
        .filter(|l| Euclidean.length(*l) > 0.)
        .map(|l| ensure_line_length(l, config.min_strip_length))
        .collect();

    let strips: Vec<Polygon> = lines
        .iter()
        .filter_map(|l| project_strip(l, config.strip_width))
        .collect();

    let targets: Vec<Target> = lines.into_iter().map(|l| Target::Line(LineString::from(l))).collect();

    (targets, strips)
}

/// Tessellates a list of cartesian polygons, applying the sharding strategy.
pub fn tessellate_strategy(polygons: &[Polygon], config: &Config) -> TessellationTuple {
    let mut targets: Vec<Target> = vec![];
    let mut coverages: Vec<Polygon> = vec![];
    for polygon in polygons {
        for shard in iterate_shards(polygon, config.shard_radius, config.shard_density_ratio) {
            let (block_targets, block_coverages) = tessellate_block(&shard, config);
            targets.extend(block_targets);
            coverages.extend(block_coverages);
        }
    }
    (targets, coverages)
}

/// Tessellates a list of geodesic geometries according to the given config.
pub fn tessellate(geometries: &[Geometry], config: &Config) -> TessellationResult {
    if geometries.is_empty() {
        return TessellationResult {
            targets: vec![],
            coverages: vec![],
            intermediates: vec![],
        };
    }

    // Anchor the Oblique Mercator projection at the bbox center of the inputs. Projection accuracy
    // near the data depends on proximity, not on an exact centroid — using the bbox avoids cloning
    // every geometry into a GeometryCollection just to call `.centroid()`.
    let Some(anchor) = geometries
        .iter()
        .filter_map(|g| g.bounding_rect())
        .reduce(|a, b| {
            Rect::new(
                coord! { x: a.min().x.min(b.min().x), y: a.min().y.min(b.min().y) },
                coord! { x: a.max().x.max(b.max().x), y: a.max().y.max(b.max().y) },
            )
        })
        .map(|r| Point::from(r.center()))
    else {
        return TessellationResult {
            targets: vec![],
            coverages: vec![],
            intermediates: vec![],
        };
    };

    let transformer = get_projection(&anchor);

    // Project every input geometry from geodesic (degrees) to cartesian (meters).
    // Geometries that fail projection are silently dropped.
    let cartesian_geometries = iterate_normalized_geometry(geometries, |g| transformer.to_cartesian(g).ok());

    // Coerce each geometry to a polygon (points become circles, lines become strips).
    // LineStrings with more than two points are treated as their first-to-last Line.
    // Empty LineStrings and geometries that cannot be buffered are skipped.
    // Simplify in cartesian (meter) space: strip_width / 50 ≈ 100 m at default settings,
    // well below tessellation precision, but dramatically reduces vertex count for complex inputs.
    let expanded: Vec<Polygon> = cartesian_geometries
        .iter()
        .filter_map(|g| match g {
            Geometry::LineString(ls) => {
                let first = ls.0.first()?;
                let last = ls.0.last()?;
                coerce_to_polygon(&Geometry::Line(Line::new(*first, *last)), config.expansion)
            }
            _ => coerce_to_polygon(g, config.expansion),
        })
        .map(|p| p.simplify(config.strip_width / 50.0))
        .collect();

    // Merge nearby polygons into intermediates.
    let cartesian_intermediates = combine_polygons(&expanded, config.inflation);

    // Tessellate each intermediate polygon into targets and coverages.
    let (cartesian_targets, cartesian_coverages) = tessellate_strategy(&cartesian_intermediates, config);

    // Enforce GeoJSON right-hand rule (CCW winding) on polygons.
    let rhr_intermediates: Vec<Polygon> = cartesian_intermediates.iter().map(polygon_ccw_no_holes).collect();
    let rhr_coverages: Vec<Polygon> = cartesian_coverages.iter().map(polygon_ccw_no_holes).collect();

    // Project everything back to geodesic (lon/lat degrees).
    let targets: Vec<Target> = cartesian_targets
        .into_iter()
        .filter_map(|t| match t {
            Target::Point(p) => {
                let g = transformer.to_geodesic(&Geometry::Point(p)).ok()?;
                if let Geometry::Point(p) = g {
                    Some(Target::Point(p))
                } else {
                    None
                }
            }
            Target::Line(ls) => {
                let g = transformer.to_geodesic(&Geometry::LineString(ls)).ok()?;
                if let Geometry::LineString(ls) = g {
                    Some(Target::Line(ls))
                } else {
                    None
                }
            }
        })
        .collect();

    let project_polygon_back = |p: Polygon| -> Option<Polygon> {
        match transformer.to_geodesic(&Geometry::Polygon(p)).ok()? {
            Geometry::Polygon(p) => Some(p),
            _ => None,
        }
    };
    let coverages: Vec<Polygon> = rhr_coverages.into_iter().filter_map(&project_polygon_back).collect();
    let intermediates: Vec<Polygon> = rhr_intermediates
        .into_iter()
        .filter_map(&project_polygon_back)
        .collect();

    TessellationResult {
        targets,
        coverages,
        intermediates,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use geojson::GeoJson;
    use serde_json::{Value, json};

    fn geojson_to_geometries(v: &Value) -> Vec<Geometry> {
        let gj: GeoJson = serde_json::from_value(v.clone()).expect("Invalid GeoJSON in test");
        match gj {
            GeoJson::Geometry(geom) => vec![Geometry::try_from(geom).expect("Failed to convert geometry")],
            _ => vec![],
        }
    }

    fn assert_valid_tessellation(result: &TessellationResult) {
        assert!(
            result
                .targets
                .iter()
                .all(|t| matches!(t, Target::Point(_) | Target::Line(_))),
            "All targets are Points or Lines"
        );
        assert!(
            !result.coverages.is_empty() || result.targets.is_empty(),
            "Coverages must not be empty when there are targets"
        );
    }

    #[test]
    fn test_tessellation_division_by_zero_bug_no_force() {
        // Was raising ZeroDivisionError in the Python version:
        let polygon = json!({
            "type": "Polygon",
            "coordinates": [[
                [-71.47409, 45.06444],
                [-71.47435, 45.06299],
                [-71.47437, 45.06217],
                [-71.47428, 45.05793],
                [-71.47410, 45.05640],
                [-71.47378, 45.05534],
                [-71.47209, 45.05201],
                [-71.47125, 45.04986],
                [-71.47064, 45.04764],
                [-71.47025, 45.04537],
                [-71.47409, 45.06444],
            ]]
        });
        let result = tessellate(&geojson_to_geometries(&polygon), &Config::default());
        assert_valid_tessellation(&result);
    }

    #[test]
    fn test_tessellation_division_by_zero_bug_force_lines() {
        let polygon = json!({
            "type": "Polygon",
            "coordinates": [[
                [-71.47409, 45.06444],
                [-71.47435, 45.06299],
                [-71.47437, 45.06217],
                [-71.47428, 45.05793],
                [-71.47410, 45.05640],
                [-71.47378, 45.05534],
                [-71.47209, 45.05201],
                [-71.47125, 45.04986],
                [-71.47064, 45.04764],
                [-71.47025, 45.04537],
                [-71.47409, 45.06444],
            ]]
        });
        let config = Config {
            force_line_targets: true,
            ..Config::default()
        };
        let result = tessellate(&geojson_to_geometries(&polygon), &config);
        assert_valid_tessellation(&result);
    }

    #[test]
    fn test_right_hand_rule_coverages() {
        use geo::Winding;
        let polygon = json!({
            "type": "Polygon",
            "coordinates": [[
                [13.332607, 52.520232],
                [13.378726, 52.520232],
                [13.378726, 52.504324],
                [13.332607, 52.504324],
                [13.332607, 52.520232],
            ]]
        });
        let config = Config {
            force_square_coverages: true,
            ..Config::default()
        };
        let result = tessellate(&geojson_to_geometries(&polygon), &config);
        assert!(
            result.coverages.iter().all(|p| p.exterior().is_ccw()),
            "All coverages must be CCW (right-hand rule)"
        );
        assert!(
            result.intermediates.iter().all(|p| p.exterior().is_ccw()),
            "All intermediates must be CCW (right-hand rule)"
        );
    }

    #[test]
    fn test_tessellate_empty_input_returns_empty_result() {
        let result = tessellate(&[], &Config::default());
        assert!(result.targets.is_empty());
        assert!(result.coverages.is_empty());
        assert!(result.intermediates.is_empty());
    }

    #[test]
    fn test_tessellate_point_input_yields_circular_coverage() {
        // A Point becomes a buffered circle (coerce_to_polygon), tessellated with one point target.
        let point = json!({ "type": "Point", "coordinates": [13.405, 52.520] });
        let result = tessellate(&geojson_to_geometries(&point), &Config::default());
        assert_valid_tessellation(&result);
        assert!(
            !result.coverages.is_empty(),
            "Point input must yield at least one coverage"
        );
    }

    #[test]
    fn test_tessellate_linestring_input() {
        // LineString becomes a rectangular strip after coerce_to_polygon.
        let ls = json!({
            "type": "LineString",
            "coordinates": [[13.405, 52.520], [13.500, 52.520]],
        });
        let result = tessellate(&geojson_to_geometries(&ls), &Config::default());
        assert_valid_tessellation(&result);
        assert!(
            !result.coverages.is_empty(),
            "LineString input must yield at least one coverage"
        );
    }

    #[test]
    fn test_tessellate_multipolygon_input() {
        let mp = json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[
                    [13.332607, 52.520232],
                    [13.378726, 52.520232],
                    [13.378726, 52.504324],
                    [13.332607, 52.504324],
                    [13.332607, 52.520232],
                ]],
                [[
                    [13.432607, 52.520232],
                    [13.478726, 52.520232],
                    [13.478726, 52.504324],
                    [13.432607, 52.504324],
                    [13.432607, 52.520232],
                ]]
            ]
        });
        let gj: geojson::GeoJson = serde_json::from_value(mp).unwrap();
        let geom = match gj {
            geojson::GeoJson::Geometry(g) => Geometry::try_from(g).unwrap(),
            _ => unreachable!(),
        };
        let result = tessellate(&[geom], &Config::default());
        assert_valid_tessellation(&result);
        assert!(!result.targets.is_empty());
    }

    #[test]
    fn test_tessellate_geometry_collection_input() {
        let gc = json!({
            "type": "GeometryCollection",
            "geometries": [
                { "type": "Point", "coordinates": [13.405, 52.520] },
                {
                    "type": "Polygon",
                    "coordinates": [[
                        [13.332607, 52.520232],
                        [13.378726, 52.520232],
                        [13.378726, 52.504324],
                        [13.332607, 52.504324],
                        [13.332607, 52.520232],
                    ]]
                }
            ]
        });
        let gj: geojson::GeoJson = serde_json::from_value(gc).unwrap();
        let geom = match gj {
            geojson::GeoJson::Geometry(g) => Geometry::try_from(g).unwrap(),
            _ => unreachable!(),
        };
        let result = tessellate(&[geom], &Config::default());
        assert_valid_tessellation(&result);
        assert!(!result.targets.is_empty());
    }

    #[test]
    fn test_tessellate_small_polygon_round_coverage() {
        // Default config => round coverage (not square). Uses a tiny polygon so is_too_small triggers
        // the single-Point target branch. ~200m square in geodesic degrees at latitude ~52°.
        let polygon = json!({
            "type": "Polygon",
            "coordinates": [[
                [13.405000, 52.520000],
                [13.405100, 52.520000],
                [13.405100, 52.520050],
                [13.405000, 52.520050],
                [13.405000, 52.520000],
            ]]
        });
        let result = tessellate(&geojson_to_geometries(&polygon), &Config::default());
        assert_valid_tessellation(&result);
        assert!(
            result.targets.iter().any(|t| matches!(t, Target::Point(_))),
            "Small polygon must yield a Point target on the is_too_small branch"
        );
    }

    #[test]
    fn test_tessellate_force_line_targets_suppresses_points() {
        // Same tiny polygon as above, but `force_line_targets` skips the Point-target branch entirely.
        let polygon = json!({
            "type": "Polygon",
            "coordinates": [[
                [13.405000, 52.520000],
                [13.405100, 52.520000],
                [13.405100, 52.520050],
                [13.405000, 52.520050],
                [13.405000, 52.520000],
            ]]
        });
        let config = Config {
            force_line_targets: true,
            ..Config::default()
        };
        let result = tessellate(&geojson_to_geometries(&polygon), &config);
        assert_valid_tessellation(&result);
        assert!(
            result.targets.iter().all(|t| matches!(t, Target::Line(_))),
            "force_line_targets must suppress Point targets"
        );
    }

    #[test]
    fn test_min_overlap() {
        let polygon = json!({
            "type": "Polygon",
            "coordinates": [[
                [-0.021433, 0.060407],
                [-0.137964, 0.060407],
                [-0.137964, -0.025299],
                [-0.021433, -0.025299],
                [-0.021433, 0.060407],
            ]]
        });
        let result = tessellate(&geojson_to_geometries(&polygon), &Config::default());
        assert_eq!(result.targets.len(), 2, "Expected exactly 2 targets for this polygon");
    }
}
