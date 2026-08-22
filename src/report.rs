use std::path::PathBuf;

#[derive(Debug)]
pub struct ScenarioResult {
    pub name: String,
    pub line: usize,
    pub failure: Option<String>,
}

#[derive(Debug)]
pub struct FileResult {
    pub path: PathBuf,
    pub scenarios: Vec<ScenarioResult>,
}

impl FileResult {
    pub fn failed(&self) -> usize {
        self.scenarios
            .iter()
            .filter(|s| s.failure.is_some())
            .count()
    }
}

/// Collects a whole file's output into one string. The caller prints it with one
/// `print!`: under a parallel run, line-by-line printing from eight workers
/// gets interleaved right in the middle of a failed request's dump.
pub fn render_file(r: &FileResult) -> String {
    let mark = if r.failed() == 0 { "✓" } else { "✗" };
    let mut out = format!(
        "  {mark} {} — scenarios: {}\n",
        r.path.display(),
        r.scenarios.len()
    );
    for s in &r.scenarios {
        if let Some(f) = &s.failure {
            out.push_str(&format!(
                "\nFAIL  {}:{} › {}\n{f}\n",
                r.path.display(),
                s.line,
                s.name
            ));
        }
    }
    out
}

pub fn print_summary(results: &[FileResult], run_id: &str) -> i32 {
    let files = results.len();
    let scenarios: usize = results.iter().map(|r| r.scenarios.len()).sum();
    let failed: usize = results.iter().map(FileResult::failed).sum();
    println!("\nrun {run_id}");
    println!("files: {files}, scenarios: {scenarios}, failed: {failed}");
    if failed == 0 { 0 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(failure: Option<&str>) -> FileResult {
        FileResult {
            path: PathBuf::from("features/auth.feature"),
            scenarios: vec![ScenarioResult {
                name: "user login".to_string(),
                line: 34,
                failure: failure.map(str::to_string),
            }],
        }
    }

    #[test]
    fn a_passing_file_renders_a_single_summary_line() {
        let out = render_file(&result(None));
        assert_eq!(
            out.lines().count(),
            1,
            "extra lines break the atomicity of the output: {out:?}"
        );
        assert!(out.contains("✓"), "{out}");
        assert!(out.contains("features/auth.feature"), "{out}");
    }

    #[test]
    fn a_failing_file_renders_the_mark_scenario_and_dump_in_one_string() {
        // Everything about the file must come back as ONE string:
        // piecemeal printing interleaves output when concurrency > 1.
        let out = render_file(&result(Some("  the response code is 200\nexpected: 200")));
        assert!(out.contains("✗"), "{out}");
        assert!(out.contains("FAIL"), "{out}");
        assert!(out.contains("features/auth.feature:34"), "{out}");
        assert!(out.contains("user login"), "{out}");
        assert!(out.contains("expected: 200"), "{out}");
    }

    #[test]
    fn every_failed_scenario_of_a_file_appears_in_the_same_string() {
        let r = FileResult {
            path: PathBuf::from("f.feature"),
            scenarios: vec![
                ScenarioResult {
                    name: "first".into(),
                    line: 3,
                    failure: Some("reason A".into()),
                },
                ScenarioResult {
                    name: "second".into(),
                    line: 9,
                    failure: Some("reason B".into()),
                },
            ],
        };
        let out = render_file(&r);
        assert!(
            out.contains("reason A") && out.contains("reason B"),
            "{out}"
        );
    }
}
