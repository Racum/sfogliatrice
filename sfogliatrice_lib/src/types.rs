use crate::defaults::{
    DEFAULT_INFLATION, DEFAULT_MAX_STRIP_LENGTH, DEFAULT_MIN_OVERLAP, DEFAULT_MIN_STRIP_LENGTH,
    DEFAULT_SHARD_DENSITY_RATIO, DEFAULT_SHARD_RADIUS, DEFAULT_STRIP_WIDTH, DEFAULT_TARGET_EXPANSION,
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
    pub strip_width: f64,
    pub min_strip_length: f64,
    pub max_strip_length: f64,
    pub min_overlap: f64,
    pub expansion: f64,
    pub shard_density_ratio: f64,
    pub shard_radius: f64,
    pub inflation: f64,
    pub force_line_targets: bool,
    pub force_square_coverages: bool,
    pub heading: Option<f64>,
}

/// Describes which parameter failed validation and why.
#[derive(Debug, PartialEq)]
pub enum ConfigError {
    ExpansionTooLarge,
    StripWidthTooLarge,
    MaxStripLengthTooLarge,
    MinOverlapTooLarge,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
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
        if expansion > MAX_EXPANSION {
            return Err(ConfigError::ExpansionTooLarge);
        }
        if strip_width > MAX_STRIP_WIDTH {
            return Err(ConfigError::StripWidthTooLarge);
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
            inflation: DEFAULT_INFLATION,
            force_line_targets: false,
            force_square_coverages: false,
            heading: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TessellationResult {
    pub targets: Vec<Target>,
    pub coverages: Vec<Polygon>,
    pub intermediates: Vec<Polygon>,
}

pub type TessellationTuple = (Vec<Target>, Vec<Polygon>);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defaults::{
        DEFAULT_INFLATION, DEFAULT_MAX_STRIP_LENGTH, DEFAULT_MIN_OVERLAP, DEFAULT_MIN_STRIP_LENGTH,
        DEFAULT_SHARD_DENSITY_RATIO, DEFAULT_SHARD_RADIUS, DEFAULT_STRIP_WIDTH, DEFAULT_TARGET_EXPANSION,
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
        assert_eq!(c.inflation, DEFAULT_INFLATION);
        assert!(!c.force_line_targets);
        assert!(!c.force_square_coverages);
        assert!(c.heading.is_none());
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
        assert_eq!(c.inflation, DEFAULT_INFLATION);
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
}
