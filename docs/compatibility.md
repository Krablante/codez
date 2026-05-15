# Codez Compatibility

Codez is intentionally close to upstream Codex where compatibility matters.
Fork-specific behavior should be explicit and optional where possible.

| Surface | Upstream Codex | Codez | Notes |
| --- | --- | --- | --- |
| CLI command | `codex` | `codex` | Kept compatible so scripts and habits transfer. |
| NPM package shape | `@openai/codex` | publication decision | Public package naming is not finalized in this projection. |
| License | Apache-2.0 | Apache-2.0 | Keep upstream attribution intact. |
| Core Rust runtime | upstream baseline | forked runtime | Codez changes should stay bounded and documented. |
| Plugin loading | upstream-compatible where present | active fork surface | Used for local plugin workflows and hook experiments. |
| Plugin hooks | optional feature | optional feature | Enable via config when a runtime supports plugin hooks. |
| RTK Codex Plugin | external | optional external plugin | Adds shell rewrite and bounded-output guard behavior. |
| Gateway layer | not required | not required | Gateway projects can use Codez, but Codez should not depend on them. |

## Plugin Hook Shape

A Codex-compatible plugin hook runtime needs:

- plugin manifests under `.codex-plugin/plugin.json`
- hook declarations under `hooks/hooks.json`
- `PreToolUse` support for shell/Bash calls
- `${PLUGIN_ROOT}` expansion in hook commands

Example feature flags:

```toml
[features]
plugins = true
plugin_hooks = true
```

## Boundaries

Codez should not include private host registries, gateway state, local operator
paths, or deployment assumptions in the public projection. Those belong in
separate private docs or future gateway projects.
