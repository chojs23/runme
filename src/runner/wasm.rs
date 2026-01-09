use std::path::PathBuf;

use anyhow::Result;

use super::host::HostSandbox;
use super::sandbox::{CommandOutcome, Sandbox};

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
