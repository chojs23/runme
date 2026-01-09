mod docker;
mod host;
pub mod sandbox;
mod wasm;

pub use docker::DockerSandbox;
pub use host::HostSandbox;
pub use sandbox::Sandbox;
pub use wasm::WasmSandbox;

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use shlex;

use crate::markdown::CodeBlock;
use sandbox::OutputSink;

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
    pub name: Option<String>,
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
            name: block.name.clone(),
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

/// Execute a parsed code block using a shell interpreter when possible.
pub fn execute(
    block: &CodeBlock,
    sandbox: &mut dyn Sandbox,
    stream_live: bool,
) -> Result<BlockReport> {
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

        let mut transcript = CommandTranscript::new(trimmed);
        let mut sink = TranscriptSink::new(&mut transcript, stream_live.then_some(block));
        let outcome = sandbox
            .run(&args, &mut sink)
            .with_context(|| format!("while executing {} line {}", block.id, idx + 1))?;

        if let Some(line_stdout) = transcript.stdout {
            stdout_chunks.push(line_stdout);
        }
        if let Some(line_stderr) = transcript.stderr {
            stderr_chunks.push(line_stderr);
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
        name: block.name.clone(),
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

struct CommandTranscript<'a> {
    command: &'a str,
    stdout: Option<String>,
    stderr: Option<String>,
}

impl<'a> CommandTranscript<'a> {
    fn new(command: &'a str) -> Self {
        Self {
            command,
            stdout: None,
            stderr: None,
        }
    }

    fn append_stdout(&mut self, chunk: &str) -> bool {
        if chunk.is_empty() {
            return false;
        }
        let first = self.stdout.is_none();
        let entry = self
            .stdout
            .get_or_insert_with(|| format!("$ {}\n", self.command));
        entry.push_str(chunk);
        entry.push('\n');
        first
    }

    fn append_stderr(&mut self, chunk: &str) -> bool {
        if chunk.is_empty() {
            return false;
        }
        let first = self.stderr.is_none();
        let entry = self
            .stderr
            .get_or_insert_with(|| format!("$ {}\n", self.command));
        entry.push_str(chunk);
        entry.push('\n');
        first
    }
}

struct TranscriptSink<'a, 'b> {
    transcript: &'a mut CommandTranscript<'b>,
    streamer: Option<HumanStreamer>,
}

impl<'a, 'b> TranscriptSink<'a, 'b> {
    fn new(transcript: &'a mut CommandTranscript<'b>, block: Option<&'b CodeBlock>) -> Self {
        Self {
            transcript,
            streamer: block.map(HumanStreamer::new),
        }
    }
}

impl OutputSink for TranscriptSink<'_, '_> {
    fn on_stdout(&mut self, chunk: &str) {
        let first = self.transcript.append_stdout(chunk);
        if let Some(streamer) = self.streamer.as_mut() {
            streamer.on_stdout(self.transcript.command, chunk, first);
        }
    }

    fn on_stderr(&mut self, chunk: &str) {
        let first = self.transcript.append_stderr(chunk);
        if let Some(streamer) = self.streamer.as_mut() {
            streamer.on_stderr(self.transcript.command, chunk, first);
        }
    }
}

struct HumanStreamer {
    label: String,
}

impl HumanStreamer {
    fn new(block: &CodeBlock) -> Self {
        let label = if let Some(name) = &block.name {
            format!("{} ({})", block.id, name)
        } else {
            block.id.clone()
        };
        Self { label }
    }

    fn on_stdout(&mut self, command: &str, chunk: &str, first: bool) {
        if first {
            println!("\x1b[36m[{}]\x1b[0m $ {}", self.label, command);
        }
        println!("{chunk}");
    }

    fn on_stderr(&mut self, command: &str, chunk: &str, first: bool) {
        if first {
            eprintln!("\x1b[31m[{}]\x1b[0m $ {} (stderr)", self.label, command);
        }
        eprintln!("\x1b[31m{chunk}\x1b[0m");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// Helper to craft a shell-ready block while keeping headings/context realistic.
    fn shell_block(script: &str) -> CodeBlock {
        CodeBlock {
            id: "block-test".into(),
            name: None,
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
            execute(&block, &mut sandbox, false).expect("skip handling should succeed without IO");

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
        let report = execute(&block, &mut sandbox, false)
            .expect("unsupported languages still yield clean reports");

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
        let report =
            execute(&block, &mut sandbox, false).expect("echo should succeed on every platform");

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
            execute(&block, &mut sandbox, false).expect("erroring commands still return a report");

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
            execute(&block, &mut sandbox, false).expect("comment-only block is a valid skip case");

        assert!(matches!(report.status, BlockStatus::Skipped));
        assert_eq!(
            report.skip_reason.as_deref(),
            Some("Block only had comments/blank lines")
        );
    }

    #[test]
    fn docker_sandbox_prefers_cli_image_over_env() {
        const KEY: &str = "RUNME_DOCKER_IMAGE";
        let previous = env::var(KEY).ok();
        unsafe {
            env::set_var(KEY, "env:image");
        }
        let from_env = DockerSandbox::new(".", None, Vec::new());
        assert_eq!(from_env.image(), "env:image");

        let from_cli = DockerSandbox::new(".", Some("cli:image".into()), vec!["--cpus=1".into()]);
        assert_eq!(from_cli.image(), "cli:image");
        assert_eq!(from_cli.extra_args(), ["--cpus=1"]);

        match previous {
            Some(val) => unsafe { env::set_var(KEY, val) },
            None => unsafe { env::remove_var(KEY) },
        }
    }
}
