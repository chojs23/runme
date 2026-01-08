//! Execution helpers that transform parsed code blocks into runtime reports.
//!
//! The sandbox abstraction here keeps host execution readable while preparing
//! the ground for Docker/Wasmtime backends without touching CLI surfaces.

use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde::Serialize;

use crate::markdown::CodeBlock;
use shlex;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BlockStatus {
    Passed,
    Failed { exit_code: Option<i32> },
    Skipped,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockReport {
    pub id: String,
    pub headings: Vec<String>,
    pub language: Option<String>,
    pub sandbox: Option<String>,
    pub duration_ms: u128,
    pub status: BlockStatus,
    pub skip_reason: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<String>,
}

impl BlockReport {
    fn from_skip(block: &CodeBlock, reason: String) -> Self {
        Self {
            id: block.id.clone(),
            headings: block.headings.clone(),
            language: block.language.clone(),
            sandbox: None,
            duration_ms: 0,
            status: BlockStatus::Skipped,
            skip_reason: Some(reason),
            stdout: None,
            stderr: None,
        }
    }
}

/// Execution result returned by sandbox implementations.
#[derive(Clone, Debug)]
pub struct CommandOutcome {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    pub duration: Duration,
}

impl CommandOutcome {
    pub fn from_output(output: std::process::Output, duration: Duration) -> Self {
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
    fn run(&mut self, argv: &[String]) -> Result<CommandOutcome>;
}

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

/// Temporary docker sandbox shim that reuses host execution until container
/// orchestration lands. This preserves the CLI contract without silently
/// ignoring the user's requested isolation level.
pub struct DockerSandbox {
    host_fallback: HostSandbox,
}

impl DockerSandbox {
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        Self {
            host_fallback: HostSandbox::new(workdir),
        }
    }
}

impl Sandbox for DockerSandbox {
    fn label(&self) -> &str {
        "docker(host-fallback)"
    }

    fn run(&mut self, argv: &[String]) -> Result<CommandOutcome> {
        // TODO: replace with real docker invocation once image management lands.
        self.host_fallback.run(argv)
    }
}

/// Placeholder Wasm sandbox that executes commands on the host until
/// Wasmtime-based runners are ready. This keeps the API stable while we build
/// the actual runtime adapter.
pub struct WasmSandbox {
    host_fallback: HostSandbox,
}

impl WasmSandbox {
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        Self {
            host_fallback: HostSandbox::new(workdir),
        }
    }
}

impl Sandbox for WasmSandbox {
    fn label(&self) -> &str {
        "wasm(host-fallback)"
    }

    fn run(&mut self, argv: &[String]) -> Result<CommandOutcome> {
        // TODO: spin up Wasmtime modules to execute supported languages.
        self.host_fallback.run(argv)
    }
}

/// Execute a parsed code block using a shell interpreter when possible.
pub fn execute(block: &CodeBlock, sandbox: &mut dyn Sandbox) -> Result<BlockReport> {
    if let Some(reason) = block.skip_reason.clone() {
        return Ok(BlockReport::from_skip(block, reason));
    }

    if !block.is_shell() {
        return Ok(BlockReport::from_skip(
            block,
            format!(
                "Language '{}' unsupported yet; add a plugin",
                block.language.clone().unwrap_or_else(|| "shell".into())
            ),
        ));
    }

    if block.content.trim().is_empty() {
        return Ok(BlockReport::from_skip(
            block,
            "Block empty; nothing to execute".into(),
        ));
    }

    let sandbox_label = sandbox.label().to_string();
    let mut total_duration = Duration::default();
    let mut stdout_chunks = Vec::new();
    let mut stderr_chunks = Vec::new();
    let mut status = BlockStatus::Passed;
    let mut executed_lines = 0_usize;

    for (idx, raw_line) in block.content.lines().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let args =
            shlex::split(trimmed).ok_or_else(|| anyhow!("unable to parse line {}", idx + 1))?;
        if args.is_empty() {
            continue;
        }
        executed_lines += 1;

        let outcome = sandbox
            .run(&args)
            .with_context(|| format!("while executing {} line {}", block.id, idx + 1))?;

        if !outcome.stdout.is_empty() {
            stdout_chunks.push(format!("$ {trimmed}\n{}", outcome.stdout));
        }
        if !outcome.stderr.is_empty() {
            stderr_chunks.push(format!("$ {trimmed}\n{}", outcome.stderr));
        }
        total_duration += outcome.duration;

        if !outcome.success {
            status = BlockStatus::Failed {
                exit_code: outcome.exit_code,
            };
            break;
        }
    }

    if executed_lines == 0 {
        return Ok(BlockReport::from_skip(
            block,
            "Block only had comments/blank lines".into(),
        ));
    }

    Ok(BlockReport {
        id: block.id.clone(),
        headings: block.headings.clone(),
        language: block.language.clone(),
        sandbox: Some(sandbox_label),
        duration_ms: total_duration.as_millis(),
        status,
        skip_reason: None,
        stdout: (!stdout_chunks.is_empty()).then(|| stdout_chunks.join("\n")),
        stderr: (!stderr_chunks.is_empty()).then(|| stderr_chunks.join("\n")),
    })
}

fn normalize_stream(stream: &[u8]) -> String {
    let text = String::from_utf8_lossy(stream);
    text.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to craft a shell-ready block while keeping headings/context realistic.
    fn shell_block(script: &str) -> CodeBlock {
        CodeBlock {
            id: "block-test".into(),
            headings: vec!["Tests".into()],
            language: Some("bash".into()),
            content: script.trim().to_string(),
            skip_reason: None,
        }
    }

    fn host_sandbox() -> HostSandbox {
        HostSandbox::new(".")
    }

    #[test]
    fn respects_explicit_skip_metadata() {
        // Skipped blocks should never spawn processes, returning the stored reason verbatim.
        let mut block = shell_block("echo never runs");
        block.skip_reason = Some("user opted out".into());

        let mut sandbox = host_sandbox();
        let report =
            execute(&block, &mut sandbox).expect("skip handling should succeed without IO");

        assert!(matches!(report.status, BlockStatus::Skipped));
        assert_eq!(report.skip_reason.as_deref(), Some("user opted out"));
    }

    #[test]
    fn skips_unknown_languages() {
        // Unsupported languages should produce a skip report instead of running garbage commands.
        let block = CodeBlock {
            language: Some("python".into()),
            ..shell_block("print('hi')")
        };

        let mut sandbox = host_sandbox();
        let report =
            execute(&block, &mut sandbox).expect("unsupported languages still yield clean reports");

        assert!(matches!(report.status, BlockStatus::Skipped));
        assert!(
            report
                .skip_reason
                .as_deref()
                .unwrap()
                .contains("unsupported")
        );
    }

    #[test]
    fn records_successful_output() {
        // A simple echo proves we capture stdout and mark the block as passed.
        let block = shell_block("echo runner-ok");

        let mut sandbox = host_sandbox();
        let report = execute(&block, &mut sandbox).expect("echo should succeed on every platform");

        assert!(matches!(report.status, BlockStatus::Passed));
        let stdout = report.stdout.expect("stdout is captured");
        assert!(stdout.contains("runner-ok"));
        assert!(stdout.starts_with("$ echo"));
        assert_eq!(report.sandbox.as_deref(), Some("host"));
    }

    #[test]
    fn fails_fast_on_command_error() {
        // Once a command exits non-zero, we should surface the exit code and stop.
        let block = shell_block("false\necho never");

        let mut sandbox = host_sandbox();
        let report =
            execute(&block, &mut sandbox).expect("erroring commands still return a report");

        match report.status {
            BlockStatus::Failed { exit_code } => {
                assert_eq!(exit_code, Some(1));
            }
            other => panic!("unexpected status: {other:?}"),
        }
        assert!(
            report.stdout.is_none(),
            "execution stops before later lines"
        );
        assert_eq!(report.sandbox.as_deref(), Some("host"));
    }

    #[test]
    fn skips_blocks_with_only_comments() {
        // Comment-only snippets should be treated as documentation, not runnable commands.
        let block = shell_block("# this is documentation");

        let mut sandbox = host_sandbox();
        let report =
            execute(&block, &mut sandbox).expect("comment-only block is a valid skip case");

        assert!(matches!(report.status, BlockStatus::Skipped));
        assert_eq!(
            report.skip_reason.as_deref(),
            Some("Block only had comments/blank lines")
        );
    }
}
