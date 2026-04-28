use js_sys::JSON;
use serde_json::Value;
use sfogliatrice_lib::defaults::{
    DEFAULT_MAX_STRIP_LENGTH, DEFAULT_MIN_OVERLAP, DEFAULT_SHARD_RADIUS, DEFAULT_STRIP_WIDTH, DEFAULT_TARGET_EXPANSION,
};
use sfogliatrice_lib::{Config, tessellate_geojson_to_geojson};
use wasm_bindgen::prelude::*;

/// Tessellate a GeoJSON geometry into targets, coverages, and intermediates.
///
/// All length/distance parameters are in **meters**. Pass `undefined` (or omit in
/// bundler wrappers) to use the default value for that parameter.
///
/// # Arguments
/// - `geojson` – GeoJSON object (any geometry, feature, or feature collection)
/// - `strip_width` – Width of each survey strip, in meters. (default: 5 000 m)
/// - `min_strip_length` – Minimum strip length before two strips are merged, in meters. (default: 5 000 m)
/// - `max_strip_length` – Maximum strip length before a strip is split, in meters. (default: 50 000 m)
/// - `min_overlap` – Minimum overlap between adjacent strips, in meters. (default: 200 m)
/// - `expansion` – Buffer applied to Points and LineStrings before merging, in meters. (default: 5 000 m)
/// - `shard_density_ratio` – Fraction of `shard_radius` used as the grid cell size when sharding large intermediates. (default: 0.3)
/// - `shard_radius` – Maximum radius of a shard cluster before an intermediate is split, in meters. (default: 50 000 m)
/// - `force_line_targets` – Always emit line targets even when the geometry is small enough for a point target. (default: false)
/// - `force_square_coverages` – Always emit square coverage for Points instead of circles. (default: false)
/// - `heading` – Fixed strip heading in degrees; empty lets the algorithm choose the optimal angle. (default: undefined)
/// - `brute_force` – Try all headings 0–179° and pick the one with fewest targets; slow but optimal. (default: false)
#[allow(clippy::too_many_arguments)]
#[wasm_bindgen]
pub fn tessellate(
    geojson: JsValue,
    strip_width: Option<f64>,
    min_strip_length: Option<f64>,
    max_strip_length: Option<f64>,
    min_overlap: Option<f64>,
    expansion: Option<f64>,
    shard_density_ratio: Option<f64>,
    shard_radius: Option<f64>,
    force_line_targets: Option<bool>,
    force_square_coverages: Option<bool>,
    heading: Option<f64>,
    brute_force: Option<bool>,
) -> Result<JsValue, JsValue> {
    let input_str = JSON::stringify(&geojson)
        .map_err(|_| JsValue::from_str("failed to stringify GeoJSON input"))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("GeoJSON input is not a string"))?;
    let geojson_value: Value = serde_json::from_str(&input_str).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let mut config = Config::new(
        expansion.unwrap_or(DEFAULT_TARGET_EXPANSION),
        strip_width.unwrap_or(DEFAULT_STRIP_WIDTH),
        max_strip_length.unwrap_or(DEFAULT_MAX_STRIP_LENGTH),
        min_overlap.unwrap_or(DEFAULT_MIN_OVERLAP),
        force_line_targets.unwrap_or(false),
        force_square_coverages.unwrap_or(false),
        shard_radius.unwrap_or(DEFAULT_SHARD_RADIUS),
        heading,
    )
    .map_err(|e| JsValue::from_str(&e.to_string()))?;

    // These fields are not validated by Config::new; apply overrides if provided.
    if let Some(v) = min_strip_length {
        config.min_strip_length = v;
    }
    if let Some(v) = shard_density_ratio {
        config.shard_density_ratio = v;
    }
    if let Some(v) = brute_force {
        config.brute_force = v;
    }

    let result = tessellate_geojson_to_geojson(&geojson_value, &config);

    let output_str = serde_json::to_string(&result).map_err(|e| JsValue::from_str(&e.to_string()))?;
    JSON::parse(&output_str).map_err(|_| JsValue::from_str("failed to parse output JSON"))
}
