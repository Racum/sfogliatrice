use clap::Parser;
use serde_json::{Value, json};
use sfogliatrice_lib::defaults::{
    DEFAULT_MAX_STRIP_LENGTH, DEFAULT_MIN_OVERLAP, DEFAULT_STRIP_WIDTH, DEFAULT_TARGET_EXPANSION,
};
use sfogliatrice_lib::geojson::{combine_geojson, coord_precision};
use sfogliatrice_lib::tessellate_geojson_to_geojson;
use sfogliatrice_lib::types::Config; // ConfigError via Display on Config::new
use std::fs;
use std::io::{self, Read, Write};

#[derive(Parser)]
#[command(
    name = "sfogliatrice",
    version = env!("CARGO_PKG_VERSION"),
    disable_version_flag = true,
    about = "Tessellate GeoJSON geometries into satellite survey targets and coverages."
)]
struct Cli {
    /// Input GeoJSON file (use - for stdin)
    geojson_file: String,

    /// Hide target points and lines.
    #[arg(short = 'n', long = "no-targets")]
    no_targets: bool,

    /// Show coverage polygons.
    #[arg(short = 'c', long = "coverages")]
    coverages: bool,

    /// Show polygons from intermediate step.
    #[arg(short = 'i', long = "intermediates")]
    intermediates: bool,

    /// Show original input elements.
    #[arg(short = 'o', long = "original")]
    original: bool,

    /// Show everything (targets, coverages, intermediates, original).
    #[arg(short = 'a', long = "all")]
    all: bool,

    /// Target expansion in meters.
    #[arg(short = 'e', long = "expansion", default_value_t = DEFAULT_TARGET_EXPANSION)]
    expansion: f64,

    /// Strip width in meters.
    #[arg(short = 'w', long = "width", default_value_t = DEFAULT_STRIP_WIDTH)]
    width: f64,

    /// Strip max length in meters.
    #[arg(short = 'l', long = "max-length", default_value_t = DEFAULT_MAX_STRIP_LENGTH)]
    max_length: f64,

    /// Minimum strip overlap in meters.
    #[arg(short = 'm', long = "min-overlap", default_value_t = DEFAULT_MIN_OVERLAP)]
    min_overlap: f64,

    /// Do not round coordinate decimal points.
    #[arg(short = 'f', long = "full-precision")]
    full_precision: bool,

    /// Pretty-print JSON output.
    #[arg(short = 'p', long = "pretty")]
    pretty: bool,

    /// Force all targets as lines (no points).
    #[arg(long = "line-targets")]
    line_targets: bool,

    /// Show point coverages as squares.
    #[arg(long = "square-coverages")]
    square_coverages: bool,

    /// Target heading angle in degrees (0.0 means north to south)
    #[arg(long = "heading")]
    heading: Option<f64>,

    /// Try many headings for better results.
    #[arg(short = 'b', long = "brute-force")]
    brute_force: bool,

    /// Ignore Polygon holes.
    #[arg(long = "ignore-holes")]
    ignore_holes: bool,

    /// Print version information.
    #[arg(short = 'v', long = "version", action = clap::ArgAction::Version)]
    version: (),
}

#[derive(Debug)]
struct RunOutput {
    stdout: String,
    warning: Option<String>,
}

fn run(cli: &Cli, input_str: &str) -> Result<RunOutput, String> {
    let input_geojson: Value = serde_json::from_str(input_str).map_err(|_| "input is not valid JSON.".to_string())?;

    if !matches!(
        input_geojson.get("type").and_then(|t| t.as_str()),
        Some(
            "Point"
                | "MultiPoint"
                | "LineString"
                | "MultiLineString"
                | "Polygon"
                | "MultiPolygon"
                | "GeometryCollection"
                | "Feature"
                | "FeatureCollection"
        )
    ) {
        return Err("input is valid JSON but not a recognized GeoJSON type.".to_string());
    }

    for (name, value) in [
        ("--expansion", cli.expansion),
        ("--width", cli.width),
        ("--max-length", cli.max_length),
        ("--min-overlap", cli.min_overlap),
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(format!("{name} must be a positive finite number, got {value}."));
        }
    }

    if let Some(angle) = cli.heading
        && !angle.is_finite()
    {
        return Err(format!("--heading must be a finite number, got {angle}."));
    }

    let mut config = Config::new(
        cli.expansion,
        cli.width,
        cli.max_length,
        cli.min_overlap,
        cli.line_targets,
        cli.square_coverages,
        cli.max_length, // shard_radius is not exposed as a CLI flag; it intentionally follows
        // --max-length so that shards never exceed the strip length in size.
        cli.heading,
    )
    .map_err(|e| e.to_string())?;
    config.brute_force = cli.brute_force;
    config.ignore_holes = cli.ignore_holes;

    // Flag precedence: targets are shown by default; `--no-targets` hides them; `--all` overrides
    // `--no-targets` and forces every output (targets + coverages + intermediates + original).
    // This is why `--all` appears as an OR on every gate below, and why `!no_targets || all`
    // works out to "show targets unless explicitly suppressed and not overridden by --all".
    let needs_tessellation = !cli.no_targets || cli.coverages || cli.intermediates || cli.all;
    let tessellated = if needs_tessellation {
        Some(tessellate_geojson_to_geojson(&input_geojson, &config))
    } else {
        None
    };

    let mut output_features: Vec<Value> = vec![];

    if cli.original || cli.all {
        let combined = combine_geojson(std::slice::from_ref(&input_geojson));
        if let Some(arr) = combined["features"].as_array() {
            output_features.extend(arr.iter().cloned());
        }
    }

    if let Some(ref t) = tessellated {
        let fc = t.to_feature_collection(
            !cli.no_targets || cli.all,
            cli.coverages || cli.all,
            cli.intermediates || cli.all,
        );
        if let Some(arr) = fc["features"].as_array() {
            output_features.extend(arr.iter().cloned());
        }
    }

    let warning = output_features
        .is_empty()
        .then(|| "no features to output. Use --help to see output options.".to_string());

    let mut output = json!({"type": "FeatureCollection", "features": output_features});
    if !cli.full_precision
        && let Some(features) = output["features"].as_array().cloned()
    {
        let rounded = coord_precision(features, 6);
        output["features"] = Value::Array(rounded);
    }

    let stdout = if cli.pretty {
        serde_json::to_string_pretty(&output)
    } else {
        serde_json::to_string(&output)
    }
    .map_err(|e| format!("failed to serialize output: {e}"))?;

    Ok(RunOutput { stdout, warning })
}

fn main() {
    let cli = Cli::parse();

    let input_str = if cli.geojson_file == "-" {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf).unwrap_or_else(|e| {
            eprintln!("Error: failed to read stdin: {e}");
            std::process::exit(1);
        });
        buf
    } else {
        fs::read_to_string(&cli.geojson_file).unwrap_or_else(|e| {
            eprintln!("Error: cannot read '{}': {e}", cli.geojson_file);
            std::process::exit(1);
        })
    };

    match run(&cli, &input_str) {
        Ok(out) => {
            if let Some(w) = out.warning {
                eprintln!("Warning: {w}");
            }
            if let Err(e) = writeln!(io::stdout(), "{}", out.stdout)
                && e.kind() != io::ErrorKind::BrokenPipe
            {
                eprintln!("Error: failed to write output: {e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn make_cli(args: &[&str]) -> Cli {
        let mut full_args: Vec<&str> = vec!["sfogliatrice", "-"];
        full_args.extend(args);
        Cli::try_parse_from(full_args).expect("cli parse must succeed")
    }

    const SAMPLE_POLYGON: &str = r#"{
        "type": "Polygon",
        "coordinates": [[
            [13.332607, 52.520232],
            [13.378726, 52.520232],
            [13.378726, 52.504324],
            [13.332607, 52.504324],
            [13.332607, 52.520232]
        ]]
    }"#;

    #[test]
    fn test_run_invalid_json_returns_err() {
        let cli = make_cli(&[]);
        let err = run(&cli, "not json").unwrap_err();
        assert!(err.contains("not valid JSON"), "got: {err}");
    }

    #[test]
    fn test_run_unrecognized_geojson_type_returns_err() {
        let cli = make_cli(&[]);
        let err = run(&cli, r#"{"type":"Bogus"}"#).unwrap_err();
        assert!(err.contains("not a recognized GeoJSON type"), "got: {err}");
    }

    #[test]
    fn test_run_rejects_negative_arg() {
        // `=` form is needed so clap doesn't treat `-100` as its own flag.
        let cli = make_cli(&["--width=-100"]);
        let err = run(&cli, SAMPLE_POLYGON).unwrap_err();
        assert!(err.contains("--width"), "got: {err}");
        assert!(err.contains("positive finite"), "got: {err}");
    }

    #[test]
    fn test_run_rejects_zero_arg() {
        let cli = make_cli(&["--expansion", "0"]);
        let err = run(&cli, SAMPLE_POLYGON).unwrap_err();
        assert!(err.contains("--expansion"), "got: {err}");
    }

    #[test]
    fn test_run_rejects_nan_arg() {
        let cli = make_cli(&["--max-length", "NaN"]);
        let err = run(&cli, SAMPLE_POLYGON).unwrap_err();
        assert!(err.contains("--max-length"), "got: {err}");
    }

    #[test]
    fn test_run_config_error_surfaces() {
        // --width above MAX_STRIP_WIDTH (5_000_000) triggers ConfigError::StripWidthTooLarge.
        let cli = make_cli(&["--width", "5000001"]);
        let err = run(&cli, SAMPLE_POLYGON).unwrap_err();
        assert!(err.contains("Width exceeds"), "got: {err}");
    }

    #[test]
    fn test_run_all_overrides_no_targets() {
        let cli = make_cli(&["--all", "--no-targets"]);
        let out = run(&cli, SAMPLE_POLYGON).unwrap();
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        let features = v["features"].as_array().unwrap();
        let has_target = features.iter().any(|f| {
            let t = f["geometry"]["type"].as_str().unwrap_or("");
            t == "Point" || t == "LineString"
        });
        assert!(has_target, "--all must force targets even with --no-targets");
    }

    #[test]
    fn test_run_no_targets_hides_targets_but_keeps_coverages() {
        let cli = make_cli(&["--no-targets", "--coverages"]);
        let out = run(&cli, SAMPLE_POLYGON).unwrap();
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        let features = v["features"].as_array().unwrap();
        assert!(
            features.iter().all(|f| {
                let t = f["geometry"]["type"].as_str().unwrap_or("");
                t != "Point" && t != "LineString"
            }),
            "--no-targets must hide target Points and LineStrings"
        );
        assert!(
            features.iter().any(|f| f["geometry"]["type"] == "Polygon"),
            "coverage polygons must still be in output"
        );
    }

    #[test]
    fn test_run_empty_output_triggers_warning() {
        // No output flags and --no-targets produces an empty FeatureCollection.
        let cli = make_cli(&["--no-targets"]);
        let out = run(&cli, SAMPLE_POLYGON).unwrap();
        assert!(out.warning.is_some(), "empty output must warn");
    }

    #[test]
    fn test_run_default_precision_rounds_to_six_decimals() {
        let cli = make_cli(&["--all"]);
        let out = run(&cli, SAMPLE_POLYGON).unwrap();
        // 13.332607 is exactly 6 decimals; any 7th digit would indicate missing rounding.
        assert!(!out.stdout.contains("13.3326070"), "no trailing 7th decimal");
    }

    #[test]
    fn test_run_full_precision_preserves_decimals() {
        let input = r#"{
            "type":"Polygon",
            "coordinates":[[
                [13.3326071234567, 52.5202321234567],
                [13.378726, 52.520232],
                [13.378726, 52.504324],
                [13.332607, 52.504324],
                [13.3326071234567, 52.5202321234567]
            ]]
        }"#;
        let cli = make_cli(&["--full-precision", "--original"]);
        let out = run(&cli, input).unwrap();
        assert!(
            out.stdout.contains("13.3326071234567"),
            "full-precision must preserve input decimals"
        );
    }

    #[test]
    fn test_run_pretty_output_contains_newlines() {
        let cli = make_cli(&["--all", "--pretty"]);
        let out = run(&cli, SAMPLE_POLYGON).unwrap();
        assert!(out.stdout.contains('\n'), "pretty output must contain newlines");
    }

    #[test]
    fn test_run_compact_output_has_no_newlines() {
        let cli = make_cli(&["--all"]);
        let out = run(&cli, SAMPLE_POLYGON).unwrap();
        assert!(!out.stdout.contains('\n'), "compact output must be single-line");
    }

    #[test]
    fn test_run_heading_produces_output() {
        let cli = make_cli(&["--heading", "45"]);
        let out = run(&cli, SAMPLE_POLYGON).unwrap();
        let v: Value = serde_json::from_str(&out.stdout).unwrap();
        assert!(
            !v["features"].as_array().unwrap().is_empty(),
            "heading must produce features"
        );
    }

    #[test]
    fn test_run_heading_rejects_nan() {
        let cli = make_cli(&["--heading", "NaN"]);
        let err = run(&cli, SAMPLE_POLYGON).unwrap_err();
        assert!(err.contains("--heading"), "got: {err}");
    }
}
