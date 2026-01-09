mod markdown;
mod runner;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{ArgAction, Args, Parser, Subcommand, ValueEnum};
use markdown::CodeBlock;
use runner::{BlockReport, DockerSandbox, HostSandbox, Sandbox, WasmSandbox};

/// `runme` keeps README snippets honest by parsing markdown and
/// executing runnable blocks inside small sandboxes (shell-only for now).
#[derive(Parser, Debug)]
#[command(
    name = "runme",
    version,
    about = "Execute README code blocks on demand"
)]
struct Cli {
    /// Path to the primary README markdown file.
    #[arg(default_value = "README.md")]
    target: PathBuf,

    /// Sandbox runtime to execute code blocks with.
    #[arg(long, value_enum, default_value_t = SandboxChoice::Host)]
    sandbox: SandboxChoice,

    /// Container image used when --sandbox=docker (overrides RUNME_DOCKER_IMAGE).
    #[arg(long, value_name = "IMAGE")]
    docker_image: Option<String>,

    /// Repeatable extra arguments forwarded to `docker run`.
    #[arg(long = "docker-arg", value_name = "ARG", action = ArgAction::Append)]
    docker_args: Vec<String>,

    #[command(flatten)]
    run: RunArgs,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List discovered blocks with metadata but do not execute them.
    List,
    /// Execute runnable blocks, optionally targeting a subset.
    Run(RunArgs),
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum ReportFormat {
    Human,
    Json,
}

#[derive(Args, Debug, Clone)]
struct RunArgs {
    /// Block identifier (e.g. block-002) or custom name (`runme:name ...`) to execute.
    #[arg(long)]
    block: Option<String>,
    /// Output format for reports.
    #[arg(long, default_value_t = ReportFormat::Human, value_enum)]
    format: ReportFormat,
}

impl Default for RunArgs {
    fn default() -> Self {
        Self {
            block: None,
            format: ReportFormat::Human,
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
enum SandboxChoice {
    Host,
    Docker,
    Wasm,
}

#[derive(Clone, Debug, Default)]
struct DockerConfig {
    image: Option<String>,
    extra_args: Vec<String>,
}

impl DockerConfig {
    fn from_cli(cli: &Cli) -> Self {
        Self {
            image: cli.docker_image.clone(),
            extra_args: cli.docker_args.clone(),
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let markdown = fs::read_to_string(&cli.target)
        .with_context(|| format!("while reading {}", cli.target.display()))?;
    let blocks = markdown::extract_blocks(&markdown)?;
    warn_duplicate_names(&blocks);

    let workdir = cli
        .target
        .parent()
        // Relative targets such as "README.md" yield an empty parent path; treat it as cwd.
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let docker_config = DockerConfig::from_cli(&cli);

    match &cli.command {
        Some(Command::List) => render_list(&blocks),
        Some(Command::Run(run_args)) => {
            run_blocks(&blocks, &workdir, run_args, cli.sandbox, &docker_config)?
        }
        None => run_blocks(&blocks, &workdir, &cli.run, cli.sandbox, &docker_config)?,
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
        let display_id = if let Some(name) = &block.name {
            format!("{} ({})", block.id, name)
        } else {
            block.id.clone()
        };
        let skip_hint = block
            .skip_reason
            .as_ref()
            .map(|reason| format!(" (skip: {reason})"))
            .unwrap_or_default();
        println!("- {} [{}] {headings}{skip_hint}", display_id, label);
    }
}

fn run_blocks(
    blocks: &[CodeBlock],
    workdir: &Path,
    run_args: &RunArgs,
    sandbox_kind: SandboxChoice,
    docker_config: &DockerConfig,
) -> Result<()> {
    let subset: Vec<&CodeBlock> = match run_args.block.as_deref() {
        Some(key) => {
            let block = blocks
                .iter()
                .find(|block| block.id == key || block.name.as_deref() == Some(key))
                .with_context(|| format!("unknown block id or name {key}"))?;
            vec![block]
        }
        None => blocks.iter().collect(),
    };

    let mut sandbox = instantiate_sandbox(workdir, sandbox_kind, docker_config)?;
    let stream_live = matches!(run_args.format, ReportFormat::Human);
    let mut reports = Vec::new();
    for block in subset {
        let report = runner::execute(block, sandbox.as_mut(), stream_live)
            .with_context(|| format!("while running {}", block.id))?;
        reports.push(report);
    }

    match run_args.format {
        ReportFormat::Human => {
            for report in &reports {
                print_human_report(report, stream_live);
            }
        }
        ReportFormat::Json => {
            let json = serde_json::to_string_pretty(&reports)?;
            println!("{json}");
        }
    }

    Ok(())
}

fn instantiate_sandbox(
    workdir: &Path,
    kind: SandboxChoice,
    docker: &DockerConfig,
) -> Result<Box<dyn Sandbox>> {
    match kind {
        SandboxChoice::Host => Ok(Box::new(HostSandbox::new(workdir))),
        SandboxChoice::Docker => Ok(Box::new(DockerSandbox::new(
            workdir,
            docker.image.clone(),
            docker.extra_args.clone(),
        ))),
        SandboxChoice::Wasm => Ok(Box::new(WasmSandbox::new(workdir))),
    }
}

fn warn_duplicate_names(blocks: &[CodeBlock]) {
    let mut map: HashMap<&str, Vec<&str>> = HashMap::new();
    for block in blocks {
        if let Some(name) = block.name.as_deref() {
            map.entry(name).or_default().push(block.id.as_str());
        }
    }

    let mut duplicates: Vec<_> = map.into_iter().filter(|(_, ids)| ids.len() > 1).collect();
    duplicates.sort_by_key(|(name, _)| *name);

    for (name, ids) in duplicates {
        let list = ids.join(", ");
        eprintln!(
            "warning: runme:name '{}' is used by multiple blocks ({list}); `--block {name}` will target the first match.",
            name
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn sandbox_flag_defaults_to_host() {
        let cli = Cli::try_parse_from(["runme", "list"]).expect("parse default");
        assert_eq!(cli.sandbox, SandboxChoice::Host);
    }

    #[test]
    fn sandbox_flag_accepts_explicit_variants() {
        let docker =
            Cli::try_parse_from(["runme", "--sandbox", "docker", "list"]).expect("parse docker");
        assert_eq!(docker.sandbox, SandboxChoice::Docker);

        let wasm = Cli::try_parse_from(["runme", "--sandbox", "wasm", "list"]).expect("parse wasm");
        assert_eq!(wasm.sandbox, SandboxChoice::Wasm);
    }

    #[test]
    fn docker_cli_flags_capture_configuration() {
        let cli = Cli::try_parse_from([
            "runme",
            "--sandbox",
            "docker",
            "--docker-image",
            "custom:tag",
            "--docker-arg=--env=FOO=bar",
            "--docker-arg=--cpus=1",
            "list",
        ])
        .expect("parse docker options");
        assert_eq!(cli.docker_image.as_deref(), Some("custom:tag"));
        assert_eq!(
            cli.docker_args,
            vec!["--env=FOO=bar".to_string(), "--cpus=1".to_string()]
        );
    }

    #[test]
    fn run_args_parse_without_run_subcommand() {
        let cli = Cli::try_parse_from(["runme", "--block", "block-123"])
            .expect("parse implicit run options");
        assert!(cli.command.is_none());
        assert_eq!(cli.run.block.as_deref(), Some("block-123"));
    }

    #[test]
    fn instantiate_builds_all_backends() {
        let docker_cfg = DockerConfig {
            image: Some("alpine:3.19".into()),
            extra_args: vec!["--cpus=1".into()],
        };
        let host = instantiate_sandbox(Path::new("."), SandboxChoice::Host, &docker_cfg)
            .expect("host sandbox exists");
        assert_eq!(host.label(), "host");

        let docker =
            instantiate_sandbox(Path::new("."), SandboxChoice::Docker, &docker_cfg.clone())
                .expect("docker sandbox exists");
        assert_eq!(docker.label(), "docker");

        let wasm = instantiate_sandbox(Path::new("."), SandboxChoice::Wasm, &docker_cfg)
            .expect("wasm sandbox exists");
        assert_eq!(wasm.label(), "wasm(host-fallback)");
    }
}

fn print_human_report(report: &BlockReport, streamed: bool) {
    let header = if let Some(name) = &report.name {
        format!("{} ({})", report.id, name)
    } else {
        report.id.clone()
    };
    println!("\n== {header} ==");
    if let Some(lang) = &report.language {
        println!("language: {lang}");
    }
    if let Some(sandbox) = &report.sandbox {
        println!("sandbox: {sandbox}");
    }
    if !report.headings.is_empty() {
        println!("context: {}", report.headings.join(" › "));
    }
    println!("status: {:?}", report.status);
    if let Some(reason) = &report.skip_reason {
        println!("skip reason: {reason}");
    }
    if !streamed {
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
}
