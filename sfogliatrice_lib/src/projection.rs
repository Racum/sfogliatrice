use geo::Point;
use geo_omerc::OMercTransformer;

/// Returns a bidirectional Oblique Mercator transformer anchored at the given centroid.
/// The transformer converts between geodesic (lon/lat in degrees) and cartesian (x/y in meters).
/// Returns `None` if the projection cannot be constructed (e.g. NaN or out-of-range coordinates).
pub fn get_projection(centroid: &Point) -> Option<OMercTransformer> {
    OMercTransformer::new(centroid).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::{Centroid, polygon};

    fn test_polygon() -> geo::Polygon {
        polygon![
            (x: 30.774421, y: -0.566473),
            (x: 30.781974, y: -0.612819),
            (x: 30.831413, y: -0.628611),
            (x: 30.857162, y: -0.648522),
            (x: 30.863685, y: -0.678733),
            (x: 30.914840, y: -0.714092),
            (x: 30.981788, y: -0.718555),
            (x: 31.008911, y: -0.700017),
            (x: 31.057319, y: -0.701047),
            (x: 31.078605, y: -0.675300),
            (x: 31.055603, y: -0.655732),
            (x: 31.045303, y: -0.568533),
            (x: 31.053886, y: -0.527336),
            (x: 30.978012, y: -0.522186),
            (x: 30.980758, y: -0.502618),
            (x: 30.925483, y: -0.530426),
            (x: 30.944366, y: -0.560980),
            (x: 30.922050, y: -0.597370),
            (x: 30.898017, y: -0.569219),
            (x: 30.862655, y: -0.573339),
            (x: 30.774421, y: -0.566473),
        ]
    }

    #[test]
    fn test_get_projection_roundtrip() {
        use geo::{Distance, Euclidean, Geometry};

        let anchor = test_polygon().centroid().unwrap();
        let transformer = get_projection(&anchor).unwrap();

        // A point slightly offset from anchor should round-trip cleanly.
        let original = Point::new(anchor.x() + 0.01, anchor.y() + 0.01);
        let cartesian = transformer.to_cartesian(&Geometry::Point(original)).unwrap();
        let back = transformer.to_geodesic(&cartesian).unwrap();
        let back_point: Point = back.try_into().unwrap();

        assert!(
            Euclidean.distance(original, back_point) < 1e-6,
            "Round-trip projection should preserve point within floating-point tolerance"
        );
    }

    #[test]
    fn test_get_projection_linestring_roundtrip() {
        use geo::{Distance, Euclidean, Geometry, LineString, line_string};

        let anchor = test_polygon().centroid().unwrap();
        let transformer = get_projection(&anchor).unwrap();
        let original = line_string![
            (x: anchor.x(), y: anchor.y()),
            (x: anchor.x() + 0.01, y: anchor.y()),
            (x: anchor.x() + 0.01, y: anchor.y() + 0.01),
        ];
        let cartesian = transformer
            .to_cartesian(&Geometry::LineString(original.clone()))
            .unwrap();
        let back = transformer.to_geodesic(&cartesian).unwrap();
        let back_ls: LineString = back.try_into().unwrap();
        for (a, b) in original.points().zip(back_ls.points()) {
            assert!(
                Euclidean.distance(a, b) < 1e-6,
                "LineString vertex must round-trip within tolerance"
            );
        }
    }

    #[test]
    fn test_get_projection_polygon_roundtrip() {
        use geo::{Distance, Euclidean, Geometry, Polygon};

        let anchor = test_polygon().centroid().unwrap();
        let transformer = get_projection(&anchor).unwrap();
        let original = test_polygon();
        let cartesian = transformer.to_cartesian(&Geometry::Polygon(original.clone())).unwrap();
        let back = transformer.to_geodesic(&cartesian).unwrap();
        let back_poly: Polygon = back.try_into().unwrap();
        for (a, b) in original.exterior().points().zip(back_poly.exterior().points()) {
            assert!(
                Euclidean.distance(a, b) < 1e-6,
                "Polygon vertex must round-trip within tolerance"
            );
        }
    }

    #[test]
    fn test_get_projection_far_from_equator_anchor() {
        use geo::{Distance, Euclidean, Geometry};

        // Anchor near the Arctic Circle: ensure the projection still constructs and round-trips.
        let anchor = Point::new(20.0_f64, 80.0_f64);
        let transformer = get_projection(&anchor).unwrap();
        let original = Point::new(anchor.x() + 0.01, anchor.y() + 0.01);
        let cartesian = transformer.to_cartesian(&Geometry::Point(original)).unwrap();
        let back: Point = transformer.to_geodesic(&cartesian).unwrap().try_into().unwrap();
        assert!(Euclidean.distance(original, back) < 1e-6);
    }

    #[test]
    fn test_get_projection_nan_anchor_returns_none() {
        assert!(get_projection(&Point::new(f64::NAN, 0.0)).is_none());
        assert!(get_projection(&Point::new(0.0, f64::NAN)).is_none());
    }

    #[test]
    fn test_get_projection_units_are_meters() {
        use geo::{Distance, Euclidean, Geometry};

        // Two points ~111 km apart (1 degree of latitude ≈ 111 000 m)
        let anchor = Point::new(0.0_f64, 0.0_f64);
        let one_degree_north = Point::new(0.0_f64, 1.0_f64);
        let transformer = get_projection(&anchor).unwrap();

        let c_anchor = transformer.to_cartesian(&Geometry::Point(anchor)).unwrap();
        let c_north = transformer.to_cartesian(&Geometry::Point(one_degree_north)).unwrap();
        let distance = Euclidean.distance(&c_anchor, &c_north);

        // 1 degree of latitude ≈ 110 574 m; allow 1 % tolerance
        assert!(
            (distance - 110_574.).abs() < 1_500.,
            "Distance in cartesian space should be in meters, got {distance}"
        );
    }
}
