use std::f64::consts::PI;

pub const DEFAULT_SHARD_DENSITY_RATIO: f64 = 0.3;
/// Empirical margin (meters) subtracted from a Point's circle-buffer radius in `coerce_to_polygon`.
/// The round buffer is a regular `ROUND_VERTICES`-gon inscribed in the requested radius — its edges
/// sit slightly inside the true circle, so for `ROUND_VERTICES = 36` and radii on the order of a
/// few km the polygon's max diameter overshoots the intended expansion by ~0.4% × radius. 100 m
/// covers this slack at default `DEFAULT_TARGET_EXPANSION = 5000 m` without making small points
/// disappear (guarded by the `radius <= 0.0` check at the call site).
pub const CIRCLE_EXPANSION_CORRECTION: f64 = 100.0;

pub const DEFAULT_STRIP_WIDTH: f64 = 5_000.0; // In meters
pub const DEFAULT_MIN_STRIP_LENGTH: f64 = 5_000.0; // In meters
pub const DEFAULT_MAX_STRIP_LENGTH: f64 = 50_000.0; // In meters
pub const DEFAULT_MIN_OVERLAP: f64 = 200.0; // In meters
pub const DEFAULT_SHARD_RADIUS: f64 = 50_000.0; // In meters
pub const DEFAULT_TARGET_EXPANSION: f64 = 5_000.0; // In meters

// Calculates the angle in radians that makes a point buffered with LineJoin::Round return a Polygon with 36 vertices:
pub const ROUND_VERTICES: usize = 36;
pub const ROUND_ANGLE: f64 = PI / 180. * (360. / ROUND_VERTICES as f64);
