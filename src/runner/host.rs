use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result, anyhow};

use super::sandbox::{CommandStatus, OutputSink, Sandbox, spawn_with_streaming};

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

    fn run(&mut self, argv: &[String], sink: &mut dyn OutputSink) -> Result<CommandStatus> {
        let (binary, rest) = argv
            .split_first()
            .ok_or_else(|| anyhow!("sandbox run requires at least one argument"))?;

        let mut cmd = Command::new(binary);
        cmd.args(rest).current_dir(&self.workdir);

        let start = Instant::now();
        let output = spawn_with_streaming(cmd, sink)
            .with_context(|| format!("while invoking {binary} inside host sandbox"))?;
        Ok(output.with_duration(start.elapsed()))
    }
}
