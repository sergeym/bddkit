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

/// `resources.api` may be absent entirely — legal for a scenario that
/// makes no HTTP requests (symmetric with `resources.db`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_config_without_any_api_resource_runs_a_non_http_scenario() {
    let dir = std::env::temp_dir().join("bddkit-no-api-ok-test");
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/vars.feature"),
        "Feature: f\n  Scenario: s\n    \
         Given set variable \"x\" to \"1\"\n    Then variable \"x\" should be equal to \"1\"\n",
    )
    .expect("write feature");
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "paths: [{}]\nresources:\n  api: {{}}\n",
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

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "a config without a single API resource must run if the scenario never touches HTTP\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
}

/// If no APIs are declared but the scenario sends a request anyway — the
/// failure happens as an ordinary step failure on first use, not a panic
/// and not a startup rejection.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_config_without_any_api_resource_fails_at_first_http_step() {
    let dir = std::env::temp_dir().join("bddkit-no-api-fail-test");
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/http.feature"),
        "Feature: f\n  Scenario: s\n    \
         When I request \"/ping\"\n    Then the response code is 200\n",
    )
    .expect("write feature");
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "paths: [{}]\nresources:\n  api: {{}}\n",
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

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "the scenario must fail\n{stdout}"
    );
    assert!(stdout.contains("resources.api"), "{stdout}");
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
            dir.join("features")
                .display()
                .to_string()
                .replace('\\', "/")
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
        .args([
            "--config",
            cfg.to_str().expect("path is UTF-8"),
            "--tag",
            "smoke",
        ])
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
        .args([
            "--config",
            cfg.to_str().expect("path is UTF-8"),
            "--tag",
            "absent",
        ])
        .output()
        .expect("failed to launch bddkit");

    assert_eq!(
        out.status.code(),
        Some(2),
        "empty selection — not a green run"
    );
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

    assert_eq!(
        out.status.code(),
        Some(1),
        "the scenario must fail, not fail validation"
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("is not supported"),
        "expected a message about the unsupported content-type:\n{stdout}"
    );
}

/// Prepares two feature files that write the same variable with different
/// values BEFORE the barrier and check it AFTER. The barrier guarantees both
/// files have written before either reads: a `VarStack` shared across the run
/// would fail this deterministically, not just sometimes.
fn write_parallel_fixture(
    dir: &std::path::Path,
    base: &str,
    concurrency: usize,
    tags: &str,
) -> std::path::PathBuf {
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    for who in ["a", "b"] {
        std::fs::write(
            dir.join(format!("features/{who}.feature")),
            format!(
                "{tags}Feature: parallel {who}\n  Scenario: both tasks meet at the barrier\n    \
                 Given set variable \"who\" to \"{who}\"\n    \
                 When I request \"/barrier\"\n    \
                 Then the response code is 200\n    \
                 And variable \"who\" should be equal to \"{who}\"\n"
            ),
        )
        .expect("write feature");
    }
    let cfg = dir.join("cfg.yaml");
    std::fs::write(
        &cfg,
        format!(
            "concurrency: {concurrency}\npaths: [{}]\nresources:\n  api:\n    stub:\n      \
             base_url: {base}\n      timeout_secs: 2\n",
            dir.join("features")
                .display()
                .to_string()
                .replace('\\', "/")
        ),
    )
    .expect("write config");
    cfg
}

/// M5 gate: two files must run at the same time, or the barrier never opens.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_feature_files_run_at_the_same_time() {
    let base = common::spawn_barrier(2).await;
    let dir = std::env::temp_dir().join(format!("bddkit-parallel-{}", std::process::id()));
    let cfg = write_parallel_fixture(&dir, &base, 2, "");

    let out = Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args(["--config", cfg.to_str().expect("path is UTF-8")])
        .output()
        .expect("failed to launch bddkit");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "both files must run in parallel\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    assert!(stdout.contains("failed: 0"), "{stdout}");
    assert!(stdout.contains("files: 2"), "{stdout}");
}

/// Negative control: with `concurrency: 1` the barrier never opens and the run
/// fails. Without this test, the previous one could pass green for any reason
/// other than actual parallelism.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_single_worker_cannot_release_the_barrier() {
    let base = common::spawn_barrier(2).await;
    let dir = std::env::temp_dir().join(format!("bddkit-sequential-{}", std::process::id()));
    let cfg = write_parallel_fixture(&dir, &base, 1, "");

    let out = Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args(["--config", cfg.to_str().expect("path is UTF-8")])
        .output()
        .expect("failed to launch bddkit");

    assert_eq!(
        out.status.code(),
        Some(1),
        "a single worker must hit the request timeout, not pass"
    );
}

/// Files in the same chain must never be in flight at the same time —
/// the two-party barrier never opens, and the run must fail on timeout. This
/// mirrors the parallelism test: the same pair of files, just tagged.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_files_in_one_serial_chain_never_run_together() {
    let base = common::spawn_barrier(2).await;
    let dir = std::env::temp_dir().join(format!("bddkit-serial-{}", std::process::id()));
    let cfg = write_parallel_fixture(&dir, &base, 2, "@serial(shared)\n");

    let out = Command::new(env!("CARGO_BIN_EXE_bddkit"))
        .args(["--config", cfg.to_str().expect("path is UTF-8")])
        .output()
        .expect("failed to launch bddkit");

    assert_eq!(
        out.status.code(),
        Some(1),
        "the chain must serialize the files, the two-party barrier must not open\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A broken scheduling tag — fails before the first request, code 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_file_in_two_chains_fails_the_startup() {
    let dir = std::env::temp_dir().join(format!("bddkit-two-chains-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/bad.feature"),
        "@serial(a)\nFeature: f\n  @serial(b)\n  Scenario: s\n    Given set variable \"x\" to \"1\"\n",
    )
    .expect("write feature");
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "paths: [{}]\nresources:\n  api: {{}}\n",
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
    assert!(stderr.contains("two chains"), "{stderr}");
}

/// An unreachable DB — fails BEFORE the first request, so code 2, not 1.
/// Invariant 6: 1 is reserved for a failed scenario.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_database_fails_the_startup_with_code_two() {
    let dir = std::env::temp_dir().join(format!("bddkit-startup-exit-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/vars.feature"),
        "Feature: f\n  Scenario: s\n    Given set variable \"x\" to \"1\"\n",
    )
    .expect("write feature");
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "paths: [{}]\nresources:\n  api: {{}}\n  db:\n    main:\n      \
             dsn: postgres://nobody:nobody@127.0.0.1:1/nothing\n",
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

    assert_eq!(
        out.status.code(),
        Some(2),
        "a failure before the first request must give 2\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("run not started"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// With a single worker, print order is queue order.
/// This checks that `@priority` sets it: higher runs first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn higher_priority_files_run_first() {
    let dir = std::env::temp_dir().join(format!("bddkit-priority-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    // Names are deliberately in alphabetical order, the reverse of what we want:
    // without the tag the queue would be low → mid → high.
    for (name, tag) in [
        ("a_low", "@priority(-1)\n"),
        ("b_mid", ""),
        ("c_high", "@priority(5)\n"),
    ] {
        std::fs::write(
            dir.join(format!("features/{name}.feature")),
            format!("{tag}Feature: {name}\n  Scenario: s\n    Given set variable \"x\" to \"1\"\n"),
        )
        .expect("write feature");
    }
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "concurrency: 1\npaths: [{}]\nresources:\n  api: {{}}\n",
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

    let stdout = String::from_utf8_lossy(&out.stdout);
    let high = stdout.find("c_high").expect("file c_high in the output");
    let mid = stdout.find("b_mid").expect("file b_mid in the output");
    let low = stdout.find("a_low").expect("file a_low in the output");
    assert!(
        high < mid && mid < low,
        "priority queue order:\n{stdout}"
    );
}

/// A non-numeric priority — fails before the first request, code 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_numeric_priority_fails_the_startup() {
    let dir = std::env::temp_dir().join(format!("bddkit-bad-priority-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/bad.feature"),
        "@priority(urgent)\nFeature: f\n  Scenario: s\n    Given set variable \"x\" to \"1\"\n",
    )
    .expect("write feature");
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "paths: [{}]\nresources:\n  api: {{}}\n",
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
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("an integer"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `--fail-fast` stops handing out NEW work after the first failure.
/// With a single worker this is deterministic: the first file fails, the
/// second and third never start at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fail_fast_stops_starting_new_files() {
    let dir = std::env::temp_dir().join(format!("bddkit-fail-fast-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/a_broken.feature"),
        "Feature: broken\n  Scenario: s\n    \
         Given set variable \"x\" to \"1\"\n    Then variable \"x\" should be equal to \"2\"\n",
    )
    .expect("write feature");
    for name in ["b_ok", "c_ok"] {
        std::fs::write(
            dir.join(format!("features/{name}.feature")),
            format!("Feature: {name}\n  Scenario: s\n    Given set variable \"x\" to \"1\"\n"),
        )
        .expect("write feature");
    }
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "concurrency: 1\npaths: [{}]\nresources:\n  api: {{}}\n",
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
            "--fail-fast",
        ])
        .output()
        .expect("failed to launch bddkit");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("a_broken"),
        "the failed file must be in the report:\n{stdout}"
    );
    assert!(
        !stdout.contains("c_ok"),
        "no new files must start after the failure:\n{stdout}"
    );
    assert!(stdout.contains("files: 1"), "{stdout}");
}

/// `--fail-fast` also stops handing out new work WITHIN a chain already
/// taken: three files sharing one `@serial` name run strictly in order on one
/// worker, the first fails, the second and third never start at all. This is
/// a separate check from `fail_fast_stops_starting_new_files`, which only
/// exercises the "before the next chain" check — there each file was its own chain.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fail_fast_stops_a_chain_partway_through() {
    let dir = std::env::temp_dir().join(format!("bddkit-fail-fast-chain-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/a_broken.feature"),
        "@serial(chain)\nFeature: broken\n  Scenario: s\n    \
         Given set variable \"x\" to \"1\"\n    Then variable \"x\" should be equal to \"2\"\n",
    )
    .expect("write feature");
    for name in ["b_ok", "c_ok"] {
        std::fs::write(
            dir.join(format!("features/{name}.feature")),
            format!(
                "@serial(chain)\nFeature: {name}\n  Scenario: s\n    Given set variable \"x\" to \"1\"\n"
            ),
        )
        .expect("write feature");
    }
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "concurrency: 1\npaths: [{}]\nresources:\n  api: {{}}\n",
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
            "--fail-fast",
        ])
        .output()
        .expect("failed to launch bddkit");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("a_broken"),
        "the failed file must be in the report:\n{stdout}"
    );
    assert!(
        !stdout.contains("b_ok") && !stdout.contains("c_ok"),
        "the rest of the chain must not start after the failure:\n{stdout}"
    );
    assert!(stdout.contains("files: 1"), "{stdout}");
}

/// Without the flag, one file failing does not block the rest: the run must
/// reach the end and show all failures at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_fail_fast_every_file_still_runs() {
    let dir = std::env::temp_dir().join(format!("bddkit-no-fail-fast-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("features")).expect("mkdir");
    std::fs::write(
        dir.join("features/a_broken.feature"),
        "Feature: broken\n  Scenario: s\n    \
         Given set variable \"x\" to \"1\"\n    Then variable \"x\" should be equal to \"2\"\n",
    )
    .expect("write feature");
    std::fs::write(
        dir.join("features/c_ok.feature"),
        "Feature: whole\n  Scenario: s\n    Given set variable \"x\" to \"1\"\n",
    )
    .expect("write feature");
    std::fs::write(
        dir.join("cfg.yaml"),
        format!(
            "concurrency: 1\npaths: [{}]\nresources:\n  api: {{}}\n",
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

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{stdout}");
    assert!(stdout.contains("files: 2"), "{stdout}");
}
