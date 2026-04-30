use geo::algorithm::buffer::{BufferStyle, LineCap, LineJoin};
use geo::algorithm::unary_union;
use geo::{
    Area, BooleanOps, BoundingRect, Buffer, Centroid, ConvexHull, Distance, Euclidean, Geometry, HasDimensions,
    InterpolatableLine, Length, Line, LineString, MinimumRotatedRect, MultiLineString, MultiPolygon, Point, Polygon,
    Simplify, Validation, Winding, coord, point,
};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

use crate::defaults::{CIRCLE_EXPANSION_CORRECTION, ROUND_ANGLE};

/// Precomputed convex-hull data for a polygon. Passing this into [`roll_lines`] avoids
/// recomputing the hull, centroid, and bounding-box length on every brute-force heading call.
pub struct ConvexHullCache {
    pub hull: Polygon,
    pub centroid: Point,
    pub length: f64,
}

impl ConvexHullCache {
    pub fn for_polygon(polygon: &Polygon) -> Option<Self> {
        let hull = polygon.convex_hull();
        let bbox = polygon.bounding_rect()?;
        let centroid = hull.centroid()?;
        Some(Self {
            hull,
            centroid,
            length: 2.0 * (bbox.width() + bbox.height()),
        })
    }
}

/// Flattens an arbitrary geometry into its primitive leaf components: Points, LineStrings, and Polygons.
/// Multi* and `GeometryCollection` types are unwrapped. Line, Rect, and Triangle are normalized to their
/// LineString or Polygon equivalents.
///
/// Nested `GeometryCollection`s (a collection inside a collection) are dropped rather than recursed
/// into — this caps recursion depth at 2 and prevents stack exhaustion from adversarial nesting.
pub fn iterate_geometry(geometry: &Geometry) -> Box<dyn Iterator<Item = Geometry> + '_> {
    match geometry {
        Geometry::Point(g) => Box::new(std::iter::once(Geometry::Point(*g))),
        Geometry::LineString(g) => Box::new(std::iter::once(Geometry::LineString(g.clone()))),
        Geometry::Polygon(g) => Box::new(std::iter::once(Geometry::Polygon(g.clone()))),
        Geometry::MultiPoint(g) => Box::new(g.into_iter().map(|p: &geo::Point| Geometry::Point(*p))),
        Geometry::MultiLineString(g) => Box::new(g.into_iter().map(|g| Geometry::LineString(g.clone()))),
        Geometry::MultiPolygon(g) => Box::new(g.into_iter().map(|g| Geometry::Polygon(g.clone()))),
        Geometry::Line(g) => Box::new(std::iter::once(Geometry::LineString(LineString::from(g)))),
        Geometry::Rect(g) => Box::new(std::iter::once(Geometry::Polygon(g.to_polygon()))),
        Geometry::Triangle(g) => Box::new(std::iter::once(Geometry::Polygon(g.to_polygon()))),
        Geometry::GeometryCollection(g) => Box::new(
            g.into_iter()
                .filter(|sub| !matches!(sub, Geometry::GeometryCollection(_)))
                .flat_map(iterate_geometry),
        ),
    }
}

/// Iterates over the results of iterate_geometry, converting all non-Polygons into Polygons.
/// Points and LineStrings are replaced by a tiny square buffer (1e-8 units); convex hull is used
/// as fallback for degenerate cases where the buffer produces no output.
pub fn iterate_polygons(geometry: &Geometry) -> impl Iterator<Item = Polygon> {
    let style = BufferStyle::new(1e-8)
        .line_cap(LineCap::Square)
        .line_join(LineJoin::Miter(f64::MAX));
    iterate_geometry(geometry).map(move |geo| match geo {
        Geometry::Point(g) => g
            .buffer_with_style(style.clone())
            .into_iter()
            .next()
            .unwrap_or_else(|| ConvexHull::convex_hull(&g)),
        Geometry::LineString(g) => g
            .buffer_with_style(style.clone())
            .into_iter()
            .next()
            .unwrap_or_else(|| ConvexHull::convex_hull(&g)),
        Geometry::Polygon(g) => g,
        // iterate_geometry always yields Point, LineString, or Polygon — never composite types.
        _ => unreachable!(),
    })
}

/// Attempts to repair a geometry, returning Ok if it can be made valid or Err if not recoverable.
///
/// For Polygons, repair steps applied in order:
///   1. CCW fast-path: already CCW → return immediately (O(n), no validity check).
///   2. Normalize winding and deduplicate near-collinear vertices (simplify 1e-10).
///   3. Area proxy: positive unsigned area → valid, return immediately (O(n)).
///   4. Near-zero area: self-union repair for bow-ties and crossing rings; Err if still invalid.
fn fix_geometry(geometry: &Geometry) -> Result<Geometry, ()> {
    // Only polygons benefit from the repair steps below; pass other types through.
    let Geometry::Polygon(polygon) = geometry else {
        return if geometry.is_valid() {
            Ok(geometry.clone())
        } else {
            Err(())
        };
    };

    // is_ccw() is O(n); is_valid() is O(n²). Check winding first — projection of
    // real-world data preserves validity, and invalid polygons get filtered by
    // combine()'s is_valid() guard downstream.
    if polygon.exterior().is_ccw() {
        return Ok(geometry.clone());
    }

    let (mut exterior, interiors) = polygon.clone().into_inner();
    exterior.make_ccw_winding();
    let fixed = Polygon::new(exterior, interiors).simplify(1e-10_f64);

    // unsigned_area() is O(n) (shoelace formula). Valid CCW polygons always have positive
    // area; bow-ties and crossing rings have area ≈ 0. Use this as a cheap proxy to avoid
    // the O(n²) is_valid() call for the common case (valid-but-CW real-world polygons).
    if fixed.unsigned_area() > 0.0 {
        return Ok(Geometry::Polygon(fixed));
    }

    // Near-zero area: genuinely degenerate. Attempt self-union repair.
    unary_union(std::iter::once(&fixed))
        .0
        .into_iter()
        .max_by(|a, b| a.unsigned_area().total_cmp(&b.unsigned_area()))
        .filter(|p| p.is_valid())
        .map(|p| Ok(Geometry::Polygon(p)))
        .unwrap_or(Err(()))
}

/// Flattens, projects, and repairs a slice of geometries. Each geometry is first unwrapped into
/// primitives via iterate_geometry, then transformed by the given projection function, then
/// validated and repaired via fix_geometry. Invalid geometries that cannot be repaired are dropped.
/// Geometries for which the projection returns None are silently skipped.
pub fn iterate_normalized_geometry<F>(geometries: &[Geometry], project: F) -> Vec<Geometry>
where
    F: Fn(&Geometry) -> Option<Geometry> + Send + Sync,
{
    #[cfg(feature = "parallel")]
    {
        // into_par_iter requires an owned collection; collect the flat geometry first.
        let flat: Vec<Geometry> = geometries.iter().flat_map(iterate_geometry).collect();
        flat.into_par_iter()
            .filter_map(|g| project(&g))
            .filter_map(|g| fix_geometry(&g).ok())
            .collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        geometries
            .iter()
            .flat_map(iterate_geometry)
            .filter_map(|g| project(&g))
            .filter_map(|g| fix_geometry(&g).ok())
            .collect()
    }
}

/// Returns the point on the polygon's convex hull that is furthest from `line`.
/// The reference lines passed here are always longer than the geometry (see `rotated_envelope`),
/// so segment distance equals perpendicular distance for all real inputs.
fn furthest_from_line(hull: &Polygon, line: &Line) -> Point {
    hull.exterior()
        .points()
        .max_by(|a, b| {
            Euclidean
                .distance(a, line)
                .partial_cmp(&Euclidean.distance(b, line))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or_else(|| Point::from(line.start))
}

/// Returns the intersection point of two infinite lines, or None if they are parallel.
/// Solved via Cramer's rule on the system a1x + b1y = c1, a2x + b2y = c2.
fn lines_intersection(l1: &Line, l2: &Line) -> Option<Point> {
    let a1 = l1.end.y - l1.start.y;
    let b1 = l1.start.x - l1.end.x;
    let c1 = a1 * l1.start.x + b1 * l1.start.y;
    let a2 = l2.end.y - l2.start.y;
    let b2 = l2.start.x - l2.end.x;
    let c2 = a2 * l2.start.x + b2 * l2.start.y;
    let det = a1 * b2 - a2 * b1;
    if det.abs() < 1e-10 {
        return None;
    }
    Some(Point::new((c1 * b2 - c2 * b1) / det, (a1 * c2 - a2 * c1) / det))
}

/// Creates a line of given length centred on `center` at the given heading in degrees
/// (0 = N-S, 90 = E-W, increasing clockwise).
fn plot_line(center: &Point, length: f64, heading: f64) -> Line {
    let angle = (90.0_f64 - heading).to_radians();
    let ext_x = (length / 2.0) * angle.cos();
    let ext_y = (length / 2.0) * angle.sin();
    Line::new(
        coord! { x: center.x() - ext_x, y: center.y() - ext_y },
        coord! { x: center.x() + ext_x, y: center.y() + ext_y },
    )
}

/// Builds a rotated bounding rectangle aligned to `heading` by finding the extreme points
/// of the convex hull along the parallel and perpendicular axes, then intersecting the four
/// boundary lines to form the rectangle corners.
fn rotated_envelope(geometry: &Geometry, heading: f64, cache: Option<&ConvexHullCache>) -> Option<Polygon> {
    let Geometry::Polygon(source) = geometry else {
        return None;
    };
    // Use precomputed hull/centroid/length when available; otherwise compute fresh.
    let owned_hull;
    let (hull, centroid, length) = match cache {
        Some(c) => (&c.hull, c.centroid, c.length),
        None => {
            owned_hull = source.convex_hull();
            let bbox = source.bounding_rect()?;
            let centroid = owned_hull.centroid()?;
            (&owned_hull, centroid, 2.0 * (bbox.width() + bbox.height()))
        }
    };

    let ref_parallel = plot_line(&centroid, length, heading);
    let ref_perp = plot_line(&centroid, length, heading + 90.0);

    let side1 = plot_line(&furthest_from_line(hull, &ref_parallel), length, heading);
    let side2 = plot_line(&furthest_from_line(hull, &side1), length, heading);
    let side3 = plot_line(&furthest_from_line(hull, &ref_perp), length, heading + 90.0);
    let side4 = plot_line(&furthest_from_line(hull, &side3), length, heading + 90.0);

    let c0 = lines_intersection(&side1, &side3)?;
    let c1 = lines_intersection(&side2, &side3)?;
    let c2 = lines_intersection(&side2, &side4)?;
    let c3 = lines_intersection(&side1, &side4)?;

    Some(Polygon::new(
        LineString::from(vec![
            coord! { x: c0.x(), y: c0.y() },
            coord! { x: c1.x(), y: c1.y() },
            coord! { x: c2.x(), y: c2.y() },
            coord! { x: c3.x(), y: c3.y() },
            coord! { x: c0.x(), y: c0.y() },
        ]),
        vec![],
    ))
}

/// Returns the four boundary lines of the bounding rectangle of a given geometry.
/// When `heading` is `Some`, a rotated envelope aligned to that heading is used;
/// otherwise the minimum rotated rectangle is used.
/// Returns None if the geometry is degenerate (e.g. a single point or collinear set of points).
pub fn get_rectangular_boundary_lines(
    geometry: &Geometry,
    heading: Option<f64>,
    cache: Option<&ConvexHullCache>,
) -> Option<[Line; 4]> {
    if let Some(mut angle) = heading {
        // Exact multiples of 90° cause degenerate perpendicular line intersections; nudge slightly.
        if (angle % 360.0) % 90.0 == 0.0 {
            angle += 0.000001;
        }
        let rect = rotated_envelope(geometry, angle, cache)?;
        rect.exterior().lines().take(4).collect::<Vec<_>>().try_into().ok()
    } else {
        let mbr = MinimumRotatedRect::minimum_rotated_rect(geometry)?;
        mbr.exterior().lines().take(4).collect::<Vec<_>>().try_into().ok()
    }
}

/// Returns the two sides of the boundary lines rectangle used as roll lines.
/// When `heading` is `Some`, sides 0 and 2 are always chosen (they are perpendicular to
/// the heading direction by construction). Otherwise the shorter pair is chosen.
pub fn get_rectangular_shorter_sides(boundary_lines: [Line; 4], heading: Option<f64>) -> [Line; 2] {
    if heading.is_none() && Euclidean.length(&boundary_lines[0]) > Euclidean.length(&boundary_lines[1]) {
        [boundary_lines[1], boundary_lines[3]]
    } else {
        [boundary_lines[0], boundary_lines[2]]
    }
}

/// Ensures the given lines are all pointing downwards.
pub fn ensure_lines_pointing_down(lines: [Line; 2]) -> [Line; 2] {
    lines.map(|l| {
        if l.start.y < l.end.y {
            Line::new(l.end, l.start)
        } else {
            l
        }
    })
}

/// Sorts lines by the y component of their starting point, top-most first.
pub fn sort_by_highest_line(lines: [Line; 2]) -> [Line; 2] {
    let mut lines = lines;
    lines.sort_by(|a, b| b.start.y.partial_cmp(&a.start.y).unwrap_or(std::cmp::Ordering::Equal));
    lines
}

/// Returns the two roll-out sides of the bounding rectangle, pointing downward, top-most first.
/// When `heading` is `Some`, the rectangle is aligned to that angle and the sides
/// perpendicular to it are chosen; otherwise the minimum rotated rectangle is used.
/// Returns None if the geometry is degenerate and has no well-defined bounding rectangle.
pub fn roll_lines(geometry: &Geometry, heading: Option<f64>, cache: Option<&ConvexHullCache>) -> Option<[Line; 2]> {
    let boundary_lines = get_rectangular_boundary_lines(geometry, heading, cache)?;
    let shorter_boundary_lines = get_rectangular_shorter_sides(boundary_lines, heading);
    let shorter_boundary_lines_pointing_down = ensure_lines_pointing_down(shorter_boundary_lines);
    Some(sort_by_highest_line(shorter_boundary_lines_pointing_down))
}

/// Distributes a given number of points equidistantly on a line. Returns an empty vec for zero
/// points. Points sit at the center of each of `number_of_points` equal segments: fractional
/// positions 1/(2n), 3/(2n), …, (2n-1)/(2n).
pub fn distribute_points(line: &Line, number_of_points: u16) -> Vec<Point> {
    if number_of_points == 0 {
        return vec![];
    }
    let x_offset = line.end.x - line.start.x;
    let y_offset = line.end.y - line.start.y;
    let denom = f64::from(number_of_points) * 2.0;
    (0..number_of_points)
        .map(|i| {
            let frac = (f64::from(i) * 2.0 + 1.0) / denom;
            point! {
                x: line.start.x + frac * x_offset,
                y: line.start.y + frac * y_offset,
            }
        })
        .collect()
}

/// Returns the angle (in radians) of a given line in relation with the Y axis.
/// Horizontal lines (Δy == 0) return π/2 — including the zero-length case where angle is
/// mathematically undefined. Callers that need to distinguish degenerate input should check the
/// line length first.
pub fn get_angle(line: &Line) -> f64 {
    if line.end.y - line.start.y == 0. {
        return 0_f64.acos();
    }
    ((line.end.x - line.start.x) / (line.end.y - line.start.y)).atan()
}

/// Returns a new line with the same center and angle of a given line, but with different length.
/// Returns the line unchanged when `length` is negative or non-finite.
pub fn resize_line(line: &Line, length: f64) -> Line {
    if !length.is_finite() || length < 0.0 {
        return *line;
    }
    let angle = get_angle(line);
    let ext_x = length / 2. * angle.sin();
    let ext_y = length / 2. * angle.cos();
    let center = line.centroid();
    Line::new(
        coord! { x: center.x() + ext_x, y: center.y() + ext_y },
        coord! { x: center.x() - ext_x, y: center.y() - ext_y },
    )
}

/// Projects a strip of a given width along the length of a line.
/// Returns None when the buffer operation produces no polygon (e.g. non-positive `strip_width`
/// or degenerate line). Callers are expected to skip the result in that case.
pub fn project_strip(line: &Line, strip_width: f64) -> Option<Polygon> {
    if !strip_width.is_finite() || strip_width <= 0.0 {
        return None;
    }
    line.buffer_with_style(
        BufferStyle::new(strip_width / 2.0)
            .line_cap(LineCap::Butt)
            .line_join(LineJoin::Round(ROUND_ANGLE)),
    )
    .into_iter()
    .next()
}

/// Intersects a given line with a polygon, truncating the line at intersection points.
///
/// The function takes into account the given strip_width in projecting a strip
/// along the line before intersecting with the geometry applying the truncation
/// to the line.
pub fn intersect_line(geometry: &Polygon, full_line: &Line, strip_width: f64, min_strip_length: f64) -> Option<Line> {
    let full_line_length = Euclidean.length(full_line);
    if full_line_length < min_strip_length {
        return Some(*full_line);
    }
    let full_strip = project_strip(full_line, strip_width)?;
    let intersected_polygon: MultiPolygon = geometry.intersection(&full_strip);
    if intersected_polygon.is_empty() {
        return None;
    }
    let Some([roll_from, roll_to]) = roll_lines(&Geometry::Polygon(full_strip), None, None) else {
        return Some(*full_line);
    };
    let leading_distance = Euclidean.distance(&intersected_polygon, &roll_from);
    let trailing_distance = Euclidean.distance(&intersected_polygon, &roll_to);
    let mut partial_line = *full_line;
    if trailing_distance > 0. {
        partial_line = substring(full_line, 0., full_line_length - trailing_distance);
    }
    if leading_distance > 0. {
        let partial_line_length = Euclidean.length(&partial_line);
        partial_line = substring(&partial_line, leading_distance, partial_line_length);
    }
    Some(partial_line)
}

/// Returns the sub-segment of `line` between `start_dist` and `end_dist` measured along its length.
/// Negative distances are measured from the end of the line. Distances are not clamped: callers
/// are responsible for keeping them within `[0, length]` (or using the from-end convention).
/// Returns the line unchanged when either distance is NaN.
pub fn substring(line: &Line, start_dist: f64, end_dist: f64) -> Line {
    if start_dist.is_nan() || end_dist.is_nan() {
        return *line;
    }
    let start = if start_dist >= 0. {
        line.point_at_distance_from_start(&Euclidean, start_dist)
    } else {
        line.point_at_distance_from_end(&Euclidean, -start_dist)
    };
    let end = if end_dist >= 0. {
        line.point_at_distance_from_start(&Euclidean, end_dist)
    } else {
        line.point_at_distance_from_end(&Euclidean, -end_dist)
    };
    Line::new(start, end)
}

/// Segments lines to a given maximum strip length. When `max_strip_length` or `strip_width` is
/// non-positive or non-finite the input lines are returned unchanged — the loop below depends on
/// both being positive to make progress.
pub fn segment_lines(lines: &[Line], max_strip_length: f64, strip_width: f64) -> Vec<Line> {
    if !max_strip_length.is_finite() || max_strip_length <= 0.0 || !strip_width.is_finite() || strip_width <= 0.0 {
        return lines.to_vec();
    }
    let mut segmented_lines: Vec<Line> = vec![];
    for input_line in lines {
        let mut line = *input_line;
        let mut line_length = Euclidean.length(&line);
        'outer: loop {
            if line_length <= (max_strip_length + strip_width) {
                segmented_lines.push(line);
                break;
            }
            while line_length > max_strip_length {
                segmented_lines.push(substring(&line, 0., max_strip_length));
                let next = substring(&line, max_strip_length, line_length);
                let next_length = Euclidean.length(&next);
                if next_length >= line_length {
                    // Floating-point stall: no progress — push remainder and stop.
                    segmented_lines.push(next);
                    break 'outer;
                }
                line = next;
                line_length = next_length;
            }
        }
    }
    segmented_lines
}

pub struct PointAndDistance {
    pub point: Point,
    pub distance: f64,
}

/// Calculates the furthest point and distance from the polygon's centroid.
pub fn furthest_from_centroid(polygon: &Polygon) -> PointAndDistance {
    let Some(centroid) = polygon.centroid() else {
        return PointAndDistance {
            point: Point::new(0., 0.),
            distance: 0.,
        };
    };
    let mut furthest_distance: f64 = 0.;
    let mut furthest_point: Point = centroid;
    for point in polygon.exterior().points() {
        let distance_from_centroid = Euclidean.distance(point, centroid);
        if distance_from_centroid > furthest_distance {
            furthest_distance = distance_from_centroid;
            furthest_point = point;
        }
    }
    PointAndDistance {
        point: furthest_point,
        distance: furthest_distance,
    }
}

/// Hard cap on shard count to prevent unbounded work with pathological inputs.
const MAX_SHARDS: usize = 10_000;

/// Streaming iterator over the shards of a polygon. One step of the sharding loop produces zero
/// or more shards (the intersection of the current remainder with a circular zone around its
/// furthest point); those are buffered in `pending` and drained before the loop advances again.
/// `remaining` holds pieces still to be sharded; when it and `pending` are both empty, the
/// iterator is exhausted. Multiple pieces arise when `difference` splits the remainder.
pub struct ShardIterator {
    remaining: std::collections::VecDeque<Polygon>,
    pending: std::collections::VecDeque<Polygon>,
    shard_radius: f64,
    shards_produced: usize,
}

impl Iterator for ShardIterator {
    type Item = Polygon;

    fn next(&mut self) -> Option<Polygon> {
        loop {
            if let Some(shard) = self.pending.pop_front() {
                self.shards_produced += 1;
                return Some(shard);
            }
            let remaining = self.remaining.pop_front()?;
            if self.shards_produced >= MAX_SHARDS {
                // Cap reached: emit this piece and drain the rest as-is so nothing is lost.
                self.pending.extend(self.remaining.drain(..));
                return Some(remaining);
            }
            let furthest_pd = furthest_from_centroid(&remaining);
            if furthest_pd.distance <= self.shard_radius {
                return Some(remaining);
            }
            let Some(shard_zone) = furthest_pd
                .point
                .buffer_with_style(
                    BufferStyle::new(self.shard_radius)
                        .line_cap(LineCap::Round(ROUND_ANGLE))
                        .line_join(LineJoin::Round(ROUND_ANGLE)),
                )
                .into_iter()
                .next()
            else {
                // Buffer failed (degenerate point) — emit this piece as-is and continue.
                return Some(remaining);
            };
            let intersected: MultiPolygon = remaining.intersection(&shard_zone);
            for semi_shard in iterate_polygons(&Geometry::MultiPolygon(intersected)) {
                self.pending.push_back(semi_shard);
            }
            self.remaining.extend(remaining.difference(&shard_zone));
            // Loop back to drain `pending` (or finish if both `pending` and `remaining` are empty).
        }
    }
}

/// Splits a polygon into smaller compact shards by iteratively intersecting the polygon with a
/// circular buffer around its furthest point from the centroid, removing that region, and repeating
/// on the remainder until no point is further than shard_radius from the centroid.
/// Sharding is skipped when the polygon is already compact enough: the density threshold is the
/// polygon's area divided by the area of its convex hull compared against shard_density_ratio.
/// Sharding is also skipped when `shard_radius` or `shard_density_ratio` is non-finite or when
/// `shard_radius <= 0` — in those cases the whole polygon is yielded as a single shard.
pub fn iterate_shards(geometry: &Polygon, shard_radius: f64, shard_density_ratio: f64) -> ShardIterator {
    let skip_sharding = !shard_radius.is_finite() || shard_radius <= 0.0 || !shard_density_ratio.is_finite() || {
        let convex_hull_area = geometry.convex_hull().unsigned_area();
        convex_hull_area == 0.0 || geometry.unsigned_area() / convex_hull_area > shard_density_ratio
    };
    // Skip-path yields the whole polygon once via `pending`; shard-path seeds `remaining` so the
    // iterator's `next()` loop drives the sharding steps.
    let (remaining, pending) = if skip_sharding {
        (
            std::collections::VecDeque::new(),
            std::collections::VecDeque::from([geometry.clone()]),
        )
    } else {
        (
            std::collections::VecDeque::from([geometry.clone()]),
            std::collections::VecDeque::new(),
        )
    };
    ShardIterator {
        remaining,
        pending,
        shard_radius,
        shards_produced: 0,
    }
}

/// Checks whether a polygon is too small for the given minimum strip length. The test is the
/// polygon's bbox diagonal vs `min_strip_length` — a cheap proxy for "does this polygon have room
/// for a full-length strip along any orientation." A degenerate polygon with no bounding rect is
/// treated as not-too-small (conservative — leaves it to downstream checks).
pub fn is_too_small(polygon: &Polygon, min_strip_length: f64) -> bool {
    let Some(rect) = polygon.bounding_rect() else {
        return false;
    };
    Euclidean.distance(Point::from(rect.min()), Point::from(rect.max())) < min_strip_length
}

/// Ensures a given line has at least a given minimum length.
pub fn ensure_line_length(line: &Line, min_length: f64) -> Line {
    if Euclidean.length(line) < min_length {
        resize_line(line, min_length)
    } else {
        *line
    }
}

/// Converts a geometry to a Polygon using the given size as the buffer radius.
/// Points are buffered into a circle, Lines are projected into a rectangular strip.
/// Polygons are returned unchanged.
///
/// Returns None when the buffer cannot produce a polygon: non-finite or non-positive `size`,
/// `size < 2 * CIRCLE_EXPANSION_CORRECTION` for Points (circle radius would be non-positive),
/// or an underlying geo buffer returning empty.
pub(crate) fn coerce_to_polygon(geometry: &Geometry, size: f64) -> Option<Polygon> {
    if !size.is_finite() || size <= 0.0 {
        return None;
    }
    match geometry {
        Geometry::Polygon(g) => Some(g.clone()),
        Geometry::Line(g) => project_strip(g, size),
        Geometry::Point(g) => {
            let radius = (size / 2.) - CIRCLE_EXPANSION_CORRECTION;
            if radius <= 0.0 {
                return None;
            }
            g.buffer_with_style(
                BufferStyle::new(radius)
                    .line_cap(LineCap::Round(ROUND_ANGLE))
                    .line_join(LineJoin::Round(ROUND_ANGLE)),
            )
            .into_iter()
            .next()
        }
        // Only reachable internally; iterate_normalized_geometry only yields Polygon, Line, Point.
        _ => unreachable!(),
    }
}

/// Returns a polygon with CCW-wound exterior and no interior rings. This is the shape tessellation
/// intermediates and coverages take when preparing for GeoJSON output (GeoJSON spec RFC-7946 §3.1.6
/// requires CCW exteriors) — holes are always dropped because tessellation outputs are simple.
pub fn polygon_ccw_no_holes(polygon: &Polygon) -> Polygon {
    let mut exterior = polygon.exterior().clone();
    exterior.make_ccw_winding();
    Polygon::new(exterior, vec![])
}

/// Enforces the GeoJSON "right hand rule" on Polygons and MultiPolygons: CCW exteriors with holes
/// dropped. Other geometry types pass through unchanged.
pub fn right_hand_rule(geometry: &Geometry) -> Geometry {
    match geometry {
        Geometry::Polygon(g) => Geometry::Polygon(polygon_ccw_no_holes(g)),
        Geometry::MultiPolygon(g) => {
            Geometry::MultiPolygon(MultiPolygon::new(g.iter().map(polygon_ccw_no_holes).collect()))
        }
        _ => geometry.clone(),
    }
}

/// Returns the segments of `lines` that lie outside all `polygons`.
pub fn clip_lines_by_polygons(lines: &[LineString], polygons: &[Polygon]) -> Vec<LineString> {
    if lines.is_empty() || polygons.is_empty() {
        return lines.to_vec();
    }
    let multi_line = MultiLineString::new(lines.to_vec());
    let multi_poly = MultiPolygon::new(polygons.to_vec());
    multi_poly.clip(&multi_line, true).into_iter().collect()
}

/// Returns strips needed to cover `roll_line_length` at the given `strip_width` and `min_overlap`,
/// or `None` for invalid inputs.
pub fn count_lines(roll_line_length: f64, strip_width: f64, min_overlap: f64) -> Option<u16> {
    if !roll_line_length.is_finite() || !strip_width.is_finite() || !min_overlap.is_finite() {
        return None;
    }
    if strip_width <= 0.0 || min_overlap >= (strip_width / 2.0) {
        return None;
    }
    let strips_float = (roll_line_length / strip_width).ceil();
    if !strips_float.is_finite() || strips_float > u16::MAX as f64 {
        return None;
    }
    let strips_with_no_overlap = strips_float as u16;
    // A degenerate roll line (zero or near-zero length) still needs at least one strip;
    // avoids the overlap_zones underflow below.
    if strips_with_no_overlap == 0 {
        return Some(1);
    }
    let overlap_zones = strips_with_no_overlap - 1;
    let limit = (strip_width * strips_with_no_overlap as f64) - (min_overlap * overlap_zones as f64);
    if roll_line_length > limit {
        strips_with_no_overlap.checked_add(1)
    } else {
        Some(strips_with_no_overlap)
    }
}

#[cfg(test)]
#[path = "./geo_utils_tests.rs"]
mod test_geo_utils;
