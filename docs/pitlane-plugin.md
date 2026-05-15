# Optional Pitlane Codex Plugin

The Pitlane Codex Plugin is the optional indexed code-navigation layer for
Codez-compatible runtimes. It relies on a runtime that actually loads plugin
hooks for shell calls; Codez keeps that compatibility surface as a supported
fork feature.

Public repo:

https://github.com/Krablante/pitlane-codex-plugin

## What It Adds

- `PreToolUse` rewrites for safe source `cat`, `head`, and simple `sed` reads
- bounded `pitlane lines` output for routine source browsing
- indexed symbol search and repo outline rewrites for simple recursive
  exploration commands
- pass-through behavior for exact-output commands, regex-like searches, tests,
  JSON modes, build commands, Docker, SSH, data reads, and shell control

Rewrites require a host-local `pitlane` CLI in `PATH`. Symbol and outline
rewrites require an existing Pitlane index for the project. If either condition
is missing, the plugin leaves the original command unchanged.

Pitlane complements Codez's built-in prompt-history pruning. Codez reduces
stale tool/reasoning/context history before sampling and compaction; Pitlane
reduces routine code-browsing output before it becomes model-visible history.

## Example Config

The exact plugin key depends on the runtime's plugin cache layout. A common
GitHub-cache style install can look like this:

```toml
[features]
plugins = true
plugin_hooks = true

[plugins."pitlane-codex-plugin@github"]
enabled = true
```

When RTK and Pitlane are both enabled, configure RTK first and Pitlane after it:

```toml
[plugins."rtk-codex-plugin@github"]
enabled = true

[plugins."pitlane-codex-plugin@github"]
enabled = true
```

## Stack Role

Codez is the runtime. Pitlane is an optional plugin. RTK is the companion shell
token-safety plugin and should run before Pitlane when both are installed. A
gateway can install or sync plugins for worker machines, but Codez itself
should stay usable without any gateway.
