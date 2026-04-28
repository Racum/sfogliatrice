use serde::Serialize;
use serde_json::Value;

use crate::defaults::{
    DEFAULT_MAX_STRIP_LENGTH, DEFAULT_MIN_OVERLAP, DEFAULT_MIN_STRIP_LENGTH, DEFAULT_SHARD_DENSITY_RATIO,
    DEFAULT_SHARD_RADIUS, DEFAULT_STRIP_WIDTH, DEFAULT_TARGET_EXPANSION,
};
use geo::{LineString, Point, Polygon};

const MAX_EXPANSION: f64 = 1_000_000.0; // 1,000 km
const MAX_STRIP_WIDTH: f64 = 5_000_000.0; // 5,000 km
const MAX_STRIP_LENGTH: f64 = 5_000_000.0; // 5,000 km

#[derive(Debug, Clone)]
pub enum Target {
    Point(Point),
    Line(LineString),
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Width of each survey strip, in meters.
    pub strip_width: f64,
    /// Minimum strip length before two strips are merged, in meters.
    pub min_strip_length: f64,
    /// Maximum strip length before a strip is split, in meters.
    pub max_strip_length: f64,
    /// Minimum overlap between adjacent strips, in meters.
    pub min_overlap: f64,
    /// Buffer applied to Points and LineStrings before merging, in meters.
    pub expansion: f64,
    /// Fraction of `shard_radius` used as the grid cell size when sharding large intermediates.
    pub shard_density_ratio: f64,
    /// Maximum radius of a shard cluster before an intermediate is split, in meters.
    pub shard_radius: f64,
    /// Always emit line targets even when the geometry is small enough for a point target.
    pub force_line_targets: bool,
    /// Always emit square coverage for Points instead of circles.
    pub force_square_coverages: bool,
    /// Fixed strip heading in degrees; empty lets the algorithm choose the optimal angle.
    pub heading: Option<f64>,
    /// When true and heading is None, iterate 0–179° and pick the heading that produces the
    /// fewest targets (tiebreaker: lowest coverage overfitting vs the input area).
    pub brute_force: bool,
}

/// Describes which parameter failed validation and why.
#[derive(Debug, PartialEq)]
pub enum ConfigError {
    ExpansionNotPositive,
    StripWidthNotPositive,
    MaxStripLengthNotPositive,
    ExpansionTooLarge,
    StripWidthTooLarge,
    MaxStripLengthTooLarge,
    MinOverlapTooLarge,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::ExpansionNotPositive => {
                write!(f, "Expansion must be a positive finite number.")
            }
            ConfigError::StripWidthNotPositive => {
                write!(f, "Width must be a positive finite number.")
            }
            ConfigError::MaxStripLengthNotPositive => {
                write!(f, "Maximum length must be a positive finite number.")
            }
            ConfigError::ExpansionTooLarge => {
                write!(f, "Expansion exceeds maximum of {}Km.", MAX_EXPANSION / 1000.0)
            }
            ConfigError::StripWidthTooLarge => {
                write!(f, "Width exceeds maximum of {}Km.", MAX_STRIP_WIDTH / 1000.0)
            }
            ConfigError::MaxStripLengthTooLarge => {
                write!(f, "Maximum length exceeds maximum of {}Km.", MAX_STRIP_LENGTH / 1000.0)
            }
            ConfigError::MinOverlapTooLarge => {
                write!(f, "Minimum overlap must be less than half of --width.")
            }
        }
    }
}

impl Config {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        expansion: f64,
        strip_width: f64,
        max_strip_length: f64,
        min_overlap: f64,
        force_line_targets: bool,
        force_square_coverages: bool,
        shard_radius: f64,
        heading: Option<f64>,
    ) -> Result<Self, ConfigError> {
        if !expansion.is_finite() || expansion <= 0.0 {
            return Err(ConfigError::ExpansionNotPositive);
        }
        if expansion > MAX_EXPANSION {
            return Err(ConfigError::ExpansionTooLarge);
        }
        if !strip_width.is_finite() || strip_width <= 0.0 {
            return Err(ConfigError::StripWidthNotPositive);
        }
        if strip_width > MAX_STRIP_WIDTH {
            return Err(ConfigError::StripWidthTooLarge);
        }
        if !max_strip_length.is_finite() || max_strip_length <= 0.0 {
            return Err(ConfigError::MaxStripLengthNotPositive);
        }
        if max_strip_length > MAX_STRIP_LENGTH {
            return Err(ConfigError::MaxStripLengthTooLarge);
        }
        if min_overlap >= strip_width / 2.0 {
            return Err(ConfigError::MinOverlapTooLarge);
        }
        Ok(Self {
            expansion,
            strip_width,
            max_strip_length,
            min_overlap,
            force_line_targets,
            force_square_coverages,
            shard_radius,
            heading,
            ..Self::default()
        })
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            strip_width: DEFAULT_STRIP_WIDTH,
            min_strip_length: DEFAULT_MIN_STRIP_LENGTH,
            max_strip_length: DEFAULT_MAX_STRIP_LENGTH,
            min_overlap: DEFAULT_MIN_OVERLAP,
            expansion: DEFAULT_TARGET_EXPANSION,
            shard_density_ratio: DEFAULT_SHARD_DENSITY_RATIO,
            shard_radius: DEFAULT_SHARD_RADIUS,
            force_line_targets: false,
            force_square_coverages: false,
            heading: None,
            brute_force: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TessellationGeoResult {
    pub targets: Vec<Target>,
    pub coverages: Vec<Polygon>,
    pub intermediates: Vec<Polygon>,
}

impl TessellationGeoResult {
    pub fn empty() -> Self {
        Self {
            targets: vec![],
            coverages: vec![],
            intermediates: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TessellationGeoJSONResult {
    pub targets: Value,
    pub coverages: Value,
    pub intermediates: Value,
}

impl TessellationGeoJSONResult {
    /// Builds a single FeatureCollection from the selected output layers.
    pub fn to_feature_collection(&self, targets: bool, coverages: bool, intermediates: bool) -> Value {
        let mut features: Vec<Value> = vec![];
        let extract = |fc: &Value| -> Vec<Value> {
            fc["features"]
                .as_array()
                .expect("TessellationGeoJSONResult fields are always FeatureCollections")
                .clone()
        };
        if intermediates {
            features.extend(extract(&self.intermediates));
        }
        if coverages {
            features.extend(extract(&self.coverages));
        }
        if targets {
            features.extend(extract(&self.targets));
        }
        crate::geojson::feature_collection(features)
    }
}

pub type TessellationTuple = (Vec<Target>, Vec<Polygon>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::{
        DEFAULT_MAX_STRIP_LENGTH, DEFAULT_MIN_OVERLAP, DEFAULT_MIN_STRIP_LENGTH, DEFAULT_SHARD_DENSITY_RATIO,
        DEFAULT_SHARD_RADIUS, DEFAULT_STRIP_WIDTH, DEFAULT_TARGET_EXPANSION,
    };

    #[test]
    fn test_config_default_matches_constants() {
        let c = Config::default();
        assert_eq!(c.strip_width, DEFAULT_STRIP_WIDTH);
        assert_eq!(c.min_strip_length, DEFAULT_MIN_STRIP_LENGTH);
        assert_eq!(c.max_strip_length, DEFAULT_MAX_STRIP_LENGTH);
        assert_eq!(c.min_overlap, DEFAULT_MIN_OVERLAP);
        assert_eq!(c.expansion, DEFAULT_TARGET_EXPANSION);
        assert_eq!(c.shard_density_ratio, DEFAULT_SHARD_DENSITY_RATIO);
        assert_eq!(c.shard_radius, DEFAULT_SHARD_RADIUS);
        assert!(!c.force_line_targets);
        assert!(!c.force_square_coverages);
        assert!(c.heading.is_none());
        assert!(!c.brute_force);
    }

    #[test]
    fn test_config_new_happy_path_populates_non_param_fields_from_default() {
        let c = Config::new(1_000.0, 2_000.0, 20_000.0, 100.0, true, true, 25_000.0, Some(30.0)).unwrap();
        // Explicit fields:
        assert_eq!(c.expansion, 1_000.0);
        assert_eq!(c.strip_width, 2_000.0);
        assert_eq!(c.max_strip_length, 20_000.0);
        assert_eq!(c.min_overlap, 100.0);
        assert!(c.force_line_targets);
        assert!(c.force_square_coverages);
        assert_eq!(c.shard_radius, 25_000.0);
        assert_eq!(c.heading, Some(30.0));
        // Defaulted fields:
        assert_eq!(c.min_strip_length, DEFAULT_MIN_STRIP_LENGTH);
        assert_eq!(c.shard_density_ratio, DEFAULT_SHARD_DENSITY_RATIO);
    }

    #[test]
    fn test_config_new_rejects_non_positive_params() {
        // NaN
        assert_eq!(
            Config::new(f64::NAN, 5_000.0, 50_000.0, 200.0, false, false, 50_000.0, None).unwrap_err(),
            ConfigError::ExpansionNotPositive
        );
        assert_eq!(
            Config::new(5_000.0, f64::NAN, 50_000.0, 200.0, false, false, 50_000.0, None).unwrap_err(),
            ConfigError::StripWidthNotPositive
        );
        assert_eq!(
            Config::new(5_000.0, 5_000.0, f64::NAN, 200.0, false, false, 50_000.0, None).unwrap_err(),
            ConfigError::MaxStripLengthNotPositive
        );
        // Zero
        assert_eq!(
            Config::new(0.0, 5_000.0, 50_000.0, 200.0, false, false, 50_000.0, None).unwrap_err(),
            ConfigError::ExpansionNotPositive
        );
        // Negative
        assert_eq!(
            Config::new(5_000.0, -1.0, 50_000.0, 200.0, false, false, 50_000.0, None).unwrap_err(),
            ConfigError::StripWidthNotPositive
        );
        assert_eq!(
            Config::new(5_000.0, 5_000.0, -1.0, 200.0, false, false, 50_000.0, None).unwrap_err(),
            ConfigError::MaxStripLengthNotPositive
        );
    }

    #[test]
    fn test_config_new_expansion_too_large() {
        let err = Config::new(
            MAX_EXPANSION + 1.0,
            5_000.0,
            50_000.0,
            220.0,
            false,
            false,
            50_000.0,
            None,
        )
        .unwrap_err();
        assert_eq!(err, ConfigError::ExpansionTooLarge);
    }

    #[test]
    fn test_config_new_strip_width_too_large() {
        let err = Config::new(
            5_000.0,
            MAX_STRIP_WIDTH + 1.0,
            50_000.0,
            220.0,
            false,
            false,
            50_000.0,
            None,
        )
        .unwrap_err();
        assert_eq!(err, ConfigError::StripWidthTooLarge);
    }

    #[test]
    fn test_config_new_max_strip_length_too_large() {
        let err = Config::new(
            5_000.0,
            5_000.0,
            MAX_STRIP_LENGTH + 1.0,
            220.0,
            false,
            false,
            50_000.0,
            None,
        )
        .unwrap_err();
        assert_eq!(err, ConfigError::MaxStripLengthTooLarge);
    }

    #[test]
    fn test_config_new_min_overlap_too_large() {
        // min_overlap >= strip_width / 2
        let err = Config::new(5_000.0, 1_000.0, 50_000.0, 500.0, false, false, 50_000.0, None).unwrap_err();
        assert_eq!(err, ConfigError::MinOverlapTooLarge);
        let err = Config::new(5_000.0, 1_000.0, 50_000.0, 1_000.0, false, false, 50_000.0, None).unwrap_err();
        assert_eq!(err, ConfigError::MinOverlapTooLarge);
    }

    #[test]
    fn test_config_error_display() {
        assert_eq!(
            ConfigError::ExpansionNotPositive.to_string(),
            "Expansion must be a positive finite number."
        );
        assert_eq!(
            ConfigError::StripWidthNotPositive.to_string(),
            "Width must be a positive finite number."
        );
        assert_eq!(
            ConfigError::MaxStripLengthNotPositive.to_string(),
            "Maximum length must be a positive finite number."
        );
        assert_eq!(
            ConfigError::ExpansionTooLarge.to_string(),
            format!("Expansion exceeds maximum of {}Km.", MAX_EXPANSION / 1000.0)
        );
        assert_eq!(
            ConfigError::StripWidthTooLarge.to_string(),
            format!("Width exceeds maximum of {}Km.", MAX_STRIP_WIDTH / 1000.0)
        );
        assert_eq!(
            ConfigError::MaxStripLengthTooLarge.to_string(),
            format!("Maximum length exceeds maximum of {}Km.", MAX_STRIP_LENGTH / 1000.0)
        );
        assert_eq!(
            ConfigError::MinOverlapTooLarge.to_string(),
            "Minimum overlap must be less than half of --width."
        );
    }

    fn make_geojson_result() -> TessellationGeoJSONResult {
        let fc = |labels: &[&str]| {
            let features: Vec<serde_json::Value> = labels
                .iter()
                .map(|l| serde_json::json!({"type": "Feature", "properties": {"layer": l}, "geometry": null}))
                .collect();
            serde_json::json!({"type": "FeatureCollection", "features": features})
        };
        TessellationGeoJSONResult {
            targets: fc(&["t1", "t2"]),
            coverages: fc(&["c1"]),
            intermediates: fc(&["i1"]),
        }
    }

    #[test]
    fn test_to_feature_collection_all_layers() {
        let r = make_geojson_result();
        let fc = r.to_feature_collection(true, true, true);
        let features = fc["features"].as_array().unwrap();
        assert_eq!(features.len(), 4);
        assert_eq!(features[0]["properties"]["layer"], "i1");
        assert_eq!(features[1]["properties"]["layer"], "c1");
        assert_eq!(features[2]["properties"]["layer"], "t1");
        assert_eq!(features[3]["properties"]["layer"], "t2");
    }

    #[test]
    fn test_to_feature_collection_targets_only() {
        let r = make_geojson_result();
        let fc = r.to_feature_collection(true, false, false);
        let features = fc["features"].as_array().unwrap();
        assert_eq!(features.len(), 2);
        assert_eq!(features[0]["properties"]["layer"], "t1");
        assert_eq!(features[1]["properties"]["layer"], "t2");
    }

    #[test]
    fn test_to_feature_collection_none_selected() {
        let r = make_geojson_result();
        let fc = r.to_feature_collection(false, false, false);
        assert_eq!(fc["features"].as_array().unwrap().len(), 0);
    }
}
