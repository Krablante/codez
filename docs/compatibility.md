# Codez Compatibility

Codez is intentionally close to upstream Codex where compatibility matters.
Fork-specific behavior should be explicit and optional where possible.

| Surface | Upstream Codex | Codez | Notes |
| --- | --- | --- | --- |
| CLI command | `codex` | `codex` | Kept compatible so scripts and habits transfer. |
| NPM package shape | `@openai/codex` | publication decision | Public package naming is not finalized in this projection. |
| License | Apache-2.0 | Apache-2.0 | Keep upstream attribution intact. |
| Core Rust runtime | upstream baseline | forked runtime | Codez changes should stay bounded and documented. |
| Prompt-history pruning | upstream behavior | Codez token-control path | Prunes stale context, older reasoning, tool calls, and tool outputs before model sampling when safe. |
| Goal-continuation pruning | upstream behavior | Codez token-control path | Treats automatic goal follow-up prompts as fresh prompt boundaries while preserving live-steering safety. |
| Remote/autocompact pruning | upstream behavior | Codez compaction path | Applies pruning before remote compaction and can trim function-call history to reduce context pressure. |
| App Server v2 | upstream-compatible where present | active fork surface | Local client/gateway protocol surface for thread operations, command events, hook/catalog inspection, and richer integrations. |
| Plugin loading | upstream-compatible where present | active fork surface | Used for local plugin workflows and hook experiments. |
| Plugin hooks | optional feature | supported Codez use case | Enable via config when a runtime supports plugin hooks; Codez keeps plugin-loaded hook paths usable for RTK-style workflows. |
| RTK Codex Plugin | external | optional external plugin | Adds shell rewrite and bounded-output guard behavior. |
| Gateway layer | not required | not required | Gateway projects can use Codez, but Codez should not depend on them. |

## Token-Control Shape

Codez includes runtime paths for reducing token waste in long sessions:

- before normal model sampling, stale context and older tool/reasoning history
  can be pruned while keeping the active turn intact
- automatic goal follow-up turns can prune stale tool-heavy work from the
  previous goal turn while keeping the active goal objective visible
- before remote/autocompact summarization, the same pruning can be applied so
  the compaction request is not forced to carry avoidable historical tool noise
- function-call history can be trimmed before remote compaction when the context
  window is already under pressure

This is separate from shell-output guarding. Prompt-history pruning reduces what
Codez sends to the model; RTK-style plugins reduce risky shell output before it
enters the conversation.

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
