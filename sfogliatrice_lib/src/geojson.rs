use geo::{Geometry, Polygon, Winding};
use geojson::GeoJson;
use serde_json::{Value, json};

/// Maximum nesting depth for GeoJSON container recursion (Feature, FeatureCollection, GeometryCollection).
/// Inputs nested deeper than this have their excess levels silently dropped.
const MAX_GEOJSON_DEPTH: usize = 4;

fn iterate_geojson_inner(geojson: &Value, depth: usize) -> Vec<Value> {
    let geo_type = geojson.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match geo_type {
        "Feature" => {
            if depth >= MAX_GEOJSON_DEPTH {
                return vec![];
            }
            geojson
                .get("geometry")
                .map(|g| iterate_geojson_inner(g, depth + 1))
                .unwrap_or_default()
        }
        "Point" | "LineString" | "Polygon" => vec![geojson.clone()],
        "MultiPoint" | "MultiLineString" | "MultiPolygon" => {
            let single_type = geo_type.strip_prefix("Multi").unwrap_or(geo_type);
            geojson
                .get("coordinates")
                .and_then(|c| c.as_array())
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .map(|c| json!({"type": single_type, "coordinates": c}))
                .collect()
        }
        "GeometryCollection" => {
            if depth >= MAX_GEOJSON_DEPTH {
                return vec![];
            }
            geojson
                .get("geometries")
                .and_then(|g| g.as_array())
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .flat_map(|g| iterate_geojson_inner(g, depth + 1))
                .collect()
        }
        "FeatureCollection" => {
            if depth >= MAX_GEOJSON_DEPTH {
                return vec![];
            }
            geojson
                .get("features")
                .and_then(|f| f.as_array())
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .flat_map(|f| {
                    f.get("geometry")
                        .map(|g| iterate_geojson_inner(g, depth + 1))
                        .unwrap_or_default()
                })
                .collect()
        }
        _ => vec![],
    }
}

/// Iterates a GeoJSON Value into its single primitives: Points, LineStrings and Polygons.
/// Nesting beyond MAX_GEOJSON_DEPTH is silently dropped to prevent stack exhaustion.
pub fn iterate_geojson(geojson: &Value) -> Vec<Value> {
    iterate_geojson_inner(geojson, 0)
}

/// Iterates geometries from GeoJSON, converting each primitive to a geo::Geometry.
pub fn iterate_geometry_from_geojson(geojson: &Value) -> Vec<Geometry> {
    iterate_geojson(geojson)
        .into_iter()
        .filter_map(|v| {
            let gj: GeoJson = serde_json::from_value(v).ok()?;
            match gj {
                GeoJson::Geometry(geom) => geo::Geometry::try_from(geom).ok(),
                _ => None,
            }
        })
        .collect()
}

/// Wraps a geometry JSON value into a GeoJSON Feature object.
pub fn feature(geometry: &Value) -> Value {
    json!({"type": "Feature", "properties": {}, "geometry": geometry})
}

/// Returns a FeatureCollection wrapping the given features.
pub fn feature_collection(features: Vec<Value>) -> Value {
    json!({"type": "FeatureCollection", "features": features})
}

/// Returns a FeatureCollection with features collected from multiple GeoJSON objects.
pub fn combine_geojson(geojsons: &[Value]) -> Value {
    let features: Vec<Value> = geojsons.iter().flat_map(iterate_geojson).map(|g| feature(&g)).collect();
    feature_collection(features)
}

/// Iterates over features reducing the precision of their coordinates.
pub fn coord_precision(features: Vec<Value>, precision: u32) -> Vec<Value> {
    let factor = 10f64.powi(precision as i32);
    features
        .into_iter()
        .map(|mut f| {
            if let Some(coords) = f.get_mut("geometry").and_then(|g| g.get_mut("coordinates")) {
                *coords = set_precision(coords, factor);
            }
            f
        })
        .collect()
}

fn set_precision(coords: &Value, factor: f64) -> Value {
    match coords {
        Value::Number(n) => {
            if let Some(v) = n.as_f64() {
                let rounded = (v * factor).round() / factor;
                serde_json::Number::from_f64(rounded)
                    .map(Value::Number)
                    .unwrap_or_else(|| Value::Number(n.clone()))
            } else {
                Value::Number(n.clone())
            }
        }
        Value::Array(arr) => Value::Array(arr.iter().map(|v| set_precision(v, factor)).collect()),
        _ => coords.clone(),
    }
}

/// Serializes a polygon as a GeoJSON Feature, enforcing RFC-7946 winding in one shot:
/// exterior CCW, each interior ring CW. Works correctly whether or not the polygon has holes.
pub fn polygon_feature_rfc7946(polygon: Polygon) -> Value {
    let (mut exterior, interiors) = polygon.into_inner();
    exterior.make_ccw_winding();
    let interiors = interiors
        .into_iter()
        .map(|mut ring| {
            ring.make_cw_winding();
            ring
        })
        .collect();
    feature(&geometry_to_json(&Geometry::Polygon(Polygon::new(exterior, interiors))))
}

/// Serializes a `geo::Geometry` as a GeoJSON geometry object (a `serde_json::Value`).
pub fn geometry_to_json(geometry: &Geometry) -> Value {
    let gj_geom = geojson::Geometry::from(geometry);
    serde_json::to_value(gj_geom).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_iterate_geojson_simple_primitives() {
        for geo_type in ["Point", "LineString", "Polygon"] {
            let v = json!({ "type": geo_type, "coordinates": [] });
            let out = iterate_geojson(&v);
            assert_eq!(out.len(), 1, "{geo_type} passes through as-is");
            assert_eq!(out[0], v);
        }
    }

    #[test]
    fn test_iterate_geojson_multipoint_splits() {
        let v = json!({ "type": "MultiPoint", "coordinates": [[0., 0.], [1., 1.], [2., 2.]] });
        let out = iterate_geojson(&v);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0]["type"], "Point");
        assert_eq!(out[0]["coordinates"], json!([0., 0.]));
    }

    #[test]
    fn test_iterate_geojson_multilinestring_splits() {
        let v = json!({
            "type": "MultiLineString",
            "coordinates": [[[0., 0.], [1., 1.]], [[2., 2.], [3., 3.]]],
        });
        let out = iterate_geojson(&v);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|g| g["type"] == "LineString"));
    }

    #[test]
    fn test_iterate_geojson_multipolygon_splits() {
        let v = json!({
            "type": "MultiPolygon",
            "coordinates": [
                [[[0., 0.], [1., 0.], [1., 1.], [0., 0.]]],
                [[[2., 2.], [3., 2.], [3., 3.], [2., 2.]]],
            ],
        });
        let out = iterate_geojson(&v);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|g| g["type"] == "Polygon"));
    }

    #[test]
    fn test_iterate_geojson_geometry_collection_unwraps() {
        let v = json!({
            "type": "GeometryCollection",
            "geometries": [
                { "type": "Point", "coordinates": [0., 0.] },
                { "type": "LineString", "coordinates": [[0., 0.], [1., 1.]] },
            ],
        });
        let out = iterate_geojson(&v);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["type"], "Point");
        assert_eq!(out[1]["type"], "LineString");
    }

    #[test]
    fn test_iterate_geojson_feature_and_collection() {
        let feat = json!({
            "type": "Feature",
            "properties": {},
            "geometry": { "type": "Point", "coordinates": [1., 2.] },
        });
        assert_eq!(iterate_geojson(&feat).len(), 1);

        let fc = json!({
            "type": "FeatureCollection",
            "features": [feat.clone(), feat.clone(), feat.clone()],
        });
        assert_eq!(iterate_geojson(&fc).len(), 3);
    }

    #[test]
    fn test_iterate_geojson_unknown_type_is_empty() {
        let v = json!({ "type": "NotAGeoJSONType" });
        assert!(iterate_geojson(&v).is_empty());
    }

    #[test]
    fn test_iterate_geojson_depth_cap_drops_deep_nesting() {
        let point = json!({ "type": "Point", "coordinates": [0., 0.] });
        let gc4 = json!({ "type": "GeometryCollection", "geometries": [point] });
        let gc3 = json!({ "type": "GeometryCollection", "geometries": [gc4] });
        let gc2 = json!({ "type": "GeometryCollection", "geometries": [gc3] });
        let gc1 = json!({ "type": "GeometryCollection", "geometries": [gc2] });
        let outer = json!({ "type": "Feature", "properties": {}, "geometry": gc1 });
        assert!(iterate_geojson(&outer).is_empty(), "deep nesting must be dropped");
    }

    #[test]
    fn test_iterate_geojson_shallow_nesting_preserved() {
        let point = json!({ "type": "Point", "coordinates": [0., 0.] });
        let gc3 = json!({ "type": "GeometryCollection", "geometries": [point] });
        let gc2 = json!({ "type": "GeometryCollection", "geometries": [gc3] });
        let gc1 = json!({ "type": "GeometryCollection", "geometries": [gc2] });
        let outer = json!({ "type": "Feature", "properties": {}, "geometry": gc1 });
        let out = iterate_geojson(&outer);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "Point");
    }

    #[test]
    fn test_coord_precision_rounds_to_given_decimals() {
        let feat = json!({
            "type": "Feature",
            "properties": {},
            "geometry": { "type": "Point", "coordinates": [1.123456789, -2.987654321] },
        });
        let rounded = coord_precision(vec![feat], 3);
        let coords = &rounded[0]["geometry"]["coordinates"];
        assert_eq!(coords[0].as_f64().unwrap(), 1.123);
        assert_eq!(coords[1].as_f64().unwrap(), -2.988);
    }

    #[test]
    fn test_coord_precision_zero_rounds_to_integer() {
        let feat = json!({
            "type": "Feature",
            "properties": {},
            "geometry": { "type": "Point", "coordinates": [1.7, 2.3] },
        });
        let rounded = coord_precision(vec![feat], 0);
        let coords = &rounded[0]["geometry"]["coordinates"];
        assert_eq!(coords[0].as_f64().unwrap(), 2.0);
        assert_eq!(coords[1].as_f64().unwrap(), 2.0);
    }

    #[test]
    fn test_coord_precision_recurses_into_multipolygon() {
        let feat = json!({
            "type": "Feature",
            "properties": {},
            "geometry": {
                "type": "MultiPolygon",
                "coordinates": [[[[1.123456, 2.123456], [3.123456, 4.123456]]]],
            },
        });
        let rounded = coord_precision(vec![feat], 2);
        let p = &rounded[0]["geometry"]["coordinates"][0][0][0];
        assert_eq!(p[0].as_f64().unwrap(), 1.12);
        assert_eq!(p[1].as_f64().unwrap(), 2.12);
    }

    #[test]
    fn test_combine_geojson_flattens_multiple_inputs() {
        let a = json!({ "type": "Point", "coordinates": [0., 0.] });
        let b = json!({
            "type": "MultiPoint",
            "coordinates": [[1., 1.], [2., 2.]],
        });
        let combined = combine_geojson(&[a, b]);
        assert_eq!(combined["type"], "FeatureCollection");
        assert_eq!(combined["features"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_feature_wrapper_shape() {
        let geom = json!({ "type": "Point", "coordinates": [1., 2.] });
        let f = feature(&geom);
        assert_eq!(f["type"], "Feature");
        assert_eq!(f["geometry"], geom);
        assert!(f["properties"].is_object());
    }

    #[test]
    fn test_feature_collection_shape() {
        let f1 = feature(&json!({ "type": "Point", "coordinates": [0., 0.] }));
        let f2 = feature(&json!({ "type": "Point", "coordinates": [1., 1.] }));
        let fc = feature_collection(vec![f1.clone(), f2.clone()]);
        assert_eq!(fc["type"], "FeatureCollection");
        let arr = fc["features"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], f1);
        assert_eq!(arr[1], f2);
    }

    #[test]
    fn test_feature_collection_empty() {
        let fc = feature_collection(vec![]);
        assert_eq!(fc["type"], "FeatureCollection");
        assert_eq!(fc["features"].as_array().unwrap().len(), 0);
    }
}
