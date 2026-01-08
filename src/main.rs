mod markdown;
mod runner;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use markdown::CodeBlock;
use runner::BlockReport;

/// `rumne` keeps README snippets honest by parsing markdown and
/// executing runnable blocks inside small sandboxes (shell-only for now).
#[derive(Parser, Debug)]
#[command(
    name = "rumne",
    version,
    about = "Execute README code blocks on demand"
)]
struct Cli {
    /// Path to the primary README markdown file.
    #[arg(default_value = "README.md")]
    target: PathBuf,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List discovered blocks with metadata but do not execute them.
    List,
    /// Execute runnable blocks, optionally targeting a subset.
    Run {
        /// Single block identifier (e.g. block-002) to execute.
        #[arg(long)]
        block: Option<String>,
        /// Output format for reports.
        #[arg(long, default_value_t = ReportFormat::Human, value_enum)]
        format: ReportFormat,
    },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ReportFormat {
    Human,
    Json,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let markdown = fs::read_to_string(&cli.target)
        .with_context(|| format!("while reading {}", cli.target.display()))?;
    let blocks = markdown::extract_blocks(&markdown)?;

    let workdir = cli
        .target
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    match cli.command.unwrap_or_else(|| Command::Run {
        block: None,
        format: ReportFormat::Human,
    }) {
        Command::List => render_list(&blocks),
        Command::Run { block, format } => run_blocks(&blocks, &workdir, block.as_deref(), format)?,
    }

    Ok(())
}

fn render_list(blocks: &[CodeBlock]) {
    println!("Discovered {} block(s):", blocks.len());
    for block in blocks {
        let label = block.language.clone().unwrap_or_else(|| "shell".into());
        let headings = if block.headings.is_empty() {
            "(root)".to_string()
        } else {
            block.headings.join(" › ")
        };
        let skip_hint = block
            .skip_reason
            .as_ref()
            .map(|reason| format!(" (skip: {reason})"))
            .unwrap_or_default();
        println!("- {} [{}] {headings}{skip_hint}", block.id, label);
    }
}

fn run_blocks(
    blocks: &[CodeBlock],
    workdir: &Path,
    filter: Option<&str>,
    format: ReportFormat,
) -> Result<()> {
    let subset: Vec<&CodeBlock> = match filter {
        Some(id) => {
            let block = blocks
                .iter()
                .find(|block| block.id == id)
                .with_context(|| format!("unknown block id {id}"))?;
            vec![block]
        }
        None => blocks.iter().collect(),
    };

    let mut reports = Vec::new();
    for block in subset {
        let report = runner::execute(block, workdir)
            .with_context(|| format!("while running {}", block.id))?;
        reports.push(report);
    }

    match format {
        ReportFormat::Human => {
            for report in &reports {
                print_human_report(report);
            }
        }
        ReportFormat::Json => {
            let json = serde_json::to_string_pretty(&reports)?;
            println!("{json}");
        }
    }

    Ok(())
}

fn print_human_report(report: &BlockReport) {
    println!("\n== {} ==", report.id);
    if let Some(lang) = &report.language {
        println!("language: {lang}");
    }
    if !report.headings.is_empty() {
        println!("context: {}", report.headings.join(" › "));
    }
    println!("status: {:?}", report.status);
    if let Some(reason) = &report.skip_reason {
        println!("skip reason: {reason}");
    }
    if let Some(stdout) = &report.stdout {
        if !stdout.is_empty() {
            println!("stdout:\n{stdout}");
        }
    }
    if let Some(stderr) = &report.stderr {
        if !stderr.is_empty() {
            println!("stderr:\n{stderr}");
        }
    }
}
