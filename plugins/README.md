# Codez Plugins

Codez supports Codex-compatible plugins as an optional runtime layer.

Plugin sources should live in their own repositories when they evolve
independently. The current public companion plugins are:

- RTK Codex Plugin: https://github.com/Krablante/rtk-codex-plugin
- Pitlane Codex Plugin: https://github.com/Krablante/pitlane-codex-plugin

RTK provides shell command rewrite and bounded-output guarding for risky
inspection commands. Pitlane provides indexed code-navigation rewrites through
a host-local `pitlane` CLI. Codez should stay usable without either plugin; the
plugins should stay installable without a gateway.
