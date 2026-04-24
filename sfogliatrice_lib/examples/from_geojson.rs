use serde_json::json;
use sfogliatrice_lib::{Config, tessellate_geojson};

fn main() {
    // Get your GeoJSON ready:
    let geojson = json!({
        "type": "Polygon",
        "coordinates": [[
            [-15.332574, 28.217488],
            [-15.865546, 28.217488],
            [-15.865546, 27.719770],
            [-15.332574, 27.719770],
            [-15.332574, 28.217488]
        ]]
    });

    // Set your tessellation options:
    let config = Config {
        strip_width: 10_000.0,
        ..Config::default()
    };

    // Run tessellation:
    let result = tessellate_geojson(&geojson, &config);

    // Use the results:
    println!("Targets: {}", result.targets.len());
}
