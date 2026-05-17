# Getting Started with Codez

Codez is a public source projection of a Codex-compatible runtime. Packaged
Codez releases are still a publication decision, so start from the source tree:

```bash
git clone https://github.com/Krablante/codez
cd codez/codex-rs
cargo build
cargo run --bin codex -- "explain this codebase to me"
```

For gateway integrations such as Teledex full mode, use App Server v2:

```bash
cargo run --bin codex -- app-server --listen stdio://
```

For full prerequisites, Rust toolchain setup, and verification commands, see
[Installing and Building Codez](./install.md). For inherited upstream Codex CLI
behavior, see the [official Codex CLI feature documentation](https://developers.openai.com/codex/cli/features#running-in-interactive-mode).
