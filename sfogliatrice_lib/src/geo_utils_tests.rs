use super::*;

use std::f64::consts::FRAC_PI_2;

use geo::algorithm::line_measures::Euclidean;
use geo::{
    Area, Centroid, ConvexHull, CoordsIter, Geometry, GeometryCollection, Length, Line, LineString, MinimumRotatedRect,
    MultiLineString, MultiPoint, MultiPolygon, Polygon, Rect, Triangle, Winding, coord, line_string, point, polygon,
};

use crate::defaults::*;

#[test]
fn test_iterate_geometry_with_polygon() {
    let geo = Geometry::Polygon(polygon![
        (x: -111., y: 45.),
        (x: -111., y: 41.),
        (x: -104., y: 41.),
        (x: -104., y: 45.),
    ]);
    assert_eq!(iterate_geometry(&geo).next().unwrap(), geo);
}

#[test]
fn test_iterate_geometry_with_multipolygon() {
    let poly1 = polygon![(x: 0., y: 0.), (x: 2., y: 0.), (x: 1., y: 2.), (x:0., y:0.)];
    let poly2 = polygon![(x: 10., y: 10.), (x: 12., y: 10.), (x: 11., y: 12.), (x:10., y:10.)];
    let geo = Geometry::MultiPolygon(MultiPolygon::new(vec![poly1.clone(), poly2.clone()]));
    let mut result = iterate_geometry(&geo);
    assert_eq!(result.next().unwrap(), Geometry::Polygon(poly1.clone()));
    assert_eq!(result.next().unwrap(), Geometry::Polygon(poly2.clone()));
}

#[test]
fn test_iterate_polygons() {
    let a_point = point! { x: 0., y: 0. };
    let a_line = line_string![(x: 0., y: 0.), (x: 1., y: 1.)];
    let a_poly = polygon![(x: 0., y: 0.), (x: 2., y: 0.), (x: 1., y: 2.), (x:0., y:0.)];
    let a_gc = GeometryCollection::new_from(vec![
        Geometry::Point(a_point),
        Geometry::MultiPoint(MultiPoint::new(vec![a_point, a_point])),
        Geometry::LineString(a_line.clone()),
        Geometry::MultiLineString(MultiLineString::new(vec![a_line.clone(), a_line.clone()])),
        Geometry::Polygon(a_poly.clone()),
        Geometry::MultiPolygon(MultiPolygon::new(vec![a_poly.clone(), a_poly.clone()])),
    ]);
    let binding = Geometry::GeometryCollection(a_gc);
    let result: Vec<Polygon> = iterate_polygons(&binding).collect();

    // 3 points (1 Point + 2 from MultiPoint) + 3 lines (1 LineString + 2 from MultiLineString)
    // + 3 polygons (1 Polygon + 2 from MultiPolygon) = 9 total.
    assert_eq!(result.len(), 9);

    // Points and lines are buffered into real polygons (non-zero area).
    for p in &result[..6] {
        assert!(p.unsigned_area() > 0.0, "buffered point/line must have area");
    }

    // Polygons pass through unchanged.
    assert_eq!(result[6], a_poly);
    assert_eq!(result[7], a_poly);
    assert_eq!(result[8], a_poly);
}

#[test]
fn test_get_boundary_lines() {
    let a_poly = polygon![(x: 0., y: 0.), (x: 2., y: 0.), (x: 1., y: 2.), (x: 0., y: 0.)];
    let result = get_rectangular_boundary_lines(&Geometry::Polygon(a_poly), None, None).unwrap();
    let lengths = result.map(|l| format!("{:.10}", Euclidean.length(&l)));
    assert_eq!(lengths[0], lengths[2]);
    assert_eq!(lengths[1], lengths[3]);
}

#[test]
fn test_get_short_width_lines() {
    let a_poly = polygon![(x: 0., y: 0.), (x: 2., y: 0.), (x: 1., y: 2.), (x: 0., y: 0.)];
    let input_lines = get_rectangular_boundary_lines(&Geometry::Polygon(a_poly), None, None).unwrap();
    let input_lines_lengths = input_lines.map(|l| format!("{:.10}", Euclidean.length(&l)));
    let result = get_rectangular_shorter_sides(input_lines, None);
    let result_lengths = result.map(|l| format!("{:.10}", Euclidean.length(&l)));
    let shortest_length_input_line = input_lines_lengths.into_iter().min().unwrap();
    assert_eq!(result_lengths[0], shortest_length_input_line);
    assert_eq!(result_lengths[1], shortest_length_input_line);
}

#[test]
fn test_ensure_lines_pointing_down() {
    let line_up = Line::new(coord! { x: 0., y: 0. }, coord! { x: 1., y: 1. });
    let line_down = Line::new(coord! { x: 1., y: 1. }, coord! { x: 0., y: 0. });
    let result = ensure_lines_pointing_down([line_up, line_down]);
    assert_eq!(result, [line_down, line_down]);
}

#[test]
fn test_sort_by_highest_line() {
    let line_low = Line::new(coord! { x: 1., y: 1. }, coord! { x: 2., y: 1. });
    let line_high = Line::new(coord! { x: 1., y: 2. }, coord! { x: 2., y: 2. });
    let result = sort_by_highest_line([line_low, line_high]);
    assert_eq!(result, [line_high, line_low]);
}

#[test]
fn test_roll_lines() {
    let a_poly = polygon![(x: 0., y: 0.), (x: 2., y: 0.), (x: 1., y: 2.), (x: 0., y: 0.)];
    let result = roll_lines(&Geometry::Polygon(a_poly.clone()), None, None).unwrap();
    assert_eq!(
        format!("{:.10}", Euclidean.length(&result[0])),
        format!("{:.10}", Euclidean.length(&result[1]))
    );
    let mbr = MinimumRotatedRect::minimum_rotated_rect(&a_poly).unwrap();
    let ch = ConvexHull::convex_hull(&GeometryCollection::new_from(vec![
        Geometry::Line(result[0]),
        Geometry::Line(result[1]),
    ]));
    assert!(mbr.signed_area() - ch.signed_area() < 0.0000000001);
}

#[test]
fn test_distribute_points() {
    fn _assert(axis_size: f64, points: u16) {
        let line = Line::new(coord! { x: 0., y: 0. }, coord! { x: axis_size, y: axis_size });
        let result = distribute_points(&line, points);
        for axis_values in [
            result.clone().into_iter().map(|p| p.x()).collect::<Vec<_>>(),
            result.clone().into_iter().map(|p| p.y()).collect::<Vec<_>>(),
        ] {
            let num_axis = axis_values.len();
            assert_eq!(num_axis, points as usize);
            let starts: Vec<f64> = axis_values[0..num_axis - 1].into();
            let ends: Vec<f64> = axis_values[1..num_axis].into();
            let axis_distances = &starts.into_iter().zip(&ends).map(|p| p.1 - p.0).collect::<Vec<_>>();
            assert!(
                axis_distances
                    .iter()
                    .all(|&item| item - axis_distances[0] < 0.0000000001)
            );
        }
    }
    _assert(12., 3);
    _assert(17., 4);
    _assert(5., 2);
    _assert(30., 7);
    _assert(-20., 7);
}

#[test]
fn test_get_angle() {
    fn _assert(point_to: [f64; 2], rads: f64) {
        let line = Line::new(coord! { x: 0., y: 0. }, coord! { x: point_to[0], y: point_to[1] });
        let result = get_angle(&line);
        assert!((result.abs() - rads.abs()) < 0.0000001);
    }
    _assert([0., 1.], 0.);
    _assert([1., 0.], FRAC_PI_2);
    _assert([2., 7.], -0.27829965);
    _assert([-11., 2.3], 1.36467498);
    _assert([100., 7.77], -1.49325212);
}

#[test]
fn test_resize_line() {
    let input_line = Line::new(coord! { x: 10., y: 10. }, coord! { x: 13., y: 14.});
    assert_eq!(Euclidean.length(&input_line), 5.);
    let result = resize_line(&input_line, 10.);
    assert_eq!(Euclidean.length(&result), 10.);
    assert_eq!(input_line.centroid(), result.centroid());
    assert_eq!(get_angle(&input_line), get_angle(&result));
}

#[test]
fn test_intersect_line() {
    let input_polygon = polygon![
        (x: -19256.33875833291, y: 5999.127809052266),  // furthest point.
        (x: -18415.427401810004, y: 874.4500305110432),
        (x: -12912.160576284308, y: -871.5999373131576),
        (x: -10045.922978637238, y: -3073.193191080783),
        (x: -9319.774702556258, y: -6413.748238430995),
        (x: -3625.59280666473, y: -10323.481937938095),
        (x: 3826.4462615317916, y: -10816.977127269925),
        (x: 6845.557255517235, y: -8767.17848445255),
        (x: 12233.915232871395, y: -8881.168746990606),
        (x: 14603.369269381328, y: -6034.262185142122),
        (x: 12043.016778886991, y: -3870.4749466353587),
        (x: 10896.674571023332, y: 5771.540142030943),
        (x: 11852.165443542572, y: 10326.863795047044),
        (x: 3406.2618108754973, y: 10896.414713736964),
        (x: 3711.9437942144473, y: 13060.132700511447),
        (x: -2440.9954980681773, y: 9985.285772387377),
        (x: -339.03719805890324, y: 6606.800543615501),
        (x: -2823.1066995348947, y: 2582.992047141981),
        (x: -5498.331553644731, y: 5695.754647872724),
        (x: -9434.614476070996, y: 5240.141825869699),
    ];

    // Intersects:
    let input_line = line_string![
        (x: 11367.360586, y: 12432.434695),
        (x: -19443.231458, y: 5338.337472),
    ]
    .lines()
    .next()
    .unwrap();
    let output_line = intersect_line(
        &input_polygon,
        &input_line,
        DEFAULT_STRIP_WIDTH,
        DEFAULT_MIN_STRIP_LENGTH,
    )
    .unwrap();
    assert!(
        Euclidean.length(&output_line) < Euclidean.length(&input_line),
        "Output line is smaller"
    );
    assert_eq!(
        get_angle(&output_line),
        get_angle(&input_line),
        "Both lines have the same direction"
    );

    // Does not intersect:
    let input_line = line_string![(x: 20_000., y: 20_000.), (x: 30_000., y: 30_000.)]
        .lines()
        .next()
        .unwrap();
    assert!(
        intersect_line(
            &input_polygon,
            &input_line,
            DEFAULT_STRIP_WIDTH,
            DEFAULT_MIN_STRIP_LENGTH
        )
        .is_none()
    );
}

#[test]
fn test_substring() {
    // From Shapely examples:
    let ls = Line::new(coord! { x: 0., y: 0. }, coord! { x: 5., y: 0. });
    let line_1_to_3 = substring(&ls, 1., 3.);
    assert_eq!(line_1_to_3.start.x, 1.);
    assert_eq!(line_1_to_3.end.x, 3.);
    let line_3_to_1 = substring(&ls, 3., 1.);
    assert_eq!(line_3_to_1.start.x, 3.);
    assert_eq!(line_3_to_1.end.x, 1.);
    let line_1_to_m3: Line = substring(&ls, 1., -3.);
    assert_eq!(line_1_to_m3.start.x, 1.);
    assert_eq!(line_1_to_m3.end.x, 2.);
    let line_to_point = substring(&ls, 2.5, 2.5);
    assert_eq!(line_to_point.start.x, 2.5);
    assert_eq!(line_to_point.end.x, 2.5);

    // Extra test:
    let extra = substring(
        &Line::new(
            coord! { x: -29365.75182926617, y: 12826.96390730498 },
            coord! { x: 25185.240790224532, y: -5187.041853216486 },
        ),
        50000.0,
        57448.369857741214,
    );
    assert_eq!(extra.start, coord! { x: 18112.52540869092, y: -2851.466834184601 });
    assert_eq!(extra.end, coord! { x: 25185.240790224532, y: -5187.041853216486 });
}

#[test]
fn test_segment_lines() {
    let input_lines = [
        Line::new(coord! { x: -100_000., y: -1_000. }, coord! { x: 100_000., y: -1_000. }),
        Line::new(coord! { x: -100_000., y: 6_000. }, coord! { x: 100_000., y: 6_000. }),
    ];
    let output_lines = segment_lines(&input_lines, DEFAULT_MAX_STRIP_LENGTH, DEFAULT_STRIP_WIDTH);
    let expected_result = vec![
        // First line:
        Line::new(coord! { x: -100_000., y: -1_000. }, coord! { x: -50_000., y: -1_000. }),
        Line::new(coord! { x: -50_000., y: -1_000. }, coord! { x: 0., y: -1_000. }),
        Line::new(coord! { x: 0., y: -1_000. }, coord! { x: 50_000., y: -1_000. }),
        Line::new(coord! { x: 50_000., y: -1_000. }, coord! { x: 100_000., y: -1_000. }),
        // Second line:
        Line::new(coord! { x: -100_000., y: 6_000. }, coord! { x: -50_000., y: 6_000. }),
        Line::new(coord! { x: -50_000., y: 6_000. }, coord! { x: 0., y: 6_000. }),
        Line::new(coord! { x: 0., y: 6_000. }, coord! { x: 50_000., y: 6_000. }),
        Line::new(coord! { x: 50_000., y: 6_000. }, coord! { x: 100_000., y: 6_000. }),
    ];
    assert_eq!(&output_lines, &expected_result, "Segments look like expected");
    for line in output_lines {
        assert!(
            Euclidean.length(&line) <= DEFAULT_MAX_STRIP_LENGTH,
            "All segments are within range"
        );
    }
}

#[test]
fn test_furthest_from_centroid() {
    let input_polygon = polygon![
        (x: -19256.33875833291, y: 5999.127809052266),  // furthest point.
        (x: -18415.427401810004, y: 874.4500305110432),
        (x: -12912.160576284308, y: -871.5999373131576),
        (x: -10045.922978637238, y: -3073.193191080783),
        (x: -9319.774702556258, y: -6413.748238430995),
        (x: -3625.59280666473, y: -10323.481937938095),
        (x: 3826.4462615317916, y: -10816.977127269925),
        (x: 6845.557255517235, y: -8767.17848445255),
        (x: 12233.915232871395, y: -8881.168746990606),
        (x: 14603.369269381328, y: -6034.262185142122),
        (x: 12043.016778886991, y: -3870.4749466353587),
        (x: 10896.674571023332, y: 5771.540142030943),
        (x: 11852.165443542572, y: 10326.863795047044),
        (x: 3406.2618108754973, y: 10896.414713736964),
        (x: 3711.9437942144473, y: 13060.132700511447),
        (x: -2440.9954980681773, y: 9985.285772387377),
        (x: -339.03719805890324, y: 6606.800543615501),
        (x: -2823.1066995348947, y: 2582.992047141981),
        (x: -5498.331553644731, y: 5695.754647872724),
        (x: -9434.614476070996, y: 5240.141825869699),
    ];
    let point_distance = furthest_from_centroid(&input_polygon);
    assert_eq!(
        [point_distance.point.x(), point_distance.point.y()],
        [-19256.33875833291, 5999.127809052266]
    );
    assert_eq!(point_distance.distance, 20169.175816200113);
}

#[test]
fn test_iterate_shards_shardable() {
    // Shardable:
    let input_polygon = polygon![
        (x: -74400.66871025234, y: 7666.642391805078),
        (x: -77202.5508468671, y: 5229.977455704472),
        (x: -29305.38633215403, y: -40399.784615454955),
        (x: -10346.276245633688, y: -28545.69984595842),
        (x: -14646.137531817527, y: -3100.5991480125026),
        (x: 54707.76383555171, y: 33778.40807570438),
        (x: 78993.9953785586, y: 4784.469144617179),
        (x: 82686.11516234672, y: 6510.707988940343),
        (x: 55757.01749507594, y: 38230.30056863385),
        (x: -19126.788912682838, y: -1398.621406933467),
        (x: -14530.643094814059, y: -26841.613049263757),
        (x: -28668.73928992677, y: -35610.42413499955),
        (x: -74400.66871025234, y: 7666.642391805078),
    ];
    let output_polygons: Vec<Polygon> =
        iterate_shards(&input_polygon, DEFAULT_SHARD_RADIUS, DEFAULT_SHARD_DENSITY_RATIO).collect();
    assert_eq!(output_polygons.len(), 3);
    let input_area = input_polygon.unsigned_area();
    let output_area = output_polygons
        .into_iter()
        .map(|a| a.unsigned_area())
        .reduce(|a, b| a + b)
        .unwrap();
    assert!((input_area - output_area).abs() < 20.);
}

#[test]
fn test_iterate_shards_area_fully_covered() {
    // A cross/plus shape: two overlapping rectangles forming a non-convex polygon.
    // The difference after each shard step can split into multiple pieces; this test
    // confirms all pieces are retained (R3 fix) and total area is conserved.
    let input_polygon = polygon![
        (x: -10_000., y: -30_000.),
        (x:  10_000., y: -30_000.),
        (x:  10_000., y: -10_000.),
        (x:  30_000., y: -10_000.),
        (x:  30_000., y:  10_000.),
        (x:  10_000., y:  10_000.),
        (x:  10_000., y:  30_000.),
        (x: -10_000., y:  30_000.),
        (x: -10_000., y:  10_000.),
        (x: -30_000., y:  10_000.),
        (x: -30_000., y: -10_000.),
        (x: -10_000., y: -10_000.),
        (x: -10_000., y: -30_000.),
    ];
    let shards: Vec<Polygon> = iterate_shards(&input_polygon, 15_000., DEFAULT_SHARD_DENSITY_RATIO).collect();
    let input_area = input_polygon.unsigned_area();
    let output_area: f64 = shards.iter().map(|p| p.unsigned_area()).sum();
    assert!(
        (input_area - output_area).abs() / input_area < 0.01,
        "shards must cover > 99% of input area"
    );
}

#[test]
fn test_iterate_shards_not_shardable() {
    // Shardable:
    let input_polygon = polygon![
        (x:1000., y: 1000.),
        (x: 0., y: 1000.),
        (x: 0., y: 0.),
        (x: 1000., y: 0.),
        (x:1000., y: 1000.),
    ];
    let output_polygons: Vec<Polygon> =
        iterate_shards(&input_polygon, DEFAULT_SHARD_RADIUS, DEFAULT_SHARD_DENSITY_RATIO).collect();
    assert_eq!(output_polygons.len(), 1);
    let input_area = input_polygon.unsigned_area();
    let output_area = output_polygons
        .into_iter()
        .map(|a| a.unsigned_area())
        .reduce(|a, b| a + b)
        .unwrap();
    assert_eq!(input_area, output_area);
}

#[test]
fn test_is_too_small() {
    fn _assert(x: f64, y: f64, expected: bool) {
        assert_eq!(
            is_too_small(
                &polygon![(x: 0., y: 0.), (x: x, y: 0.), (x: x, y: y), (x: 0., y: y), (x: 0., y: 0.)],
                DEFAULT_MIN_STRIP_LENGTH
            ),
            expected
        );
    }
    _assert(1., 1., true);
    _assert(3500., 3500., true);
    _assert(1000., 5000., false);
    _assert(5000., 1., false);
    _assert(5000., 5000., false);
}

#[test]
fn test_ensure_line_length() {
    fn _assert(length_in: f64, length_out: f64) {
        let input_line = Line::new(coord! { x: 0., y: 0. }, coord! { x: 0., y: length_in });
        let output_line = ensure_line_length(&input_line, DEFAULT_MIN_STRIP_LENGTH);
        assert_eq!(Euclidean.length(&output_line), length_out);
        assert_eq!(get_angle(&input_line), get_angle(&output_line));
    }
    _assert(1., DEFAULT_MIN_STRIP_LENGTH);
    _assert(DEFAULT_MIN_STRIP_LENGTH, DEFAULT_MIN_STRIP_LENGTH);
    _assert(1_000_000., 1_000_000.);
}

#[test]
fn test_right_hand_rule_polygon() {
    let polygon1 = Geometry::Polygon(polygon![(x: -1., y: 1.), (x: 1., y: 1.), (x: 1., y: -1.), (x: -1., y: -1.)]);
    let polygon2 = Geometry::Polygon(polygon![(x: -1., y: 1.), (x: -1., y: -1.), (x: 1., y: -1.), (x: 1., y: 1.)]);
    let expected = Geometry::Polygon(polygon![(x: -1., y: 1.), (x: -1., y: -1.), (x: 1., y: -1.), (x: 1., y: 1.)]);
    for polygon in [polygon1, polygon2] {
        let result = right_hand_rule(&polygon);
        assert_eq!(result, expected);
        let polygon_result: Result<Polygon<f64>, _> = result.try_into();
        assert!(polygon_result.unwrap().exterior().is_ccw());
    }
}

#[test]
fn test_right_hand_rule_multipolygon() {
    let expected = Geometry::MultiPolygon(MultiPolygon::new(vec![
        polygon![(x: -1., y: 1.), (x: -1., y: -1.), (x: 1., y: -1.), (x: 1., y: 1.), (x: -1., y: 1.)],
        polygon![(x:  3., y: 1.), (x:  3., y: -1.), (x: 5., y: -1.), (x: 5., y: 1.), (x:  3., y: 1.)],
    ]));
    let multi_polygon1 = Geometry::MultiPolygon(MultiPolygon::new(vec![
        polygon![(x: -1., y: 1.), (x: -1., y: -1.), (x: 1., y: -1.), (x: 1., y: 1.), (x: -1., y: 1.)],
        polygon![(x:  3., y: 1.), (x:  3., y: -1.), (x: 5., y: -1.), (x: 5., y: 1.), (x:  3., y: 1.)],
    ]));
    let multi_polygon2 = Geometry::MultiPolygon(MultiPolygon::new(vec![
        polygon![(x: -1., y: 1.), (x: 1., y: 1.), (x: 1., y: -1.), (x: -1., y: -1.), (x: -1., y: 1.)],
        polygon![(x:  3., y: 1.), (x: 5., y: 1.), (x: 5., y: -1.), (x:  3., y: -1.), (x:  3., y: 1.)],
    ]));
    for multi_polygon in [multi_polygon1, multi_polygon2] {
        let result = right_hand_rule(&multi_polygon);
        assert_eq!(result, expected);
        let multi_polygon_result: Result<MultiPolygon<f64>, _> = result.try_into();
        assert!(multi_polygon_result.unwrap().iter().all(|p| p.exterior().is_ccw()));
    }
}

#[test]
fn test_count_lines() {
    // Default values: strip_width = 5000., min_overlap = 220:
    assert_eq!(count_lines(1., 5000., 220.), Some(1));
    assert_eq!(count_lines(2000., 5000., 220.), Some(1));
    assert_eq!(count_lines(5000., 5000., 220.), Some(1));
    assert_eq!(count_lines(5001., 5000., 220.), Some(2));
    assert_eq!(count_lines(7000., 5000., 220.), Some(2));
    assert_eq!(count_lines(9779., 5000., 220.), Some(2));
    assert_eq!(count_lines(9780., 5000., 220.), Some(2));
    assert_eq!(count_lines(9781., 5000., 220.), Some(3));
    assert_eq!(count_lines(12000., 5000., 220.), Some(3));
    assert_eq!(count_lines(14559., 5000., 220.), Some(3));
    assert_eq!(count_lines(14560., 5000., 220.), Some(3));
    assert_eq!(count_lines(14561., 5000., 220.), Some(4));
    assert_eq!(count_lines(17000., 5000., 220.), Some(4));
    assert_eq!(count_lines(19339., 5000., 220.), Some(4));
    assert_eq!(count_lines(19340., 5000., 220.), Some(4));
    assert_eq!(count_lines(19341., 5000., 220.), Some(5));
    assert_eq!(count_lines(22000., 5000., 220.), Some(5));
    assert_eq!(count_lines(24119., 5000., 220.), Some(5));
    assert_eq!(count_lines(24120., 5000., 220.), Some(5));
    assert_eq!(count_lines(24121., 5000., 220.), Some(6));
    // Different Values:
    assert_eq!(count_lines(1., 1000., 100.), Some(1));
    assert_eq!(count_lines(1000., 1000., 100.), Some(1));
    assert_eq!(count_lines(1001., 1000., 100.), Some(2));
    assert_eq!(count_lines(1899., 1000., 100.), Some(2));
    assert_eq!(count_lines(1900., 1000., 100.), Some(2));
    assert_eq!(count_lines(1901., 1000., 100.), Some(3));
    assert_eq!(count_lines(2799., 1000., 100.), Some(3));
    assert_eq!(count_lines(2800., 1000., 100.), Some(3));
    assert_eq!(count_lines(2801., 1000., 100.), Some(4));
    // Zero overlap:
    assert_eq!(count_lines(1., 1000., 0.), Some(1));
    assert_eq!(count_lines(1000., 1000., 0.), Some(1));
    assert_eq!(count_lines(1001., 1000., 0.), Some(2));
    assert_eq!(count_lines(2000., 1000., 0.), Some(2));
    assert_eq!(count_lines(2001., 1000., 0.), Some(3));
    assert_eq!(count_lines(3000., 1000., 0.), Some(3));
    assert_eq!(count_lines(3001., 1000., 0.), Some(4));
}

#[test]
fn test_iterate_normalized_geometry() {
    let identity = |g: &Geometry| Some(g.clone());

    // Two LineStrings (no Z — geo crate has no Z support) → 2 results:
    let result = iterate_normalized_geometry(
        &[
            Geometry::LineString(line_string![(x: -100_000., y: -1_000.), (x: 100_000., y: -1_000.)]),
            Geometry::LineString(line_string![(x: -100_000., y:  6_000.), (x: 100_000., y:  6_000.)]),
        ],
        identity,
    );
    assert_eq!(result.len(), 2, "Two LineStrings yield 2 geometries");

    // Single Polygon → 1 result:
    let result = iterate_normalized_geometry(
        &[Geometry::Polygon(polygon![
            (x: 0., y: 0.), (x: 1., y: 0.), (x: 1., y: 1.), (x: 0., y: 1.), (x: 0., y: 0.),
        ])],
        identity,
    );
    assert_eq!(result.len(), 1, "Single Polygon yields 1 geometry");

    // MultiPolygon with 6 sub-polygons → 6 results:
    let make_poly = |dx: f64, dy: f64| polygon![(x: dx, y: dy), (x: dx + 1., y: dy), (x: dx + 1., y: dy + 1.), (x: dx, y: dy + 1.), (x: dx, y: dy)];
    let result = iterate_normalized_geometry(
        &[Geometry::MultiPolygon(MultiPolygon::new(vec![
            make_poly(0., 0.),
            make_poly(2., 0.),
            make_poly(4., 0.),
            make_poly(0., 2.),
            make_poly(2., 2.),
            make_poly(4., 2.),
        ]))],
        identity,
    );
    assert_eq!(result.len(), 6, "MultiPolygon with 6 sub-polygons yields 6 geometries");
}

#[test]
fn test_count_lines_errors() {
    assert!(count_lines(2000., 1000., 1000.).is_none());
    assert!(count_lines(0., 0., 0.).is_none());
    assert!(count_lines(2000., 1000., 1001.).is_none());
    assert!(count_lines(1000., 0., 10.).is_none());
    assert!(count_lines(10000., 5000., 2500.).is_none());
}

#[test]
fn test_project_strip() {
    let input_line = Line::new(coord! { x: 0., y: 0. }, coord! { x: 0., y: 10_000. });
    let output_polygon = project_strip(&input_line, DEFAULT_STRIP_WIDTH).unwrap();
    assert_eq!(output_polygon.unsigned_area(), 10_000. * DEFAULT_STRIP_WIDTH);
    assert_eq!(output_polygon.centroid().unwrap(), input_line.centroid());
    assert!(project_strip(&input_line, 0.0).is_none());
    assert!(project_strip(&input_line, -1.0).is_none());
    assert!(project_strip(&input_line, f64::NAN).is_none());
}

#[test]
fn test_coerce_to_polygon_from_point() {
    let result = coerce_to_polygon(&Geometry::Point(point! { x: 0., y: 0. }), 1000.).unwrap();
    assert_eq!(result.unsigned_area().round(), 500_107_f64); // math.pi * (1_000 / 2 - 100) ** 2
    assert_eq!(result.exterior().coords_count(), 37); // 4 * 9 + 1
    // size below 2 * CIRCLE_EXPANSION_CORRECTION yields a non-positive circle radius → None.
    assert!(coerce_to_polygon(&Geometry::Point(point! { x: 0., y: 0. }), 100.).is_none());
    assert!(coerce_to_polygon(&Geometry::Point(point! { x: 0., y: 0. }), 0.).is_none());
    assert!(coerce_to_polygon(&Geometry::Point(point! { x: 0., y: 0. }), f64::NAN).is_none());
}

#[test]
fn test_coerce_to_polygon_from_line() {
    let result = coerce_to_polygon(
        &Geometry::Line(Line::new(coord! { x: 0., y: 0. }, coord! { x: 10_000., y: 0. })),
        1000.,
    )
    .unwrap();
    assert_eq!(result.unsigned_area(), 10_000_000_f64);
    assert_eq!(result.exterior().coords_count(), 5);
}

#[test]
fn test_coerce_to_polygon_from_polygon() {
    let result = coerce_to_polygon(
        &Geometry::Polygon(polygon![
            (x:1000., y: 1000.),
            (x: 0., y: 1000.),
            (x: 0., y: 0.),
            (x: 1000., y: 0.),
            (x:1000., y: 1000.),
        ]),
        1000.,
    )
    .unwrap();
    assert_eq!(result.unsigned_area(), 1_000_000_f64);
    assert_eq!(result.exterior().coords_count(), 5);
}

#[test]
fn test_count_lines_nan_inputs() {
    assert!(count_lines(f64::NAN, 5000., 220.).is_none());
    assert!(count_lines(10_000., f64::NAN, 220.).is_none());
    assert!(count_lines(10_000., 5000., f64::NAN).is_none());
    assert!(count_lines(f64::INFINITY, 5000., 220.).is_none());
}

#[test]
fn test_count_lines_saturation_overflow() {
    // roll_line_length / strip_width saturates `as u16` to u16::MAX; `checked_add(1)` then returns None.
    assert!(count_lines(1e20, 1.0, 0.0).is_none());
}

#[test]
fn test_distribute_points_zero() {
    let line = Line::new(coord! { x: 0., y: 0. }, coord! { x: 10., y: 10. });
    assert!(distribute_points(&line, 0).is_empty());
}

#[test]
fn test_resize_line_invalid_length_returns_input() {
    let line = Line::new(coord! { x: 10., y: 10. }, coord! { x: 13., y: 14. });
    assert_eq!(resize_line(&line, -5.0), line);
    assert_eq!(resize_line(&line, f64::NAN), line);
    assert_eq!(resize_line(&line, f64::INFINITY), line);
    assert_eq!(resize_line(&line, f64::NEG_INFINITY), line);
}

#[test]
fn test_substring_nan_returns_input() {
    let line = Line::new(coord! { x: 0., y: 0. }, coord! { x: 10., y: 0. });
    assert_eq!(substring(&line, f64::NAN, 5.0), line);
    assert_eq!(substring(&line, 1.0, f64::NAN), line);
    assert_eq!(substring(&line, f64::NAN, f64::NAN), line);
}

#[test]
fn test_segment_lines_invalid_args_returns_input() {
    let input = [Line::new(coord! { x: 0., y: 0. }, coord! { x: 1_000_000., y: 0. })];
    assert_eq!(segment_lines(&input, 0.0, DEFAULT_STRIP_WIDTH), input.to_vec());
    assert_eq!(segment_lines(&input, -1.0, DEFAULT_STRIP_WIDTH), input.to_vec());
    assert_eq!(segment_lines(&input, f64::NAN, DEFAULT_STRIP_WIDTH), input.to_vec());
    assert_eq!(segment_lines(&input, DEFAULT_MAX_STRIP_LENGTH, 0.0), input.to_vec());
    assert_eq!(
        segment_lines(&input, DEFAULT_MAX_STRIP_LENGTH, f64::NAN),
        input.to_vec()
    );
}

#[test]
fn test_iterate_shards_invalid_args_yields_single_shard() {
    let poly = polygon![
        (x: 0., y: 0.),
        (x: 100_000., y: 0.),
        (x: 100_000., y: 100_000.),
        (x: 0., y: 100_000.),
        (x: 0., y: 0.),
    ];
    assert_eq!(iterate_shards(&poly, 0.0, DEFAULT_SHARD_DENSITY_RATIO).count(), 1);
    assert_eq!(iterate_shards(&poly, -1.0, DEFAULT_SHARD_DENSITY_RATIO).count(), 1);
    assert_eq!(iterate_shards(&poly, f64::NAN, DEFAULT_SHARD_DENSITY_RATIO).count(), 1);
    assert_eq!(iterate_shards(&poly, DEFAULT_SHARD_RADIUS, f64::NAN).count(), 1);
}

#[test]
fn test_is_too_small_degenerate_polygon_is_false() {
    // A polygon with an empty exterior has no bounding rect; the conservative default is `false`.
    let poly = Polygon::new(LineString::new(vec![]), vec![]);
    assert!(!is_too_small(&poly, DEFAULT_MIN_STRIP_LENGTH));
}

#[test]
fn test_coerce_to_polygon_invalid_size() {
    let p = Geometry::Point(point! { x: 0., y: 0. });
    assert!(coerce_to_polygon(&p, -1.0).is_none());
    assert!(coerce_to_polygon(&p, f64::INFINITY).is_none());
    let l = Geometry::Line(Line::new(coord! { x: 0., y: 0. }, coord! { x: 100., y: 0. }));
    assert!(coerce_to_polygon(&l, -1.0).is_none());
    assert!(coerce_to_polygon(&l, f64::NAN).is_none());
    let poly_geom = Geometry::Polygon(polygon![(x: 0., y: 0.), (x: 1., y: 0.), (x: 0., y: 1.)]);
    assert!(coerce_to_polygon(&poly_geom, -1.0).is_none());
    assert!(coerce_to_polygon(&poly_geom, f64::NAN).is_none());
}

#[test]
fn test_iterate_geometry_point_and_linestring() {
    let p = Geometry::Point(point! { x: 1., y: 2. });
    let ls = Geometry::LineString(line_string![(x: 0., y: 0.), (x: 1., y: 1.)]);
    assert_eq!(iterate_geometry(&p).collect::<Vec<_>>(), vec![p.clone()]);
    assert_eq!(iterate_geometry(&ls).collect::<Vec<_>>(), vec![ls.clone()]);
}

#[test]
fn test_iterate_geometry_line_rect_triangle() {
    let line = Line::new(coord! { x: 0., y: 0. }, coord! { x: 10., y: 0. });
    let rect = Rect::new(coord! { x: 0., y: 0. }, coord! { x: 5., y: 5. });
    let tri = Triangle::new(
        coord! { x: 0., y: 0. },
        coord! { x: 1., y: 0. },
        coord! { x: 0., y: 1. },
    );
    let from_line: Vec<Geometry> = iterate_geometry(&Geometry::Line(line)).collect();
    let from_rect: Vec<Geometry> = iterate_geometry(&Geometry::Rect(rect)).collect();
    let from_tri: Vec<Geometry> = iterate_geometry(&Geometry::Triangle(tri)).collect();
    assert!(matches!(from_line.as_slice(), [Geometry::LineString(_)]));
    assert!(matches!(from_rect.as_slice(), [Geometry::Polygon(_)]));
    assert!(matches!(from_tri.as_slice(), [Geometry::Polygon(_)]));
}

#[test]
fn test_iterate_geometry_multi_points_and_lines() {
    let mp = Geometry::MultiPoint(MultiPoint::new(vec![
        point! { x: 0., y: 0. },
        point! { x: 1., y: 1. },
        point! { x: 2., y: 2. },
    ]));
    let mls = Geometry::MultiLineString(MultiLineString::new(vec![
        line_string![(x: 0., y: 0.), (x: 1., y: 1.)],
        line_string![(x: 2., y: 2.), (x: 3., y: 3.)],
    ]));
    let pts: Vec<Geometry> = iterate_geometry(&mp).collect();
    let lines: Vec<Geometry> = iterate_geometry(&mls).collect();
    assert_eq!(pts.len(), 3);
    assert!(pts.iter().all(|g| matches!(g, Geometry::Point(_))));
    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|g| matches!(g, Geometry::LineString(_))));
}

#[test]
fn test_iterate_geometry_nested_collection_is_dropped() {
    // Flat GC: items pass through.
    let inner_point = Geometry::Point(point! { x: 5., y: 5. });
    let inner_collection = Geometry::GeometryCollection(GeometryCollection::new_from(vec![
        Geometry::Point(point! { x: 99., y: 99. }),
        Geometry::Point(point! { x: 100., y: 100. }),
    ]));
    // A GC containing one Point and one nested GC; only the Point should survive.
    let outer = Geometry::GeometryCollection(GeometryCollection::new_from(vec![
        inner_point.clone(),
        inner_collection,
    ]));
    let out: Vec<Geometry> = iterate_geometry(&outer).collect();
    assert_eq!(out, vec![inner_point]);
}

#[test]
fn test_furthest_from_centroid_degenerate_polygon() {
    let degenerate = Polygon::new(LineString::new(vec![]), vec![]);
    let pd = furthest_from_centroid(&degenerate);
    assert_eq!(pd.point, point! { x: 0., y: 0. });
    assert_eq!(pd.distance, 0.0);
}

#[test]
fn test_get_rectangular_boundary_lines_degenerate_returns_none() {
    let single_point = Geometry::Point(point! { x: 0., y: 0. });
    assert!(get_rectangular_boundary_lines(&single_point, None, None).is_none());
}

#[test]
fn test_roll_lines_degenerate_returns_none() {
    let single_point = Geometry::Point(point! { x: 0., y: 0. });
    assert!(roll_lines(&single_point, None, None).is_none());
}

#[test]
fn test_polygon_ccw_no_holes_drops_interiors_and_flips_cw() {
    let cw_with_hole = Polygon::new(
        line_string![(x: 0., y: 0.), (x: 0., y: 10.), (x: 10., y: 10.), (x: 10., y: 0.), (x: 0., y: 0.)],
        vec![line_string![(x: 2., y: 2.), (x: 8., y: 2.), (x: 8., y: 8.), (x: 2., y: 8.), (x: 2., y: 2.)]],
    );
    assert!(!cw_with_hole.exterior().is_ccw(), "input is CW");
    let out = polygon_ccw_no_holes(&cw_with_hole);
    assert!(out.exterior().is_ccw(), "output must be CCW");
    assert!(out.interiors().is_empty(), "interiors must be dropped");
}

#[test]
fn test_right_hand_rule_passthrough_types() {
    let p = Geometry::Point(point! { x: 1., y: 2. });
    let ls = Geometry::LineString(line_string![(x: 0., y: 0.), (x: 1., y: 1.)]);
    let line = Geometry::Line(Line::new(coord! { x: 0., y: 0. }, coord! { x: 1., y: 1. }));
    assert_eq!(right_hand_rule(&p), p);
    assert_eq!(right_hand_rule(&ls), ls);
    assert_eq!(right_hand_rule(&line), line);
}

#[test]
fn test_iterate_normalized_geometry_flips_cw_polygon() {
    // CW polygon is valid but violates the CCW invariant. fix_geometry's winding-normalize path.
    let cw = Polygon::new(
        line_string![(x: 0., y: 0.), (x: 0., y: 10.), (x: 10., y: 10.), (x: 10., y: 0.), (x: 0., y: 0.)],
        vec![],
    );
    assert!(!cw.exterior().is_ccw());
    let out = iterate_normalized_geometry(&[Geometry::Polygon(cw)], |g| Some(g.clone()));
    assert_eq!(out.len(), 1);
    if let Geometry::Polygon(p) = &out[0] {
        assert!(p.exterior().is_ccw(), "fix_geometry must return CCW exterior");
    } else {
        panic!("expected Polygon");
    }
}

#[test]
fn test_iterate_normalized_geometry_repairs_bowtie() {
    // Self-intersecting "bow-tie" polygon: invalid; fix_geometry should repair via self-union.
    let bowtie = Polygon::new(
        line_string![(x: 0., y: 0.), (x: 10., y: 10.), (x: 0., y: 10.), (x: 10., y: 0.), (x: 0., y: 0.)],
        vec![],
    );
    let out = iterate_normalized_geometry(&[Geometry::Polygon(bowtie)], |g| Some(g.clone()));
    assert!(!out.is_empty(), "bow-tie must be repaired, not dropped");
    if let Geometry::Polygon(p) = &out[0] {
        use geo::Validation;
        assert!(p.is_valid(), "repaired polygon must be valid");
    } else {
        panic!("expected Polygon");
    }
}

#[test]
fn test_get_boundary_lines_with_heading() {
    // A simple axis-aligned rectangle. With any heading the result should still be
    // a valid 4-line rectangle (opposing sides equal length).
    let a_poly = polygon![
        (x: 0., y: 0.), (x: 10_000., y: 0.), (x: 10_000., y: 5_000.),
        (x: 0., y: 5_000.), (x: 0., y: 0.),
    ];
    for angle in [15.0_f64, 45.0, 60.0, 120.0] {
        let result = get_rectangular_boundary_lines(&Geometry::Polygon(a_poly.clone()), Some(angle), None).unwrap();
        let lengths = result.map(|l| format!("{:.6}", Euclidean.length(&l)));
        assert_eq!(lengths[0], lengths[2], "opposite sides equal at angle {angle}");
        assert_eq!(lengths[1], lengths[3], "opposite sides equal at angle {angle}");
    }
}

#[test]
fn test_roll_lines_heading_changes_orientation() {
    // A wide rectangle (10_000 x 2_000). Without heading (auto MBR), roll lines span ~2_000
    // (the short sides). With heading ≈ 0 (N-S strips), roll lines span ~10_000 (the E-W
    // width), because the perpendicular sides of the rotated envelope run across the long axis.
    let a_poly = polygon![
        (x: 0., y: 0.), (x: 10_000., y: 0.), (x: 10_000., y: 2_000.),
        (x: 0., y: 2_000.), (x: 0., y: 0.),
    ];
    let geo = Geometry::Polygon(a_poly);
    let auto_lines = roll_lines(&geo, None, None).unwrap();
    let angled_lines = roll_lines(&geo, Some(0.1), None).unwrap();
    let auto_len = Euclidean.length(&auto_lines[0]);
    let angled_len = Euclidean.length(&angled_lines[0]);
    assert!(
        (auto_len - angled_len).abs() > 100.0,
        "heading should change roll line length (got {auto_len:.0} vs {angled_len:.0})"
    );
}

#[test]
fn test_roll_lines_right_angle_multiples_dont_panic() {
    // Exact multiples of 90° are nudged internally; they should return a result, not None.
    let a_poly = polygon![
        (x: 0., y: 0.), (x: 10_000., y: 0.), (x: 10_000., y: 5_000.),
        (x: 0., y: 5_000.), (x: 0., y: 0.),
    ];
    let geo = Geometry::Polygon(a_poly);
    for angle in [0.0_f64, 90.0, 180.0, 270.0, 360.0] {
        assert!(
            roll_lines(&geo, Some(angle), None).is_some(),
            "roll_lines must succeed for exact right-angle multiple {angle}"
        );
    }
}

#[test]
fn test_iterate_normalized_geometry_passthrough_point_and_line() {
    // Non-polygon geometries bypass the repair steps entirely.
    let p = Geometry::Point(point! { x: 5., y: 5. });
    let ls = Geometry::LineString(line_string![(x: 0., y: 0.), (x: 1., y: 1.)]);
    let out = iterate_normalized_geometry(&[p.clone(), ls.clone()], |g| Some(g.clone()));
    assert_eq!(out.len(), 2);
    assert_eq!(out[0], p);
    assert_eq!(out[1], ls);
}

#[test]
fn test_clip_lines_by_polygons() {
    let lines = vec![
        line_string![(x: -3., y:  0.5), (x: 3., y:  0.5)],
        line_string![(x: -3., y: -0.5), (x: 3., y: -0.5)],
    ];
    let polygons = vec![
        polygon![(x: 2., y: 1.), (x: 2., y: -1.), (x: 3., y: -1.), (x: 3., y: 1.), (x: 2., y: 1.)],
        polygon![(x: -1., y: 1.), (x: -1., y: -1.), (x: 1., y: -1.), (x: 1., y: 1.), (x: -1., y: 1.)],
    ];

    let result = clip_lines_by_polygons(&lines, &polygons);

    assert_eq!(result.len(), 4);

    let mut lengths: Vec<f64> = result.iter().map(|ls| Euclidean.length(ls)).collect();
    lengths.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!(
        (lengths[0] - 1.0).abs() < 1e-10,
        "expected length 1, got {}",
        lengths[0]
    );
    assert!(
        (lengths[1] - 1.0).abs() < 1e-10,
        "expected length 1, got {}",
        lengths[1]
    );
    assert!(
        (lengths[2] - 2.0).abs() < 1e-10,
        "expected length 2, got {}",
        lengths[2]
    );
    assert!(
        (lengths[3] - 2.0).abs() < 1e-10,
        "expected length 2, got {}",
        lengths[3]
    );
}
