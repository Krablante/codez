<h1 align="center">Codez</h1>

<p align="center">
  <strong>A Codex-compatible runtime fork for local agents, plugin hooks, and practical context control.</strong>
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
- support plugin and hook workflows that can harden shell usage
- make context-pressure and prompt-history behavior easier to evolve
- provide a clear core layer for future gateway projects without baking a
  specific gateway into the runtime

## What Is Different

Codez keeps upstream names such as `codex`, `@openai/codex`, and Codex protocol
terms where compatibility matters. Fork-specific docs use the Codez name when
describing additions, release projections, or stack positioning.

The public stack shape is:

```text
Codez runtime
  -> optional RTK Codex Plugin for shell token-safety
  -> optional gateway layer later
```

RTK is already published separately:

https://github.com/Krablante/rtk-codex-plugin

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
- [Installing and building](./docs/install.md)
- [Configuration](./docs/config.md)
- [Contributing](./docs/contributing.md)

## License and Attribution

Codez is distributed under the same [Apache-2.0 license](./LICENSE) as upstream
Codex. OpenAI Codex remains the upstream project; Codez is an independent fork
projection and is not presented as an official OpenAI release.
