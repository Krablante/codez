# Contributing to Codez

Codez is a focused public fork projection of OpenAI Codex. Contributions should
keep upstream compatibility clear, avoid private deployment assumptions, and
make fork-specific behavior explicit.

## Good Contributions

- bug reports with concrete reproduction steps
- documentation fixes that clarify Codez vs upstream Codex behavior
- small compatibility fixes
- focused plugin-hook or local-agent runtime improvements
- tests for Codez-specific behavior

## Ground Rules

- Keep upstream attribution intact.
- Do not rename Codex concepts where compatibility depends on the upstream
  name.
- Do not add private host paths, local operator notes, private remotes, tokens,
  or deployment-specific assumptions.
- Keep RTK as an optional plugin, not a required runtime dependency.
- Keep gateway integrations optional and separate from the Codez core.

## Development

Start from the public source tree:

```bash
git clone https://github.com/Krablante/codez
cd codez
pnpm install
just install
```

Run the narrowest relevant checks for the files you changed. Full Rust and
Bazel sweeps can be expensive, so prefer scoped tests first and document any
checks you could not run.

## Pull Requests

Open an issue first for behavior changes, compatibility changes, or anything
that affects plugin hooks. Pull requests should be focused, explain the user
impact, and include verification notes.
