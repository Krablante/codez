# Codez Stack

The Codez stack is one system, but it is intentionally split into small layers.
Use the lower layers alone when you only need local Codex-compatible execution;
add the higher layers only when you need their job.

## Layers

| Layer                                                                     | Public surface                 | Responsibility                                                                    | Dependency                                                                                                                    |
| ------------------------------------------------------------------------- | ------------------------------ | --------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| [Codez](https://github.com/Krablante/codez)                               | Codex-compatible runtime       | App Server v2, goal RPC, long-session hardening, prompt pruning, and plugin hooks | Does not require Teledex                                                                                                      |
| [RTK Codex Plugin](https://github.com/Krablante/rtk-codex-plugin)         | Optional Codex plugin          | Shell/token safety through `rtk rewrite` and bounded output guarding              | Requires a Codex-compatible plugin-hook runtime; does not require Teledex                                                     |
| [Pitlane Codex Plugin](https://github.com/Krablante/pitlane-codex-plugin) | Optional Codex plugin          | Code-navigation/token-saving rewrites through a host-local `pitlane` CLI          | Requires a Codex-compatible plugin-hook runtime and local `pitlane`; does not require Teledex                                 |
| Teledex (planned public repo: `Krablante/teledex`)                        | Telegram gateway/session layer | Topics, queues, live steer, `/goal` UX, and multi-host delivery/recovery          | Basic mode can drive upstream Codex; full mode requires a Codez-compatible runtime with App Server v2 and plugin-hook support |

## Plain Version

Codez runs the agent. RTK makes risky shell output safer and smaller. Pitlane
makes routine code browsing smaller by replacing safe source reads with indexed
CLI calls. Teledex is the Telegram/session gateway that can drive a runtime,
but it is not part of the Codez runtime itself.

Codez does not depend on Teledex. RTK and Pitlane do not depend on Teledex.
Teledex can run a basic Codex path, but its full `/goal`, live-steering, and
plugin-sync workflow expects a Codez-compatible runtime with the App Server v2
and plugin-hook surfaces.

## Public Status

- Codez: public at <https://github.com/Krablante/codez>
- RTK Codex Plugin: public at <https://github.com/Krablante/rtk-codex-plugin>
- Pitlane Codex Plugin: public at <https://github.com/Krablante/pitlane-codex-plugin>
- Teledex: planned public name `Krablante/teledex`; no public repository is
  linked until that repo exists.
