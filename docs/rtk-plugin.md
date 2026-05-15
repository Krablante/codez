# Optional RTK Codex Plugin

The RTK Codex Plugin is the optional shell-safety layer for Codez-compatible
runtimes. It relies on a runtime that actually loads plugin hooks for shell
calls; Codez keeps that compatibility surface as a supported fork feature.

Public repo:

https://github.com/Krablante/rtk-codex-plugin

## What It Adds

- `PreToolUse` shell command rewrite through `rtk rewrite`
- bounded stdout for risky long-line inspections
- protection for common JSONL, log, and prompt-capture token floods
- pass-through behavior for exact-output commands, tests, JSON modes, and
  interactive commands

The output guard works even when the `rtk` binary is not installed. Rewrite mode
requires `rtk` in `PATH`.

RTK complements Codez's built-in prompt-history pruning. Codez reduces stale
tool/reasoning/context history before sampling and compaction; RTK reduces
risky shell output before it becomes model-visible history. Pitlane can be
enabled separately for indexed source-navigation rewrites.

## Example Config

The exact plugin key depends on the runtime's plugin cache layout. A common
GitHub-cache style install can look like this:

```toml
[features]
plugins = true
plugin_hooks = true

[plugins."rtk-codex-plugin@github"]
enabled = true
```

## Stack Role

Codez is the runtime. RTK is an optional plugin. Pitlane is another optional
plugin that should be configured after RTK when both are enabled. A gateway can
install or sync plugins for worker machines, but Codez itself should stay usable
without any gateway.
