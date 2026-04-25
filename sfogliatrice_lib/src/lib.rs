pub mod defaults;
pub mod geo_utils;
pub mod geojson;
pub mod intermediate;
pub mod projection;
pub mod tessellation;
pub mod types;

// Re-export the primary public API.
pub use tessellation::{tessellate, tessellate_geojson_to_geo, tessellate_geojson_to_geojson, tessellate_strategy};
pub use types::{Config, ConfigError, Target, TessellationGeoJSONResult, TessellationGeoResult, TessellationTuple};
