# Codez Plugins

Codez supports Codex-compatible plugins as an optional runtime layer.

Plugin sources should live in their own repositories when they evolve
independently. Public companion plugins:

- https://github.com/Krablante/rtk-codex-plugin
- https://github.com/Krablante/pitlane-codex-plugin

RTK provides shell command rewrite and bounded-output guarding for risky
inspection commands. Pitlane provides indexed code-navigation rewrites through
host-local Pitlane CLI. Codez should stay usable without either plugin; plugins
should stay installable without a gateway.
