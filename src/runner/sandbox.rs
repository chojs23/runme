use anyhow::Result;
use std::io::{BufRead, BufReader};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

/// Low-level hook that sandboxes use to stream stdout/stderr data back to the runner.
pub trait OutputSink {
    fn on_stdout(&mut self, chunk: &str);
    fn on_stderr(&mut self, chunk: &str);
}

/// Minimal status metadata returned after a sandboxed command exits.
#[derive(Clone, Debug)]
pub struct CommandStatus {
    pub exit_code: Option<i32>,
    pub success: bool,
    pub duration: Duration,
}

impl CommandStatus {
    pub fn from_output(output: Output, duration: Duration) -> Self {
        Self {
            exit_code: output.status.code(),
            success: output.status.success(),
            duration,
        }
    }

    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }
}

/// Trait implemented by every sandbox backend (host, Docker, Wasm, ...).
pub trait Sandbox {
    /// Short label surfaced in reports, e.g. `host` or `docker:ubuntu-22.04`.
    fn label(&self) -> &str;
    /// Run a parsed argv vector inside the sandbox environment and push stdout/stderr chunks
    /// into the supplied sink as they arrive.
    fn run(&mut self, argv: &[String], sink: &mut dyn OutputSink) -> Result<CommandStatus>;
}

pub fn spawn_with_streaming(
    mut command: Command,
    sink: &mut dyn OutputSink,
) -> Result<CommandStatus> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let (tx, rx) = mpsc::channel::<(StreamKind, anyhow::Result<String>)>();
    let mut handles = Vec::new();

    if let Some(stdout) = child.stdout.take() {
        handles.push(spawn_reader(StreamKind::Stdout, stdout, tx.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        handles.push(spawn_reader(StreamKind::Stderr, stderr, tx.clone()));
    }
    drop(tx);

    for (kind, msg) in rx {
        let chunk = msg?;
        match kind {
            StreamKind::Stdout => sink.on_stdout(&chunk),
            StreamKind::Stderr => sink.on_stderr(&chunk),
        }
    }

    for handle in handles {
        handle.join().expect("stream thread panicked")?;
    }

    let output = child.wait_with_output()?;
    Ok(CommandStatus::from_output(output, Duration::default()))
}

fn spawn_reader(
    kind: StreamKind,
    stream: impl std::io::Read + Send + 'static,
    sender: Sender<(StreamKind, Result<String>)>,
) -> thread::JoinHandle<Result<()>> {
    thread::spawn(move || {
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            let bytes = reader.read_line(&mut line)?;
            if bytes == 0 {
                break;
            }
            let trimmed = line.trim_end_matches(&['\r', '\n'][..]).to_string();
            if trimmed.is_empty() {
                continue;
            }
            sender
                .send((kind, Ok(trimmed)))
                .map_err(|_| anyhow::anyhow!("output channel closed"))?;
        }
        Ok(())
    })
}

#[derive(Copy, Clone)]
enum StreamKind {
    Stdout,
    Stderr,
}
