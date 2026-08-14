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

/// Per-file output is printed whole and atomically — under a parallel run (M4)
/// line-by-line printing from eight workers would be unreadable.
pub fn print_file(r: &FileResult) {
    let failed = r.failed();
    let mark = if failed == 0 { "✓" } else { "✗" };
    println!(
        "  {mark} {} — scenarios: {}",
        r.path.display(),
        r.scenarios.len()
    );
    for s in &r.scenarios {
        if let Some(f) = &s.failure {
            println!("\nFAIL  {}:{} › {}", r.path.display(), s.line, s.name);
            println!("{f}");
        }
    }
}

pub fn print_summary(results: &[FileResult], run_id: &str) -> i32 {
    let files = results.len();
    let scenarios: usize = results.iter().map(|r| r.scenarios.len()).sum();
    let failed: usize = results.iter().map(FileResult::failed).sum();
    println!("\nrun {run_id}");
    println!("files: {files}, scenarios: {scenarios}, failed: {failed}");
    if failed == 0 { 0 } else { 1 }
}
