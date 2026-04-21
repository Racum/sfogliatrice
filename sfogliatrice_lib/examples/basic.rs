use geo::{Geometry, polygon};
use sfogliatrice_lib::{Config, tessellate};

fn main() {
    // Get your geometry ready:
    let polygon = Geometry::Polygon(polygon![
        (x: -15.332574, y: 28.217488),
        (x: -15.865546, y: 28.217488),
        (x: -15.865546, y: 27.719770),
        (x: -15.332574, y: 27.719770),
    ]);

    // Set your tessellation options:
    let config = Config {
        strip_width: 10_000.0,
        ..Config::default()
    };

    // Run tessellation:
    let result = tessellate(&[polygon], &config);

    // Use the results:
    println!("Targets: {}", result.targets.len());
}
