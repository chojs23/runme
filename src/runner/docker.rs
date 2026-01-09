use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context, Result};

use super::sandbox::{CommandStatus, OutputSink, Sandbox, spawn_with_streaming};

/// Docker sandbox that runs each line inside a disposable container.
///
/// Environment variables:
/// - `RUNME_DOCKER_IMAGE`: override the base image (default `ubuntu:22.04`).
pub struct DockerSandbox {
    mount_dir: PathBuf,
    image: String,
    extra_args: Vec<String>,
}

impl DockerSandbox {
    pub fn new(
        workdir: impl Into<PathBuf>,
        image: Option<String>,
        extra_args: Vec<String>,
    ) -> Self {
        let workdir = workdir.into();
        let mount_dir = workdir.canonicalize().unwrap_or_else(|_| workdir.clone());
        let image = image
            .or_else(|| env::var("RUNME_DOCKER_IMAGE").ok())
            .unwrap_or_else(|| "ubuntu:22.04".to_string());
        Self {
            mount_dir,
            image,
            extra_args,
        }
    }

    #[cfg(test)]
    pub(crate) fn image(&self) -> &str {
        &self.image
    }

    #[cfg(test)]
    pub(crate) fn extra_args(&self) -> &[String] {
        &self.extra_args
    }
}

impl Sandbox for DockerSandbox {
    fn label(&self) -> &str {
        "docker"
    }

    fn run(&mut self, argv: &[String], sink: &mut dyn OutputSink) -> Result<CommandStatus> {
        let mut volume_spec = OsString::new();
        volume_spec.push(&self.mount_dir);
        volume_spec.push(":");
        volume_spec.push("/workspace");

        let mut cmd = Command::new("docker");
        cmd.arg("run")
            .arg("--rm")
            .arg("--network=none")
            .arg("-v")
            .arg(&volume_spec)
            .arg("-w")
            .arg("/workspace")
            .args(&self.extra_args)
            .arg(&self.image)
            .args(argv);

        let start = Instant::now();
        let status = spawn_with_streaming(cmd, sink).context("while invoking docker")?;
        Ok(status.with_duration(start.elapsed()))
    }
}
