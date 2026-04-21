use std::io::Write;
use std::process::{Command, Stdio};

const SAMPLE_POLYGON: &str = r#"{"type":"Polygon","coordinates":[[[13.332607,52.520232],[13.378726,52.520232],[13.378726,52.504324],[13.332607,52.504324],[13.332607,52.520232]]]}"#;

#[test]
fn test_stdin_pipe() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sfogliatrice"))
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn binary");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(SAMPLE_POLYGON.as_bytes())
        .expect("failed to write to stdin");

    let output = child.wait_with_output().expect("failed to wait on child");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("output must be valid JSON");
    assert_eq!(v["type"], "FeatureCollection");
    assert!(v["features"].as_array().unwrap().len() > 0);
}

#[test]
fn test_broken_stdout_pipe_exits_cleanly() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sfogliatrice"))
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("failed to spawn binary");

    // Drop the read end before writing to stdin. The binary blocks on stdin first,
    // so by the time it tries to write output the pipe is guaranteed to be broken.
    drop(child.stdout.take());

    child
        .stdin
        .take()
        .unwrap()
        .write_all(SAMPLE_POLYGON.as_bytes())
        .expect("failed to write to stdin");

    let status = child.wait().expect("failed to wait on child");
    assert!(
        status.success(),
        "broken pipe must not cause a non-zero exit: {status:?}"
    );
}
