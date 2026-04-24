use geo::algorithm::concave_hull::ConcaveHullOptions;
use geo::{Area, BoundingRect, ConcaveHull, Coord, Polygon, Rect, Validation};
use static_aabb2d_index::StaticAABB2DIndexBuilder;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

/// Clusters nearby polygons and wraps each cluster with a concave hull.
pub fn combine_polygons(polygons: &[Polygon], threshold: f64) -> Vec<Polygon> {
    if polygons.is_empty() {
        return vec![];
    }

    let clusters = find_clusters(polygons, threshold);

    // Hull computation is independent per cluster, so it parallelises cleanly.
    #[cfg(feature = "parallel")]
    {
        clusters
            .into_par_iter()
            .filter_map(|indices| hull_of(polygons, &indices, threshold))
            .collect()
    }
    #[cfg(not(feature = "parallel"))]
    {
        clusters
            .into_iter()
            .filter_map(|indices| hull_of(polygons, &indices, threshold))
            .collect()
    }
}

/// Groups polygon indices into clusters using a Flatbush spatial index and union-find.
///
/// Two polygons belong to the same cluster when their AABBs, each expanded by
/// `threshold` on all sides, overlap.
///
/// Runs in two phases to keep union-find (mutable shared state) sequential:
/// 1. Query phase: collect all (i, j) neighbour pairs where j > i.
/// 2. Merge phase: walk the edge list and union roots.
fn find_clusters(polygons: &[Polygon], threshold: f64) -> Vec<Vec<usize>> {
    let n = polygons.len();

    // Only index polygons that have a valid bounding rect; degenerate ones are
    // kept in their own single-element clusters by the union-find default.
    let valid: Vec<(usize, Rect)> = polygons
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.bounding_rect().map(|r| (i, r)))
        .collect();
    let m = valid.len();

    // StaticAABB2DIndex (Flatbush). Items are inserted in `valid` order;
    // query() returns positions in `valid`, not original polygon indices.
    let mut builder = StaticAABB2DIndexBuilder::new(m);
    for (_, r) in &valid {
        builder.add(r.min().x, r.min().y, r.max().x, r.max().y);
    }
    let index = builder.build().expect("static_aabb2d_index build failed");

    // Phase 1: collect neighbour edges (pos_j > pos_i avoids duplicate pairs).
    #[cfg(feature = "parallel")]
    let edges: Vec<(usize, usize)> = (0..m)
        .into_par_iter()
        .flat_map_iter(|pos_i| {
            let index = &index; // &T is Copy, so the inner move closure can capture it
            let valid = &valid;
            let (poly_i, r) = valid[pos_i];
            index
                .query(
                    r.min().x - threshold,
                    r.min().y - threshold,
                    r.max().x + threshold,
                    r.max().y + threshold,
                )
                .into_iter()
                .filter_map(move |pos_j| {
                    if pos_j > pos_i {
                        Some((poly_i, valid[pos_j].0))
                    } else {
                        None
                    }
                })
        })
        .collect();

    #[cfg(not(feature = "parallel"))]
    let edges: Vec<(usize, usize)> = {
        let mut edges = vec![];
        for (pos_i, &(poly_i, r)) in valid.iter().enumerate() {
            for pos_j in index.query(
                r.min().x - threshold,
                r.min().y - threshold,
                r.max().x + threshold,
                r.max().y + threshold,
            ) {
                if pos_j > pos_i {
                    edges.push((poly_i, valid[pos_j].0));
                }
            }
        }
        edges
    };

    // Phase 2: union-find over the edge list.
    let mut parent: Vec<usize> = (0..n).collect();

    // Path-halving: compresses the tree without a second pass.
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }

    for (i, j) in edges {
        let ri = find(&mut parent, i);
        let rj = find(&mut parent, j);
        if ri != rj {
            parent[ri] = rj;
        }
    }

    let mut clusters: std::collections::HashMap<usize, Vec<usize>> = Default::default();
    for i in 0..n {
        clusters.entry(find(&mut parent, i)).or_default().push(i);
    }
    clusters.into_values().collect()
}

/// Computes the concave hull over the exterior vertices of all polygons in the cluster.
/// Returns the polygon directly for single-member clusters.
/// Returns `None` if the hull is degenerate.
fn hull_of(polygons: &[Polygon], indices: &[usize], threshold: f64) -> Option<Polygon> {
    if indices.len() == 1 {
        return Some(polygons[indices[0]].clone());
    }

    let coords: Vec<Coord> = indices
        .iter()
        .flat_map(|&i| polygons[i].exterior().0.iter().copied())
        .collect();

    if coords.is_empty() {
        return None;
    }

    // concavity = 2.0: empirically good balance between tightness and stability.
    // length_threshold = threshold: prevents the hull from diving into gaps smaller
    // than the clustering distance.
    let hull = coords.concave_hull_with_options(ConcaveHullOptions {
        concavity: 2.0,
        length_threshold: threshold,
    });

    if hull.is_valid() && hull.unsigned_area() > 0.0 {
        Some(hull)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use geo::{
        Area, Centroid, Geometry, GeometryCollection, LineString, algorithm::bool_ops::BooleanOps,
        algorithm::unary_union, polygon,
    };
    use geojson::GeoJson;
    use serde_json::Value;

    use super::*;
    use crate::geo_utils::{coerce_to_polygon, iterate_normalized_geometry};
    use crate::projection::get_projection;

    fn iterate_geometry_from_geojson(geojson: &Value) -> Vec<Geometry> {
        let gj: GeoJson = serde_json::from_value(geojson.clone()).expect("Invalid GeoJSON");
        match gj {
            GeoJson::Geometry(geom) => Geometry::try_from(geom).into_iter().collect(),
            GeoJson::Feature(f) => f
                .geometry
                .and_then(|g| Geometry::try_from(g).ok())
                .into_iter()
                .collect(),
            GeoJson::FeatureCollection(fc) => fc
                .features
                .into_iter()
                .filter_map(|f| f.geometry.and_then(|g| Geometry::try_from(g).ok()))
                .collect(),
        }
    }

    #[test]
    fn test_combine_polygons() {
        let fixtures = ["comunidad_valenciana.geojson", "iberia.geojson"];
        let mut failures: Vec<String> = Vec::new();

        for name in fixtures {
            let path = std::path::Path::new("../fixtures").join(name);
            let geojson_str = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("Failed to read {name}: {e}"));
            let geojson: Value =
                serde_json::from_str(&geojson_str).unwrap_or_else(|e| panic!("Invalid JSON in {name}: {e}"));

            let geodesic_geometries = iterate_geometry_from_geojson(&geojson);
            let gc = GeometryCollection::new_from(geodesic_geometries.clone());
            let centroid = gc
                .centroid()
                .unwrap_or_else(|| panic!("Failed to compute centroid for {name}"));
            let transformer =
                get_projection(&centroid).unwrap_or_else(|| panic!("Failed to build projection for {name}"));

            let cartesian_geometries =
                iterate_normalized_geometry(&geodesic_geometries, |g| transformer.to_cartesian(g).ok());

            let expanded: Vec<Polygon> = cartesian_geometries
                .iter()
                .filter_map(|g| coerce_to_polygon(g, 5_000.0))
                .collect();

            let result = combine_polygons(&expanded, 5_000.0);

            let input_area: f64 = cartesian_geometries.iter().map(|p| p.unsigned_area()).sum();
            let output_area: f64 = result.iter().map(|p| p.unsigned_area()).sum();

            if result.is_empty() {
                failures.push(format!("{name}: returned empty result"));
            } else if output_area < input_area {
                failures.push(format!(
                    "{name}: output area ({output_area:.0}) is smaller than input area ({input_area:.0})"
                ));
            } else if output_area / input_area >= 1.5 {
                failures.push(format!(
                    "{name}: output area ({output_area:.0}) is {:.2}× input area ({input_area:.0}), expected < 1.5×",
                    output_area / input_area
                ));
            } else {
                for (i, input) in expanded.iter().enumerate() {
                    let input_area = input.unsigned_area();
                    let covered: f64 = result.iter().map(|o| o.intersection(input).unsigned_area()).sum();
                    let coverage = covered / input_area;
                    if coverage < 0.9 {
                        failures.push(format!(
                            "{name}: input polygon #{i} is only {:.1}% covered by output polygons, expected >= 90%",
                            coverage * 100.0
                        ));
                    }
                }

                let input_union = unary_union(expanded.iter());
                let input_union_area = input_union.unsigned_area();
                let output_union = unary_union(result.iter());
                let global_covered = output_union.intersection(&input_union).unsigned_area();
                let global_coverage = global_covered / input_union_area;
                if global_coverage < 0.99 {
                    failures.push(format!(
                        "{name}: union of all inputs is only {:.1}% covered by output polygons, expected >= 99%",
                        global_coverage * 100.0
                    ));
                }
            }
        }

        assert!(failures.is_empty(), "Failures:\n  - {}", failures.join("\n  - "));
    }

    fn square(x: f64, y: f64, size: f64) -> Polygon {
        polygon![
            (x: x, y: y),
            (x: x + size, y: y),
            (x: x + size, y: y + size),
            (x: x, y: y + size),
            (x: x, y: y),
        ]
    }

    #[test]
    fn test_combine_polygons_empty() {
        let out = combine_polygons(&[], 5_000.0);
        assert!(out.is_empty());
    }

    #[test]
    fn test_combine_polygons_single() {
        let p = square(0.0, 0.0, 10_000.0);
        let out = combine_polygons(std::slice::from_ref(&p), 1_000.0);
        assert_eq!(out.len(), 1, "single polygon passes through as one");
        assert!(out[0].unsigned_area() >= p.unsigned_area() * 0.99);
    }

    #[test]
    fn test_combine_polygons_far_apart_remain_separate() {
        let threshold = 1_000.0;
        let a = square(0.0, 0.0, 5_000.0);
        let b = square(100_000.0, 100_000.0, 5_000.0);
        let out = combine_polygons(&[a.clone(), b.clone()], threshold);
        assert_eq!(out.len(), 2, "non-overlapping polygons must remain separate");
    }

    #[test]
    fn test_combine_polygons_overlapping_collapse_to_one() {
        let a = square(0.0, 0.0, 10_000.0);
        let out = combine_polygons(&[a.clone(), a.clone()], 1_000.0);
        assert_eq!(out.len(), 1, "fully-overlapping polygons must collapse to one");
    }

    #[test]
    fn test_combine_polygons_degenerate_not_merged() {
        // A polygon with an empty exterior has no bounding rect, so it gets NaN
        // sentinel coords in the spatial index and must never cluster with a valid polygon.
        let normal = square(0.0, 0.0, 10_000.0);
        let degenerate = Polygon::new(LineString::new(vec![]), vec![]);
        assert!(degenerate.bounding_rect().is_none(), "precondition: no bounding rect");
        let out = combine_polygons(&[normal, degenerate], 1_000.0);
        assert_eq!(out.len(), 2, "degenerate polygon must not merge with the normal one");
    }
}
