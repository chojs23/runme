//! Execution helpers that transform parsed code blocks into runtime reports.

use std::path::Path;
use std::process::Command;
use std::time::Instant;

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
            duration_ms: 0,
            status: BlockStatus::Skipped,
            skip_reason: Some(reason),
            stdout: None,
            stderr: None,
        }
    }
}

/// Execute a parsed code block using a shell interpreter when possible.
pub fn execute(block: &CodeBlock, working_dir: &Path) -> Result<BlockReport> {
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

    let start = Instant::now();
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

        let (binary, rest) = args.split_first().expect("non-empty checked");
        let output = Command::new(binary)
            .args(rest)
            .current_dir(working_dir)
            .output()
            .with_context(|| format!("while executing {} line {}", block.id, idx + 1))?;

        let stdout_norm = normalize_stream(&output.stdout);
        if !stdout_norm.is_empty() {
            stdout_chunks.push(format!("$ {trimmed}\n{stdout_norm}"));
        }
        let stderr_norm = normalize_stream(&output.stderr);
        if !stderr_norm.is_empty() {
            stderr_chunks.push(format!("$ {trimmed}\n{stderr_norm}"));
        }

        if !output.status.success() {
            status = BlockStatus::Failed {
                exit_code: output.status.code(),
            };
            break;
        }
    }

    let duration = start.elapsed();

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
        duration_ms: duration.as_millis(),
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
