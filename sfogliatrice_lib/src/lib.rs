pub mod defaults;
pub mod geo_utils;
pub mod intermediate;
pub mod projection;
pub mod tessellation;
pub mod types;

// Re-export the primary public API.
pub use tessellation::{tessellate, tessellate_strategy};
pub use types::{Config, ConfigError, Target, TessellationResult, TessellationTuple};
