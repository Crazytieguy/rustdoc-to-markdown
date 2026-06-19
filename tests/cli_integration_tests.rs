use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn run_cargo_doc_md(args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("cargo");
    cmd.arg("run").arg("--").args(args);

    let output = cmd.output().expect("Failed to execute command");

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// The host target triple, parsed from `rustc -vV` (e.g. `x86_64-unknown-linux-gnu`).
fn host_triple() -> String {
    let output = Command::new("rustc")
        .args(["-vV"])
        .output()
        .expect("Failed to run rustc");
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find(|line| line.starts_with("host:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .map(String::from)
        .expect("Failed to parse host triple from rustc -vV")
}

#[test]
fn test_flag_validation_json_with_package() {
    let result = run_cargo_doc_md(&["--json", "test.json", "-p", "tokio"]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    // Clap's conflict error message
    assert!(err.contains("cannot be used with") || err.contains("conflicts with"));
}

#[test]
fn test_flag_validation_workspace_with_package() {
    let result = run_cargo_doc_md(&["--workspace", "-p", "tokio"]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    // Clap's conflict error message
    assert!(err.contains("cannot be used with") || err.contains("conflicts with"));
}

#[test]
fn test_json_validation_file_not_found() {
    let result = run_cargo_doc_md(&["--json", "nonexistent_file_12345.json"]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("JSON file not found"));
}

#[test]
fn test_json_validation_path_is_directory() {
    // Create a temporary directory
    let temp_dir = std::env::temp_dir().join("cargo_doc_md_test_dir");
    fs::create_dir_all(&temp_dir).unwrap();

    let result = run_cargo_doc_md(&["--json", temp_dir.to_str().unwrap()]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Path is not a file"));

    // Cleanup
    fs::remove_dir(&temp_dir).ok();
}

#[test]
fn test_help_output() {
    let mut cmd = Command::new("cargo");
    cmd.arg("run").arg("--").arg("--help");

    let output = cmd.output().expect("Failed to execute command");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Combine both streams as help might be in either
    let combined = format!("{}{}", stdout, stderr);

    assert!(output.status.success());
    assert!(
        combined.contains("Generate markdown documentation")
            || combined.contains("Cargo subcommand")
    );
    assert!(combined.contains("--json"));
    assert!(combined.contains("--workspace"));
    assert!(combined.contains("--package") || combined.contains("-p,"));
    assert!(combined.contains("--output") || combined.contains("-o,"));
}

#[test]
fn test_output_directory_creation() {
    let output_dir = PathBuf::from("target/doc-md-test-creation");

    fs::remove_dir_all(&output_dir).ok();
    assert!(!output_dir.exists());

    // Run with --no-deps (will document current crate only, should succeed)
    let _ = run_cargo_doc_md(&["-o", output_dir.to_str().unwrap(), "--no-deps"]);

    // Key assertion: output directory should be created
    assert!(output_dir.exists(), "Output directory should be created");

    fs::remove_dir_all(&output_dir).ok();
}

#[test]
fn test_cleanup_removes_stale_files() {
    let output_dir = PathBuf::from("target/doc-md-test-cleanup");

    fs::remove_dir_all(&output_dir).ok();

    let result = run_cargo_doc_md(&["-o", output_dir.to_str().unwrap(), "--no-deps"]);
    assert!(result.is_ok(), "First doc generation should succeed");

    let crate_dir = output_dir.join("cargo_doc_md");
    assert!(crate_dir.exists(), "Crate directory should be created");

    let sentinel_file = crate_dir.join("stale_module.md");
    fs::write(&sentinel_file, "This file should be removed on next run").unwrap();
    assert!(sentinel_file.exists(), "Sentinel file should exist");

    let result = run_cargo_doc_md(&["-o", output_dir.to_str().unwrap(), "--no-deps"]);
    assert!(result.is_ok(), "Second doc generation should succeed");

    assert!(
        !sentinel_file.exists(),
        "Sentinel file should be removed during cleanup"
    );
    assert!(
        crate_dir.exists(),
        "Crate directory should still exist with fresh docs"
    );

    fs::remove_dir_all(&output_dir).ok();
}

#[test]
fn test_package_flag_single() {
    // Test that -p flag is accepted (gracefully fails if package doesn't exist)
    let result = run_cargo_doc_md(&["-p", "nonexistent-crate-12345"]);
    // May succeed (exit 0) with failure message or error out
    match result {
        Err(err) => {
            // Should NOT contain flag validation errors
            assert!(!err.contains("Cannot use"));
        }
        Ok(output) => {
            // If it succeeded, output should mention the failure
            assert!(output.contains("Failed") || output.contains("failed"));
        }
    }
}

#[test]
fn test_package_flag_multiple() {
    // Test that multiple -p flags are accepted
    let result = run_cargo_doc_md(&["-p", "nonexistent-crate-1", "-p", "nonexistent-crate-2"]);
    // May succeed (exit 0) with failure messages or error out
    match result {
        Err(err) => {
            // Should NOT contain flag validation errors
            assert!(!err.contains("Cannot use"));
        }
        Ok(output) => {
            // If it succeeded, should report failures
            assert!(output.contains("Failed") || output.contains("Summary"));
        }
    }
}

#[test]
fn test_workspace_flag_in_single_crate() {
    // Test --workspace in a single-crate project (this project)
    let result = run_cargo_doc_md(&["--workspace", "--no-deps"]);
    // Should succeed - this is a single-member workspace
    // (or fail gracefully if not in workspace)
    if let Err(err) = result {
        // If it errors, should be about workspace detection, not flag validation
        assert!(
            err.contains("workspace") || err.contains("Workspace"),
            "Error should be about workspace detection: {}",
            err
        );
    }
}

#[test]
fn test_workspace_with_no_deps() {
    // Test that --workspace --no-deps combination is valid
    let result = run_cargo_doc_md(&["--workspace", "--no-deps"]);
    if let Err(err) = result {
        // Should NOT be a flag validation error
        assert!(!err.contains("Cannot use --workspace with"));
    }
}

#[test]
fn test_output_directory_validation_is_file() {
    use std::io::Write;

    let temp_file = std::env::temp_dir().join("cargo_doc_md_test_file.txt");
    let mut file = fs::File::create(&temp_file).unwrap();
    file.write_all(b"test").unwrap();
    drop(file);

    let result = run_cargo_doc_md(&["-o", temp_file.to_str().unwrap()]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("is a file, not a directory"),
        "Error should mention file vs directory: {}",
        err
    );

    fs::remove_file(&temp_file).ok();
}

#[test]
fn test_output_directory_validation_parent_missing() {
    let temp_dir = std::env::temp_dir();
    let nested_path = temp_dir
        .join("cargo_doc_md_test_parent_12345")
        .join("nested")
        .join("output");

    let result = run_cargo_doc_md(&[
        "-o",
        nested_path.to_str().unwrap(),
        "--json",
        "nonexistent.json",
    ]);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("JSON file not found"),
        "Should fail with JSON not found: {}",
        err
    );

    assert!(
        !nested_path.parent().unwrap().exists(),
        "Parent directory should not be created on early validation failure"
    );

    fs::remove_dir_all(temp_dir.join("cargo_doc_md_test_parent_12345")).ok();
}

#[test]
fn test_target_flag_host_triple() {
    // Pass the host triple to --target so the test runs anywhere without a
    // cross-compilation target installed. With --target set, rustdoc writes its
    // JSON under `target/<triple>/doc` rather than `target/doc`; documenting the
    // host triple confirms cargo-doc-md looks in the right place.
    let output_dir = PathBuf::from("target/doc-md-test-target");
    fs::remove_dir_all(&output_dir).ok();

    let host = host_triple();
    let result = run_cargo_doc_md(&[
        "--target",
        &host,
        "--no-deps",
        "-o",
        output_dir.to_str().unwrap(),
    ]);

    assert!(
        result.is_ok(),
        "Documenting for the host target should succeed: {:?}",
        result
    );
    assert!(
        output_dir.join("cargo_doc_md").exists(),
        "Crate documentation should be generated under the output directory"
    );

    fs::remove_dir_all(&output_dir).ok();
}
