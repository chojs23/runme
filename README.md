# RUNME

`runme` scans Markdown docs, finds fenced code blocks, and runs the ones that look like shell scripts. The tool is written in Rust end-to-end to keep everything lean and type-safe.

## How it works

1. Parse `README.md` (and soon, other docs) with `pulldown-cmark`.
2. Create structured block metadata that tracks headings, inferred language, and skip hints.
3. Run shell-flavored blocks with `sh -c` inside the repo root.
4. Report success or failure as human text or JSON.

The executor intentionally supports only shell commands today so we can ship quickly, then grow into container and Wasm sandboxes.

> **Note:** Each non-empty line runs via a direct `execve` call after `shlex` parsing, so pipelines/redirection/conditionals are not supported yet. Add a `runme:ignore` directive if a block needs richer shell semantics.

## Quickstart

<!-- runme:ignore quickstart recursion -->

```bash
cargo run -- list
cargo run -- run
```

- `list` only prints metadata.
- `run` executes every runnable block; add `--block block-002` to target a specific block.
- Add `--format json` to `run` for machine-readable logs.

## Sample blocks inside this README

The snippet below runs successfully and serves as a smoke test when you execute `runme run`:

```bash
echo "README blocks stay honest"
```

This block shows how to prevent execution when a snippet is unsafe or flaky:

<!-- runme:ignore -->

```bash
# TODO: replace with a deterministic integration test command.
```

## Future work

- Add Docker/Wasmtime sandboxes instead of plain `sh`.
- Support language-specific plugin bundles (Python, Node, Cargo, etc.).
- Wire a GitHub Action that reports README drift on pull requests.
- Cache dependencies per block hash for faster reruns.

## Implementation map

See `PROJECT_PLAN.md` for the phased rollout plan.
