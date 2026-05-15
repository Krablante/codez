<h1 align="center">Codez</h1>

<p align="center">
  <strong>A Codex-compatible runtime fork for local agents, token-aware context control, App Server v2, and plugin hooks.</strong>
</p>

<p align="center">
  Codez keeps upstream Codex compatibility where it matters, then layers focused runtime experiments on top.
</p>

<p align="center">
  <a href="./LICENSE">
    <img src="https://img.shields.io/badge/License-Apache--2.0-blue.svg?style=for-the-badge" alt="Apache-2.0 License">
  </a>
  <img src="https://img.shields.io/badge/Rust-CLI%20runtime-b7410e?style=for-the-badge&logo=rust&logoColor=white" alt="Rust CLI runtime">
  <img src="https://img.shields.io/badge/Codex-compatible-111111?style=for-the-badge" alt="Codex-compatible">
  <img src="https://img.shields.io/badge/Plugin%20hooks-optional-0f766e?style=for-the-badge" alt="Optional plugin hooks">
</p>

<p align="center">
  <a href="./docs/compatibility.md">Compatibility</a>
  ·
  <a href="./docs/rtk-plugin.md">RTK Plugin</a>
  ·
  <a href="./docs/pitlane-plugin.md">Pitlane Plugin</a>
  ·
  <a href="./docs/getting-started.md">Getting Started</a>
  ·
  <a href="./docs/config.md">Config</a>
</p>

Codez is a fork of [OpenAI Codex](https://github.com/openai/codex). The goal is
not to hide that lineage: upstream Codex remains the base CLI, protocol, package
shape, and user-facing mental model. Codez is the fork layer for local-agent
runtime work that benefits from moving faster than upstream.

## Why Codez Exists

- keep a Codex-compatible CLI/runtime while experimenting with local-agent needs
- reduce wasted tokens from stale tool, reasoning, and context history
- support plugin and hook workflows that can harden shell usage
- expose App Server v2 surfaces for local clients and future gateways
- make context-pressure and prompt-history behavior easier to evolve
- provide a clear core layer for future gateway projects without baking a
  specific gateway into the runtime

## Core Additions

Codez is not only a renamed README around upstream Codex. The fork carries
runtime work aimed at long local-agent sessions:

- prompt-history pruning before model sampling: stale context messages, older
  reasoning items, tool calls, and tool outputs can be removed before the next
  request while preserving the active turn and image-output dependencies
- goal-continuation pruning: automatic goal follow-up turns are treated as
  fresh prompt boundaries, so previous tool-heavy work can be trimmed while the
  active goal objective remains visible
- compaction-aware pruning: the remote/autocompact path can apply the same
  pruning and trim function-call history before compaction, reducing context
  pressure before a summarization turn
- App Server v2: a local app/server protocol surface for richer clients,
  gateway experiments, thread operations, hook/catalog inspection, and command
  event flows
- plugin hook compatibility: Codez keeps plugin-loaded hook paths usable for
  `PreToolUse` workflows, so optional plugins such as RTK and Pitlane can run
  as normal runtime extensions

## What Is Different

Codez keeps upstream names such as `codex`, `@openai/codex`, and Codex protocol
terms where compatibility matters. Fork-specific docs use the Codez name when
describing additions, release projections, or stack positioning.

The public stack shape is:

```text
Codez runtime
  -> optional RTK Codex Plugin for shell token-safety
  -> optional Pitlane Codex Plugin for indexed code navigation
  -> Telegram gateway layer later (Teledex coming next)
```

RTK and Pitlane are published separately:

https://github.com/Krablante/rtk-codex-plugin
https://github.com/Krablante/pitlane-codex-plugin

The Telegram gateway layer is not linked here yet because its public repo has
not been published.

## Quick Start

Packaged Codez releases are a publication decision. Until then, use the source
tree directly:

```bash
git clone https://github.com/Krablante/codez
cd codez
pnpm install
just install
just codex --help
```

For upstream Codex usage, package names and official setup are still documented
by OpenAI:

- [Codex documentation](https://developers.openai.com/codex)
- [Upstream repository](https://github.com/openai/codex)

## Read Next

- [Compatibility matrix](./docs/compatibility.md)
- [Optional RTK plugin integration](./docs/rtk-plugin.md)
- [Optional Pitlane plugin integration](./docs/pitlane-plugin.md)
- [Installing and building](./docs/install.md)
- [Configuration](./docs/config.md)
- [Contributing](./docs/contributing.md)

## License and Attribution

Codez is distributed under the same [Apache-2.0 license](./LICENSE) as upstream
Codex. OpenAI Codex remains the upstream project; Codez is an independent fork
projection and is not presented as an official OpenAI release.
