use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};

use super::sandbox::{CommandOutcome, Sandbox};

/// Straightforward sandbox that shells out on the host OS.
///
/// Keeping this minimal allows future containerized sandboxes to plug in
/// without altering block orchestration.
pub struct HostSandbox {
    workdir: PathBuf,
}

impl HostSandbox {
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        Self {
            workdir: workdir.into(),
        }
    }
}

impl Sandbox for HostSandbox {
    fn label(&self) -> &str {
        "host"
    }

    fn run(&mut self, argv: &[String]) -> Result<CommandOutcome> {
        let (binary, rest) = argv
            .split_first()
            .ok_or_else(|| anyhow!("sandbox run requires at least one argument"))?;

        let start = Instant::now();
        let output = Command::new(binary)
            .args(rest)
            .current_dir(&self.workdir)
            .output()
            .with_context(|| format!("while invoking {binary} inside host sandbox"))?;
        let duration = start.elapsed();

        Ok(CommandOutcome::from_output(output, duration))
    }
}
