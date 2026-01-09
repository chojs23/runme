use std::process::Output;
use std::time::Duration;

use serde::Serialize;

/// Execution result returned by sandbox implementations.
#[derive(Clone, Debug, Serialize)]
pub struct CommandOutcome {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub duration: Duration,
}

impl CommandOutcome {
    pub fn from_output(output: Output, duration: Duration) -> Self {
        Self {
            stdout: normalize_stream(&output.stdout),
            stderr: normalize_stream(&output.stderr),
            exit_code: output.status.code(),
            success: output.status.success(),
            duration,
        }
    }
}

/// Trait implemented by every sandbox backend (host, Docker, Wasm, ...).
pub trait Sandbox {
    /// Short label surfaced in reports, e.g. `host` or `docker:ubuntu-22.04`.
    fn label(&self) -> &str;
    /// Run a parsed argv vector inside the sandbox environment.
    fn run(&mut self, argv: &[String]) -> anyhow::Result<CommandOutcome>;
}

fn normalize_stream(stream: &[u8]) -> String {
    let text = String::from_utf8_lossy(stream);
    text.trim().to_string()
}
