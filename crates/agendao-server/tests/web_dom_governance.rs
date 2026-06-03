use std::path::PathBuf;
use std::process::Command;

#[test]
fn web_dom_governance_suite_passes() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let test_file = manifest_dir.join("web").join("app.dom.test.mjs");

    if !test_file.is_file() {
        eprintln!(
            "skipping web DOM governance tests; {} is not present",
            test_file.display()
        );
        return;
    }

    let output = Command::new("node")
        .arg("--test")
        .arg(&test_file)
        .output()
        .expect("node must be available to run web DOM governance tests");

    if !output.status.success() {
        panic!(
            "web DOM governance tests failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
