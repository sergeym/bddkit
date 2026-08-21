mod common;

use std::process::Command;

/// Gate 1: scenarios against the reference stub must be green.
// Multi-thread: the stub runs inside `tokio::spawn`, and the test blocks on
// `Command::output()`. On a single-threaded runtime, blocking prevents polling
// the server task — the port is bound, but connections are not accepted, and requests hang
// until timeout. A separate worker thread fixes this.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn all_feature_files_pass_against_the_stub() {
    let base = common::spawn().await;

    let exe = env!("CARGO_BIN_EXE_bddkit");
    let out = Command::new(exe)
        .args(["--config", "tests/acceptance.yaml"])
        .env("BDDKIT_STUB_URL", &base)
        .output()
        .expect("failed to launch bddkit");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the run must be green\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    assert!(stdout.contains("failed: 0"), "{stdout}");
}

/// An unknown step must fail BEFORE the first request, with exit code 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_step_fails_before_running() {
    let base = common::spawn().await;
    let dir = std::env::temp_dir().join("bddkit-validate-test");
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/bad.feature"),
        "Feature: f\n  Scenario: s\n    When I refund the order\n",
    )
    .expect("write feature");
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "paths: [{}]\nresources:\n  api:\n    stub:\n      base_url: {base}\n",
            dir.join("features")
                .display()
                .to_string()
                .replace('\\', "/")
        ),
    )
    .expect("write config");

    let exe = env!("CARGO_BIN_EXE_bddkit");
    let out = Command::new(exe)
        .args([
            "--config",
            dir.join("cfg.yaml").to_str().expect("path is UTF-8"),
        ])
        .output()
        .expect("failed to launch bddkit");

    assert_eq!(out.status.code(), Some(2), "exit code of the static check");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("run not started"), "{stderr}");
    assert!(stderr.contains("I refund the order"), "{stderr}");
}

/// Shared helper: writes two tagged feature files and a stub config to a
/// temp directory, returns the config path.
fn write_tagged_project(base: &str, name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("bddkit-{name}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/smoke.feature"),
        "Feature: smoke\n  @smoke\n  Scenario: ping\n    When I request \"/ping\"\n    Then the response code is 200\n",
    )
    .expect("write smoke feature");
    std::fs::write(
        dir.join("features/slow.feature"),
        "Feature: slow\n  @slow\n  Scenario: ping slowly\n    When I request \"/ping\"\n    Then the response code is 200\n",
    )
    .expect("write slow feature");
    let cfg = dir.join("cfg.yaml");
    std::fs::write(
        &cfg,
        format!(
            "paths: [{}]\nresources:\n  api:\n    stub:\n      base_url: {base}\n",
            dir.join("features").display().to_string().replace('\\', "/")
        ),
    )
    .expect("write config");
    cfg
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tag_filter_runs_only_the_matching_scenarios() {
    let base = common::spawn().await;
    let cfg = write_tagged_project(&base, "tagfilter");

    let out = Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args(["--config", cfg.to_str().expect("path is UTF-8"), "--tag", "smoke"])
        .output()
        .expect("failed to launch bddkit");

    // Scenario names are only printed on failure, so selection is visible via
    // file names and the final counters.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("smoke.feature"),
        "the selected scenario must run:\n{stdout}"
    );
    assert!(
        !stdout.contains("slow.feature"),
        "the untagged scenario must not run:\n{stdout}"
    );
    assert!(
        stdout.contains("files: 1, scenarios: 1, failed: 0"),
        "exactly one scenario must pass:\n{stdout}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tag_matching_nothing_fails_with_exit_code_two() {
    let base = common::spawn().await;
    let cfg = write_tagged_project(&base, "tagempty");

    let out = Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args(["--config", cfg.to_str().expect("path is UTF-8"), "--tag", "absent"])
        .output()
        .expect("failed to launch bddkit");

    assert_eq!(out.status.code(), Some(2), "empty selection — not a green run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no scenario selected"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_positional_path_overrides_the_config_paths() {
    let base = common::spawn().await;
    let cfg = write_tagged_project(&base, "positional");
    let only = cfg
        .parent()
        .expect("parent directory")
        .join("features/smoke.feature");

    let out = Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args([
            "--config",
            cfg.to_str().expect("path is UTF-8"),
            only.to_str().expect("path is UTF-8"),
        ])
        .output()
        .expect("failed to launch bddkit");

    // The counter is required: without it the assertion would pass even if
    // the positional path were completely ignored (`paths` from the config yield two files).
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("slow.feature"),
        "a file outside the given path must not run:\n{stdout}"
    );
    assert!(
        stdout.contains("files: 1, scenarios: 1, failed: 0"),
        "exactly one file from the given path must pass:\n{stdout}"
    );
}

/// M4 gate: one scenario talks to two different APIs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_scenario_can_call_two_different_apis() {
    let primary = common::spawn().await;
    let secondary = common::spawn_secondary().await;

    let dir = std::env::temp_dir().join(format!("bddkit-two-apis-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/switch.feature"),
        r#"Feature: switching between APIs
  Scenario: the request goes to the selected API, the previous response survives the switch
    When I request "/ping"
    Then the response body contains JSON:
      """
      {"version": 3}
      """
    When I use "secondary" api
    Then the response body contains JSON:
      """
      {"version": 3}
      """
    When I request "/ping"
    Then the response body contains JSON:
      """
      {"source": "secondary"}
      """
"#,
    )
    .expect("write feature");
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "paths: [{}]\ndefault_api: primary\nresources:\n  api:\n    primary:\n      base_url: {primary}\n    secondary:\n      base_url: {secondary}\n",
            dir.join("features").display().to_string().replace('\\', "/")
        ),
    )
    .expect("write config");

    let out = Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args([
            "--config",
            dir.join("cfg.yaml").to_str().expect("path is UTF-8"),
        ])
        .output()
        .expect("failed to launch bddkit");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "the two-API scenario must be green\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    // Scenario names are only printed on failure, so the counters are the
    // only proof the gate actually ran something.
    assert!(
        stdout.contains("files: 1, scenarios: 1, failed: 0"),
        "exactly one scenario must pass:\n{stdout}"
    );
}

#[test]
fn macro_cycle_fails_validation_with_exit_code_two() {
    let dir = std::env::temp_dir().join(format!("bddkit-cycle-test-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/cycle.feature"),
        "Feature: f\n  Scenario: s\n    When I do first\n",
    )
    .expect("write feature");
    std::fs::write(
        dir.join("macros.yaml"),
        "- step: I do first\n  do: [I do second]\n- step: I do second\n  do: [I do first]\n",
    )
    .expect("write macros");
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "macro_paths: [{}]\npaths: [{}]\nresources:\n  api:\n    stub:\n      base_url: http://example.test\n",
            dir.join("macros.yaml")
                .display()
                .to_string()
                .replace('\\', "/"),
            dir.join("features")
                .display()
                .to_string()
                .replace('\\', "/")
        ),
    )
    .expect("write config");

    let out = Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args([
            "--config",
            dir.join("cfg.yaml").to_str().expect("path is UTF-8"),
        ])
        .output()
        .expect("failed to launch bddkit");

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("cycle in macros"), "{stderr}");
    assert!(stderr.contains("run not started"), "{stderr}");
}

/// `Print response body as "<path>"` cannot work without structure: for
/// text/plain this is an explicit error, not silent degradation.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn print_body_as_path_fails_for_plain_content_type() {
    let base = common::spawn().await;
    let dir = std::env::temp_dir().join(format!("bddkit-debug-plain-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/plain.feature"),
        "Feature: f\n  Scenario: s\n    When I request \"/plain\"\n    Then Print response body as \"x\"\n",
    )
    .expect("write feature");
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "paths: [{}]\nresources:\n  api:\n    stub:\n      base_url: {base}\n",
            dir.join("features").display().to_string().replace('\\', "/")
        ),
    )
    .expect("write config");

    let out = Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args([
            "--config",
            dir.join("cfg.yaml").to_str().expect("path is UTF-8"),
        ])
        .output()
        .expect("failed to launch bddkit");

    assert_eq!(out.status.code(), Some(1), "the scenario must fail, not fail validation");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("is not supported"),
        "expected a message about the unsupported content-type:\n{stdout}"
    );
}
