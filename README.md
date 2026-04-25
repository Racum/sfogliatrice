# Sfogliatrice – Geometry Tessellation Tool

<img src="https://raw.githubusercontent.com/Racum/sfogliatrice/main/assets/sfogliatrice.png" width="300">

*"Sfogliatrice" = "Mechanized Pasta Cutter"*

Sfogliatrice is both a **CLI tool** and a **Rust library** to handle tessellation of complex GeoJSON inputs. It understands the full set of GeoJSON types, and can process thousands of elements in a few seconds because of its intermediate optimization step.

## CLI Tool

### Installation

To use Sfogliatrice, you can simply install it via cargo:

```bash
$ cargo install sfogliatrice
```

### Usage

```bash
$ sfogliatrice --help
Tessellate GeoJSON geometries into satellite survey targets and coverages.

Usage: sfogliatrice [OPTIONS] <GEOJSON_FILE>

Arguments:
  <GEOJSON_FILE>  Input GeoJSON file (use - for stdin)

Options:
  -n, --no-targets                 Hide target points and lines
  -c, --coverages                  Show coverage polygons
  -i, --intermediates              Show polygons from intermediate step
  -o, --original                   Show original input elements
  -a, --all                        Show everything (targets, coverages, intermediates, original)
  -e, --expansion <EXPANSION>      Target expansion in meters [default: 5000]
  -w, --width <WIDTH>              Strip width in meters [default: 5000]
  -l, --max-length <MAX_LENGTH>    Strip max length in meters [default: 50000]
  -m, --min-overlap <MIN_OVERLAP>  Minimum strip overlap in meters [default: 200]
  -f, --full-precision             Do not round coordinate decimal points
  -p, --pretty                     Pretty-print JSON output
      --line-targets               Force all targets as lines (no points)
      --square-coverages           Show point coverages as squares
      --heading <HEADING>          Target heading angle in degrees (0.0 means north to south)
  -v, --version                    Print version information
  -h, --help                       Print help
```

For the following examples, consider this polygon over the Gran Canaria island:

<img src="https://raw.githubusercontent.com/Racum/sfogliatrice/main/assets/gran_canaria_input.png" width="400">

### Targets

Targets are the result of tessellation, meaning the points and lines that would be sent to satellite tasking. Sfogliatrice always outputs the tessellation results to stdout as GeoJSON. Given the common use-case scenario for this tool, targets are enabled by default, but they can be ignored with the `-n` / `--no-targets` option.

```bash
$ sfogliatrice fixtures/gran_canaria.geojson
```

<img src="https://raw.githubusercontent.com/Racum/sfogliatrice/main/assets/gran_canaria_targets.png" width="400">

### Coverages

Coverages are the projected area from the targets. They are also known as “strips”, but since this tool also returns points that could be projected as circles, the name “strips” does not convey the whole meaning.

To return the coverages in _addition to the targets_ (in the same GeoJSON structure), use the option `-c` / `--coverages`. Combining `-c` and `-n` would result in _only_ returning the coverages.

```bash
$ sfogliatrice -n -c fixtures/gran_canaria.geojson
```

<img src="https://raw.githubusercontent.com/Racum/sfogliatrice/main/assets/gran_canaria_coverages.png" width="400">


### Intermediates

Intermediates are the polygons representing the intermediate state of the tessellation algorithm, when we try to combine nearby polygons to optimize the results.

This can be used for debug or as a pre-processing step to tessellate using different libraries/tools.

To return the intermediates, use the option `-i` / `--intermediates`.

```bash
$ sfogliatrice -n -i fixtures/gran_canaria.geojson
```

<img src="https://raw.githubusercontent.com/Racum/sfogliatrice/main/assets/gran_canaria_intermediates.png" width="400">

### Compatibility Options

#### Full Precision

As an optimization step, the tool reduces the precision of the coordinates to 6 decimal points, if you are having problems with that, or got an invalid GeoJSON output, try using the full precision with the option `-f`, `--full-precision`.

```bash
$ sfogliatrice fixtures/gran_canaria.geojson | jq -c '.features[0].geometry.coordinates'
[[-15.388633,28.192086],[-15.365164,27.847629]]

$ sfogliatrice -f fixtures/gran_canaria.geojson | jq -c '.features[0].geometry.coordinates'
[[-15.388633491248596,28.192085832235954],[-15.365164062590058,27.847629496691788]]
```

#### Line Targets

If you are integrating with a system that expects **only** LineStrings (no Points), use the option `--line-targets` to force all points into lines.

#### Square Coverages

If you prefer to see the coverages as squares, even coming from points, use the option `--square-coverages`. Please notice that you also need to use `-c` / `--coverages` to see the effect of this option, given it doesn’t affect the tessellation itself.

#### Custom Heading

By default, the tool automatically finds the optimal strip orientation for each polygon using its minimum rotated bounding rectangle. If you need strips to run in a specific direction use `--heading` to fix the orientation.

The angle follows a clockwise convention starting from North: `0` means North-South strips, `90` means East-West strips, and so on.

```bash
$ sfogliatrice --heading 45 fixtures/gran_canaria.geojson
```

Notice the 45 degrees lines now:

<img src="https://raw.githubusercontent.com/Racum/sfogliatrice/main/assets/gran_canaria_heading.png" width="400">

## Rust Library

### Installation

To use Sfogliatrice as a library, you can simply install it via cargo:

```bash
$ cargo add sfogliatrice_lib
```

### Usage

From `geo` geometry:

```rust
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

    // Set your tessellation options: (strips 10Km wide)
    let config = Config {strip_width: 10_000.0, ..Config::default()};

    // Run tessellation:
    let result = tessellate(&[polygon], &config);

    // Use the results:
    println!("Targets: {}", result.targets.len());  // Outputs: Targets: 10
}
```

From Serde GeoJSON:

```rust
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
    let config = Config { strip_width: 10_000.0, ..Config::default() };

    // Run tessellation:
    let result = tessellate_geojson(&geojson, &config);

    // Use the results:
    println!("Targets: {}", result.targets.len());  // Outputs: Targets: 10
}
```

The CLI parameters actually just follow the library’s Config parameters; check the notes on the CLI usage and you’ll know how to set this:

```rust
pub struct Config {
    pub strip_width: f64,
    pub min_strip_length: f64,
    pub max_strip_length: f64,
    pub min_overlap: f64,
    pub expansion: f64,
    pub shard_density_ratio: f64,
    pub shard_radius: f64,
    pub force_line_targets: bool,
    pub force_square_coverages: bool,
    pub heading: Option<f64>,
}
```

### WebAssenbly Package

Before you build it, you need some requirements first:

```
$ rustup target add wasm32-unknown-unknown
$ cargo install wasm-pack
```

Than, on `sfogliatrice_wasm` folder, run:

```
$ wasm-pack build --target web
$ python3 -m http.server 8080
```

That will build the WASM package and run a webserver, than, open the page http://localhost:8080/test.html

There is also an online demo here: https://racum.blog/sfogliatrice/

## Contributing

### Code Contribution

Just create a PR, but try to follow some basic guidelines:

- Look at the current structure and try to emulate it.
- One format per file, unless they are related.
- Run `cargo fmt` and `cargo clippy` before committing.


## License

**Sfogliatrice** is under the MIT License.
