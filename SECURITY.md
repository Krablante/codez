# Security Policy

Codez is an independent public fork projection of OpenAI Codex. Security issues
in Codez-specific behavior should be reported through the Codez GitHub
repository.

## Reporting a Vulnerability

Please open a private security advisory on GitHub when the issue is sensitive.
For non-sensitive hardening issues, open a public issue with reproduction steps
and affected versions or commits.

If the issue is in upstream OpenAI Codex rather than Codez-specific fork code,
follow the upstream reporting guidance from OpenAI.

## Scope

Codez keeps the same broad security model as Codex: local execution requires
careful sandboxing, approval settings, model/tool trust boundaries, and prompt
context hygiene. The optional RTK Codex Plugin can add shell-output guarding and
command rewrite behavior, but Codez must remain safe to inspect and run without
assuming RTK is installed.
