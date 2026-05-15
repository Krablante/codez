# Codez Plugins

Codez supports Codex-compatible plugins as an optional runtime layer.

Plugin sources should live in their own repositories when they evolve
independently. The first public companion plugin is RTK Codex Plugin:

https://github.com/Krablante/rtk-codex-plugin

RTK provides shell command rewrite and bounded-output guarding for risky
inspection commands. Codez should stay usable without RTK; RTK should stay
installable without a gateway.
