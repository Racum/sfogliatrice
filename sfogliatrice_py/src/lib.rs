use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pythonize::{depythonize, pythonize};
use sfogliatrice_lib::defaults::{
    DEFAULT_MAX_STRIP_LENGTH, DEFAULT_MIN_OVERLAP, DEFAULT_SHARD_RADIUS, DEFAULT_STRIP_WIDTH, DEFAULT_TARGET_EXPANSION,
};
use sfogliatrice_lib::{Config, tessellate_geojson_to_geojson};

/// Tessellate a GeoJSON geometry into targets, coverages, and intermediates.
///
/// Parameters
/// ----------
/// geojson : dict | str
///     GeoJSON object or JSON string (any geometry, feature, or feature collection).
/// strip_width : float, optional
///     Width of each survey strip, in meters. (default: 5 000 m)
/// min_strip_length : float, optional
///     Minimum strip length before two strips are merged, in meters. (default: 5 000 m)
/// max_strip_length : float, optional
///     Maximum strip length before a strip is split, in meters. (default: 50 000 m)
/// min_overlap : float, optional
///     Minimum overlap between adjacent strips, in meters. (default: 200 m)
/// expansion : float, optional
///     Buffer applied to Points and LineStrings before merging, in meters. (default: 5 000 m)
/// shard_density_ratio : float, optional
///     Fraction of shard_radius used as the grid cell size when sharding large intermediates. (default: 0.3)
/// shard_radius : float, optional
///     Maximum radius of a shard cluster before an intermediate is split, in meters. (default: 50 000 m)
/// force_line_targets : bool, optional
///     Always emit line targets even when the geometry is small enough for a point target. (default: False)
/// force_square_coverages : bool, optional
///     Always emit square coverage for Points instead of circles. (default: False)
/// heading : float, optional
///     Fixed strip heading in degrees; None lets the algorithm choose the optimal angle. (default: None)
/// brute_force : bool, optional
///     Try all headings 0–179° and pick the one with fewest targets; slow but optimal. (default: False)
/// ignore_holes : bool, optional
///     Ignore Polygon holes. (default: False)
///
/// Returns
/// -------
/// dict
///     A dict with keys ``targets``, ``coverages``, and ``intermediates``,
///     each a GeoJSON FeatureCollection dict.
#[pyfunction]
#[pyo3(signature = (
    geojson,
    strip_width=None,
    min_strip_length=None,
    max_strip_length=None,
    min_overlap=None,
    expansion=None,
    shard_density_ratio=None,
    shard_radius=None,
    force_line_targets=None,
    force_square_coverages=None,
    heading=None,
    brute_force=None,
    ignore_holes=None,
))]
#[allow(clippy::too_many_arguments)]
fn tessellate(
    py: Python<'_>,
    geojson: &Bound<'_, PyAny>,
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
    ignore_holes: Option<bool>,
) -> PyResult<Py<PyAny>> {
    let geojson_value: serde_json::Value = if let Ok(s) = geojson.extract::<String>() {
        serde_json::from_str(&s).map_err(|e| PyValueError::new_err(e.to_string()))?
    } else {
        depythonize(geojson).map_err(|e| PyValueError::new_err(e.to_string()))?
    };

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
    .map_err(|e| PyValueError::new_err(e.to_string()))?;

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
    if let Some(v) = ignore_holes {
        config.ignore_holes = v;
    }

    let result = tessellate_geojson_to_geojson(&geojson_value, &config);
    pythonize(py, &result)
        .map(|b| b.unbind())
        .map_err(|e| PyValueError::new_err(e.to_string()))
}

#[pymodule]
fn sfogliatrice(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(tessellate, m)?)?;
    Ok(())
}
