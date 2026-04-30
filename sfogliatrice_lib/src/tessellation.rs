use geo::algorithm::buffer::{BufferStyle, LineCap, LineJoin};
use geo::algorithm::unary_union;
use geo::{
    Area, BooleanOps, BoundingRect, Buffer, Coord, Euclidean, Geometry, Length, Line, LineString, MultiPolygon, Point,
    Polygon, Rect, Simplify, Validation, coord,
};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

use serde_json::Value;

use crate::defaults::ROUND_ANGLE;
use crate::geo_utils::{
    ConvexHullCache, coerce_to_polygon, count_lines, distribute_points, ensure_line_length, intersect_line,
    is_too_small, iterate_normalized_geometry, iterate_shards, project_strip, roll_lines, segment_lines,
};
use crate::intermediate::combine_polygons;
use crate::projection::get_projection;
use crate::types::{Config, Target, TessellationGeoJSONResult, TessellationGeoResult, TessellationTuple};

/// Extracts hole rings from `polygon`, applies simplify + inset, unions them, and returns the
/// resulting void polygons ready to be used for line clipping. Returns empty when `ignore_holes`
/// is set or there are no meaningful holes.
fn hole_masks(polygon: &Polygon, config: &Config) -> MultiPolygon {
    if config.ignore_holes {
        return MultiPolygon::new(vec![]);
    }
    let transform = |h: Polygon| {
        // MARKER: apply transformations to hole_polygons here.
        // After transformation, re-filter: drop invalid and too-small results.
        h.simplify(config.strip_width / 50.0)
            .buffer_with_style(BufferStyle::new(-config.strip_width / 2.0))
            .into_iter()
            .filter(|h: &Polygon| h.is_valid() && !is_too_small(h, config.min_strip_length))
            .collect::<Vec<_>>()
    };

    let ring_to_masks = |ring: &LineString| -> Vec<Polygon> {
        let h = Polygon::new(ring.clone(), vec![]);
        if !h.is_valid() || is_too_small(&h, config.min_strip_length) {
            return vec![];
        }
        transform(h)
    };

    #[cfg(feature = "parallel")]
    let masks: Vec<Polygon> = polygon.interiors().par_iter().flat_map_iter(ring_to_masks).collect();
    #[cfg(not(feature = "parallel"))]
    let masks: Vec<Polygon> = polygon.interiors().iter().flat_map(ring_to_masks).collect();
    if masks.is_empty() {
        return MultiPolygon::new(vec![]);
    }
    unary_union(masks.iter())
}

/// Clips `line` against `holes`, then bridges any gap smaller than `strip_width` back together.
/// If the line falls entirely inside a hole but the hole is smaller than `strip_width`, the
/// original line is returned unchanged.
fn clip_line_by_holes(line: &Line, holes: &MultiPolygon, strip_width: f64) -> Vec<Line> {
    use geo::MultiLineString;
    let multi_line = MultiLineString::new(vec![LineString::from(*line)]);
    let mut segs: Vec<Line> = holes
        .clip(&multi_line, true)
        .into_iter()
        .filter_map(|s| Some(Line::new(s.points().next()?, s.points().next_back()?)))
        .collect();

    if segs.is_empty() {
        return if Euclidean.length(line) < strip_width {
            vec![*line]
        } else {
            vec![]
        };
    }

    // Sort by signed scalar projection onto the line direction so segment order is
    // unambiguous regardless of what BooleanOps::clip returns.
    let dx = line.end.x - line.start.x;
    let dy = line.end.y - line.start.y;
    let project = |c: Coord| (c.x - line.start.x) * dx + (c.y - line.start.y) * dy;
    segs.sort_by(|a, b| {
        project(a.start)
            .partial_cmp(&project(b.start))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut result: Vec<Line> = Vec::with_capacity(segs.len());
    let mut current = segs[0];
    for next in segs.into_iter().skip(1) {
        if Euclidean.length(&Line::new(current.end, next.start)) < strip_width {
            current = Line::new(current.start, next.end);
        } else {
            result.push(current);
            current = next;
        }
    }
    result.push(current);
    result
}

/// Returns a point-target result when `polygon` is too small for strip tessellation, or `None`
/// when the polygon is large enough to proceed normally.
fn try_as_point(polygon: &Polygon, config: &Config) -> Option<TessellationTuple> {
    if config.force_line_targets || !is_too_small(polygon, config.min_strip_length) {
        return None;
    }
    let bbox = polygon.bounding_rect()?;
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
        center
            .buffer_with_style(
                BufferStyle::new(config.strip_width / 2.0)
                    .line_cap(LineCap::Round(ROUND_ANGLE))
                    .line_join(LineJoin::Round(ROUND_ANGLE)),
            )
            .into_iter()
            .next()
            .unwrap_or_else(|| Polygon::new(LineString::new(vec![]), vec![]))
    };
    Some((vec![Target::Point(center)], vec![coverage]))
}

/// Computes the clipped, segmented, hole-masked strip lines for `polygon`.
/// Returns an empty vec when the geometry is degenerate (no roll lines, invalid config).
fn compute_lines(polygon: &Polygon, config: &Config, hull_cache: Option<&ConvexHullCache>) -> Vec<Line> {
    let Some([roll_from, roll_to]) = roll_lines(&Geometry::Polygon(polygon.clone()), config.heading, hull_cache) else {
        return vec![];
    };
    let roll_from_length = Euclidean.length(&roll_from);
    // count_lines returns None when Config invariants are violated (min_overlap >= strip_width/2).
    // Config::new enforces this, but a hand-built Config can bypass it — skip rather than panic.
    let Some(number_of_lines) = count_lines(roll_from_length, config.strip_width, config.min_overlap) else {
        return vec![];
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
        .filter_map(|l| intersect_line(polygon, l, config.strip_width, config.min_strip_length))
        .collect();

    lines = segment_lines(&lines, config.max_strip_length, config.strip_width)
        .into_iter()
        .filter_map(|l| intersect_line(polygon, &l, config.strip_width, config.min_strip_length))
        .collect();

    if number_of_lines == 1 && lines.is_empty() {
        lines = segment_lines(&full_lines, config.max_strip_length, config.strip_width)
            .into_iter()
            .filter_map(|l| intersect_line(polygon, &l, config.strip_width, config.min_strip_length))
            .collect();
    }

    lines = lines
        .iter()
        .filter(|l| Euclidean.length(*l) > 0.)
        .map(|l| ensure_line_length(l, config.min_strip_length))
        .collect();

    let holes = hole_masks(polygon, config);
    if !holes.0.is_empty() {
        lines = lines
            .iter()
            .flat_map(|l| clip_line_by_holes(l, &holes, config.strip_width))
            .collect();
    }

    lines
}

/// Tessellates a single cartesian polygon according to the given config.
fn tessellate_block(polygon: &Polygon, config: &Config, hull_cache: Option<&ConvexHullCache>) -> TessellationTuple {
    if let Some(r) = try_as_point(polygon, config) {
        return r;
    }
    let lines = compute_lines(polygon, config, hull_cache);
    let strips = lines
        .iter()
        .filter_map(|l| project_strip(l, config.strip_width))
        .collect();
    let targets = lines.into_iter().map(|l| Target::Line(LineString::from(l))).collect();
    (targets, strips)
}

/// Tessellates one polygon, sweeping all headings 0°–175° in 5° steps when brute-force is active.
/// Delegates to [`tessellate_block`] otherwise. The convex hull is precomputed once and reused.
fn tessellate_with_best_heading(polygon: &Polygon, config: &Config) -> TessellationTuple {
    if !(config.brute_force && config.heading.is_none()) {
        return tessellate_block(polygon, config, None);
    }
    let default = tessellate_block(polygon, config, None);
    if default.0.len() <= 1 {
        return default;
    }

    let hull_cache = ConvexHullCache::for_polygon(polygon);
    let cache_ref = hull_cache.as_ref();
    let sweep_base = config.clone();
    let input_area = polygon.unsigned_area();

    let compare = |a: &TessellationTuple, b: &TessellationTuple| -> std::cmp::Ordering {
        a.0.len().cmp(&b.0.len()).then_with(|| {
            if input_area == 0.0 {
                return std::cmp::Ordering::Equal;
            }
            let ov = |cs: &[Polygon]| cs.iter().map(|p| p.unsigned_area()).sum::<f64>() / input_area;
            ov(&a.1).partial_cmp(&ov(&b.1)).unwrap_or(std::cmp::Ordering::Equal)
        })
    };

    let try_heading = |deg: u32| {
        tessellate_block(
            polygon,
            &Config {
                heading: Some(f64::from(deg)),
                ..sweep_base.clone()
            },
            cache_ref,
        )
    };

    #[cfg(feature = "parallel")]
    let best = (0_u32..180).into_par_iter().step_by(5).map(try_heading).min_by(compare);
    #[cfg(not(feature = "parallel"))]
    let best = (0_u32..180).step_by(5).map(try_heading).min_by(compare);

    best.unwrap_or_else(|| tessellate_block(polygon, config, cache_ref))
}

/// Tessellates cartesian polygons (metre space). Shards all inputs into a flat collection,
/// then dispatches each to [`tessellate_with_best_heading`] — in parallel when `parallel` is enabled.
pub fn tessellate_strategy(polygons: &[Polygon], config: &Config) -> TessellationTuple {
    let polygons_flat: Vec<Polygon> = polygons
        .iter()
        .flat_map(|p| iterate_shards(p, config.shard_radius, config.shard_density_ratio))
        .collect();

    let combine = |(mut t, mut c): TessellationTuple, (bt, bc): TessellationTuple| -> TessellationTuple {
        t.extend(bt);
        c.extend(bc);
        (t, c)
    };

    #[cfg(feature = "parallel")]
    {
        polygons_flat
            .par_iter()
            .map(|polygon| tessellate_with_best_heading(polygon, config))
            .reduce(|| (vec![], vec![]), combine)
    }
    #[cfg(not(feature = "parallel"))]
    {
        polygons_flat
            .iter()
            .map(|polygon| tessellate_with_best_heading(polygon, config))
            .fold((vec![], vec![]), combine)
    }
}

/// Tessellates a list of geodesic geometries according to the given config.
pub fn tessellate(geometries: &[Geometry], config: &Config) -> TessellationGeoResult {
    if geometries.is_empty() {
        return TessellationGeoResult::empty();
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
        return TessellationGeoResult::empty();
    };

    let Some(transformer) = get_projection(&anchor) else {
        return TessellationGeoResult::empty();
    };

    // Project every input geometry from geodesic (degrees) to cartesian (meters).
    // Geometries that fail projection are silently dropped.
    let cartesian_geometries = iterate_normalized_geometry(geometries, |g| transformer.to_cartesian(g).ok());

    // Coerce each geometry to a polygon (points become circles, lines become strips).
    // Empty LineStrings and geometries that cannot be buffered are skipped.
    // Simplify in cartesian (meter) space: strip_width / 50 ≈ 100 m at default settings,
    // well below tessellation precision, but dramatically reduces vertex count for complex inputs.
    let expanded: Vec<Polygon> = cartesian_geometries
        .iter()
        .filter_map(|g| match g {
            Geometry::LineString(ls) => {
                if ls.0.len() < 2 {
                    return None;
                }
                ls.buffer_with_style(
                    BufferStyle::new(config.expansion)
                        .line_cap(LineCap::Round(ROUND_ANGLE))
                        .line_join(LineJoin::Round(ROUND_ANGLE)),
                )
                .into_iter()
                .next()
            }
            _ => coerce_to_polygon(g, config.expansion),
        })
        .map(|p| p.simplify(config.strip_width / 50.0))
        .collect();

    // Merge nearby polygons into intermediates.
    let cartesian_intermediates = combine_polygons(&expanded, config.strip_width);

    // Tessellate each intermediate polygon into targets and coverages.
    // Brute-force heading optimisation is handled inside tessellate_strategy, per shard.
    let (cartesian_targets, cartesian_coverages) = tessellate_strategy(&cartesian_intermediates, config);

    // Project everything back to geodesic (lon/lat degrees).
    // Winding correction is deferred to the GeoJSON serialisation step via polygon_feature_rfc7946,
    // keeping the geo layer free of JSON conventions.
    let project_target_back = |t: Target| -> Option<Target> {
        match t {
            Target::Point(p) => match transformer.to_geodesic(&Geometry::Point(p)).ok()? {
                Geometry::Point(p) => Some(Target::Point(p)),
                _ => None,
            },
            Target::Line(ls) => match transformer.to_geodesic(&Geometry::LineString(ls)).ok()? {
                Geometry::LineString(ls) => Some(Target::Line(ls)),
                _ => None,
            },
        }
    };
    let project_polygon_back = |p: Polygon| -> Option<Polygon> {
        match transformer.to_geodesic(&Geometry::Polygon(p)).ok()? {
            Geometry::Polygon(p) => Some(p),
            _ => None,
        }
    };

    #[cfg(feature = "parallel")]
    let targets: Vec<Target> = cartesian_targets
        .into_par_iter()
        .filter_map(project_target_back)
        .collect();
    #[cfg(not(feature = "parallel"))]
    let targets: Vec<Target> = cartesian_targets.into_iter().filter_map(project_target_back).collect();

    #[cfg(feature = "parallel")]
    let coverages: Vec<Polygon> = cartesian_coverages
        .into_par_iter()
        .filter_map(project_polygon_back)
        .collect();
    #[cfg(not(feature = "parallel"))]
    let coverages: Vec<Polygon> = cartesian_coverages
        .into_iter()
        .filter_map(project_polygon_back)
        .collect();

    #[cfg(feature = "parallel")]
    let intermediates: Vec<Polygon> = cartesian_intermediates
        .into_par_iter()
        .filter_map(project_polygon_back)
        .collect();
    #[cfg(not(feature = "parallel"))]
    let intermediates: Vec<Polygon> = cartesian_intermediates
        .into_iter()
        .filter_map(project_polygon_back)
        .collect();

    TessellationGeoResult {
        targets,
        coverages,
        intermediates,
    }
}

/// Parses a GeoJSON Value and tessellates the contained geometries, returning geo objects.
pub fn tessellate_geojson_to_geo(geojson: &Value, config: &Config) -> TessellationGeoResult {
    let geometries = crate::geojson::iterate_geometry_from_geojson(geojson);
    tessellate(&geometries, config)
}

/// Parses a GeoJSON Value, tessellates, and returns the result as GeoJSON FeatureCollections.
pub fn tessellate_geojson_to_geojson(geojson: &Value, config: &Config) -> TessellationGeoJSONResult {
    let r = tessellate_geojson_to_geo(geojson, config);
    let target_feature = |t: Target| {
        let geom = match t {
            Target::Point(p) => crate::geojson::geometry_to_json(&Geometry::Point(p)),
            Target::Line(ls) => crate::geojson::geometry_to_json(&Geometry::LineString(ls)),
        };
        crate::geojson::feature(&geom)
    };
    TessellationGeoJSONResult {
        targets: crate::geojson::feature_collection(r.targets.into_iter().map(target_feature).collect()),
        coverages: crate::geojson::feature_collection(
            r.coverages
                .into_iter()
                .map(crate::geojson::polygon_feature_rfc7946)
                .collect(),
        ),
        intermediates: crate::geojson::feature_collection(
            r.intermediates
                .into_iter()
                .map(crate::geojson::polygon_feature_rfc7946)
                .collect(),
        ),
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

    fn assert_valid_tessellation(result: &TessellationGeoResult) {
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
    fn test_brute_force_yields_valid_result() {
        let polygon = json!({
            "type": "Polygon",
            "coordinates": [[
                [13.332607, 52.520232],
                [13.378726, 52.520232],
                [13.378726, 52.504324],
                [13.332607, 52.504324],
                [13.332607, 52.520232],
            ]],
        });
        let config = Config {
            brute_force: true,
            ..Config::default()
        };
        let result = tessellate(&geojson_to_geometries(&polygon), &config);
        assert_valid_tessellation(&result);
        assert!(!result.targets.is_empty(), "brute-force must produce targets");
    }

    #[test]
    fn test_brute_force_ignored_when_heading_is_set() {
        // Explicit heading wins: brute_force is a no-op when heading is Some.
        let polygon = json!({
            "type": "Polygon",
            "coordinates": [[
                [1.524868, 39.134803],
                [1.294929, 39.069947],
                [1.261153, 39.009406],
                [1.142936, 38.994936],
                [1.149432, 38.932310],
                [1.191003, 38.913781],
                [1.156793, 38.854118],
                [1.200529, 38.829834],
                [1.307054, 38.845013],
                [1.359916, 38.808978],
                [1.425597, 38.808645],
                [1.439245, 38.871099],
                [1.546295, 38.930179],
                [1.564634, 38.966002],
                [1.620079, 38.981918],
                [1.636286, 39.010755],
                [1.668273, 39.038256],
                [1.616667, 39.108119],
                [1.524868, 39.134803],
            ]],
        });
        let config_bf = Config {
            brute_force: true,
            heading: Some(45.0),
            ..Config::default()
        };
        let config_plain = Config {
            heading: Some(45.0),
            ..Config::default()
        };
        let r1 = tessellate(&geojson_to_geometries(&polygon), &config_bf);
        let r2 = tessellate(&geojson_to_geometries(&polygon), &config_plain);
        assert_eq!(r1.targets.len(), r2.targets.len());
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

    #[test]
    fn test_tessellate_geojson_feature_input() {
        let feat = json!({
            "type": "Feature",
            "properties": {},
            "geometry": {
                "type": "Polygon",
                "coordinates": [[
                    [13.332607, 52.520232],
                    [13.378726, 52.520232],
                    [13.378726, 52.504324],
                    [13.332607, 52.504324],
                    [13.332607, 52.520232],
                ]],
            },
        });
        let result = tessellate_geojson_to_geo(&feat, &Config::default());
        assert!(!result.targets.is_empty(), "Feature-wrapped polygon should tessellate");
    }

    #[test]
    fn test_tessellate_geojson_to_geojson_returns_feature_collections() {
        let polygon = json!({
            "type": "Polygon",
            "coordinates": [[
                [13.332607, 52.520232],
                [13.378726, 52.520232],
                [13.378726, 52.504324],
                [13.332607, 52.504324],
                [13.332607, 52.520232],
            ]],
        });
        let result = tessellate_geojson_to_geojson(&polygon, &Config::default());
        for field in [&result.targets, &result.coverages, &result.intermediates] {
            assert_eq!(
                field["type"], "FeatureCollection",
                "each field must be a FeatureCollection"
            );
            assert!(
                field["features"].as_array().is_some(),
                "each field must have a features array"
            );
        }
        assert!(
            !result.targets["features"].as_array().unwrap().is_empty(),
            "polygon must produce targets"
        );
        assert!(
            !result.coverages["features"].as_array().unwrap().is_empty(),
            "polygon must produce coverages"
        );
        assert!(
            !result.intermediates["features"].as_array().unwrap().is_empty(),
            "polygon must produce intermediates"
        );
    }
}
