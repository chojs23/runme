# RUNME

`runme` scans Markdown docs, finds fenced code blocks, and runs the ones that look like shell scripts. The tool is written in Rust end-to-end to keep everything lean and type-safe.

## How it works

1. Parse `README.md` (and soon, other docs) with `pulldown-cmark`.
2. Create structured block metadata that tracks headings, inferred language, and skip hints.
3. Run shell-flavored blocks via a sandbox (host shell by default, Docker via `--sandbox docker`).
4. Report success or failure as human text (with live, colorized streaming) or JSON.

The executor intentionally supports only shell commands today so we can ship quickly, then grow into container and Wasm sandboxes.

> **Note:** Each non-empty line runs via a direct `execve` call after `shlex` parsing, so pipelines/redirection/conditionals are not supported yet. Add a `runme:ignore` directive if a block needs richer shell semantics.

## Quickstart

```bash <!-- runme:ignore quickstart recursion -->
cargo run -- list
cargo run -- run
cargo run -- run --format json
cargo run -- --sandbox docker --block block-002 --docker-arg=--env=FOO=bar
```

- `list` only prints metadata.
- `run` executes every runnable block; add `--block block-002` to target a specific block.
- Add `--format json` to `run` for machine-readable logs; omit it to see live, colorized stdout/stderr as each command runs.
- Use `--sandbox docker` to isolate commands inside a container (override the image with `--docker-image` or `RUNME_DOCKER_IMAGE`, and forward additional docker flags with repeated `--docker-arg`).

## Sample blocks inside this README

The snippet below runs successfully and serves as a smoke test when you execute `runme run`:

```bash runme:name=smoke-test
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
- Name runnable blocks either by placing `<!-- runme:name my-friendly-label -->` immediately before the fenced code or by adding `runme:name=my-friendly-label` after the fence info string (e.g., ` ```bash runme:name=my-friendly-label `); then invoke the block with `--block my-friendly-label`.
