# PTY benchmark baselines

Baselines include the machine, platform, terminal, dimensions, build profile,
measured frame-event count, and per-scenario results. A comparison fails when
the platform, terminal, dimensions, or build profile differs. It also fails
when any scenario's p99 frame time grows by more than 15% by default.

File naming: `<platform>.json` where `<platform>` matches the CI artifact
arch name — `linux-x86_64`, `linux-aarch64`, `macos-aarch64`.

## Producing a baseline

Run the full bench suite on a quiet machine:

```bash
cargo build --release -p xai-grok-pager-bin --bin bot
cargo bench -p xai-grok-pager-pty-harness --bench pty_bench -- \
  --all \
  --binary target/release/bot \
  --write-baseline crates/codegen/xai-grok-pager-pty-harness/benches/pty_baselines/<platform>.json
```

## Overwriting after an intentional perf change

A PR that intentionally shifts frame timing (either direction) must update
the affected baselines. Include the `pty-bench` output from a clean run in
the PR body so reviewers can sanity-check the new numbers.

Pass `--machine` or `--build-profile` when the harness cannot infer that value.
Use `--terminal` to test a TERM profile other than `xterm-256color`. A missing
baseline fails before a release comparison.
