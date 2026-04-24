use geo::{Coord, LineString, Point, Polygon};
use js_sys::JSON;
use serde::Serialize;
use serde_json::Value;
use sfogliatrice_lib::defaults::{
    DEFAULT_MAX_STRIP_LENGTH, DEFAULT_MIN_OVERLAP, DEFAULT_SHARD_RADIUS, DEFAULT_STRIP_WIDTH,
    DEFAULT_TARGET_EXPANSION,
};
use sfogliatrice_lib::{tessellate_geojson, Config, Target};
use wasm_bindgen::prelude::*;

#[derive(Serialize)]
struct WasmResult {
    targets: Value,
    coverages: Value,
    intermediates: Value,
}

fn coords_to_vec(coords: &[Coord<f64>]) -> Vec<Vec<f64>> {
    coords.iter().map(|c| vec![c.x, c.y]).collect()
}

fn feature(geometry: Value) -> Value {
    serde_json::json!({"type": "Feature", "geometry": geometry, "properties": null})
}

fn feature_collection(features: Vec<Value>) -> Value {
    serde_json::json!({"type": "FeatureCollection", "features": features})
}

fn polygon_to_feature(poly: Polygon<f64>) -> Value {
    let mut rings = vec![coords_to_vec(&poly.exterior().0)];
    rings.extend(poly.interiors().iter().map(|r| coords_to_vec(&r.0)));
    feature(serde_json::json!({"type": "Polygon", "coordinates": rings}))
}

fn target_to_feature(target: Target) -> Value {
    match target {
        Target::Point(Point(c)) => {
            feature(serde_json::json!({"type": "Point", "coordinates": [c.x, c.y]}))
        }
        Target::Line(LineString(coords)) => {
            feature(serde_json::json!({"type": "LineString", "coordinates": coords_to_vec(&coords)}))
        }
    }
}

/// Tessellate a GeoJSON geometry into targets, coverages, and intermediates.
///
/// All length/distance parameters are in **meters**. Pass `undefined` (or omit in
/// bundler wrappers) to use the default value for that parameter.
///
/// # Arguments
/// - `geojson` – GeoJSON object (any geometry, feature, or feature collection)
/// - `strip_width` – width of each survey strip (default: 5 000 m)
/// - `min_strip_length` – minimum strip length before merging (default: 5 000 m)
/// - `max_strip_length` – maximum strip length before splitting (default: 50 000 m)
/// - `min_overlap` – minimum overlap between adjacent strips (default: 200 m)
/// - `expansion` – outward expansion applied to point targets (default: 5 000 m)
/// - `shard_density_ratio` – internal shard density ratio (default: 0.3)
/// - `shard_radius` – radius used for shard clustering (default: 50 000 m)
/// - `force_line_targets` – always produce line targets even for small geometries (default: false)
/// - `force_square_coverages` – always produce square coverage boxes (default: false)
/// - `heading` – fixed heading angle in degrees, or `undefined` for auto (default: undefined)
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
) -> Result<JsValue, JsValue> {
    let input_str = JSON::stringify(&geojson)
        .map_err(|_| JsValue::from_str("failed to stringify GeoJSON input"))?
        .as_string()
        .ok_or_else(|| JsValue::from_str("GeoJSON input is not a string"))?;
    let geojson_value: Value =
        serde_json::from_str(&input_str).map_err(|e| JsValue::from_str(&e.to_string()))?;

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

    // These two fields are not validated by Config::new; apply overrides if provided.
    if let Some(v) = min_strip_length { config.min_strip_length = v; }
    if let Some(v) = shard_density_ratio { config.shard_density_ratio = v; }

    let result = tessellate_geojson(&geojson_value, &config);

    let wasm_result = WasmResult {
        targets: feature_collection(result.targets.into_iter().map(target_to_feature).collect()),
        coverages: feature_collection(result.coverages.into_iter().map(polygon_to_feature).collect()),
        intermediates: feature_collection(result.intermediates.into_iter().map(polygon_to_feature).collect()),
    };

    let output_str =
        serde_json::to_string(&wasm_result).map_err(|e| JsValue::from_str(&e.to_string()))?;
    JSON::parse(&output_str).map_err(|_| JsValue::from_str("failed to parse output JSON"))
}
