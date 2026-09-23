# ADR-0007: Secrets — OS keychain primary, file fallback with surfaced warning

- Status: Accepted
- Date: 2026-09-23
- Deciders: EigenDesk core

## Context

Spec §44–45: secrets (API keys etc.) must never sit in plain config
files, must be owner-scoped per plugin, and the app must work on
machines without a desktop keychain (headless Linux, CI, containers).

## Decision

1. **Primary backend**: the OS keychain via the `keyring` crate —
   Linux Secret Service (D-Bus), macOS Keychain, Windows Credential
   Manager. Entries are namespaced `eigendesk / <pluginId>/<key>`.
2. **Documented fallback**: when no Secret Service is reachable, secrets
   are stored in a single `0600`-permission JSON file under XDG data
   (`secrets-fallback.json`), and the UI **surfaces a warning** that
   secrets are file-protected only. Settings files are never used.
3. **Owner scoping**: `secrets.get/set/delete` operate on
   `<callerPluginId>/<key>` only — a plugin cannot read another
   plugin's namespace, and the shell never receives secret values into
   JavaScript.
4. The fallback file's permissions are verified on startup; a
   group/world-readable file is a hard error, not a warning.

## Consequences

- On workstations with a keychain, nothing sensitive ever touches disk
  in plaintext.
- Headless/CI use remains possible; the degraded mode is visible, not
  silent (spec: silent degradation is a bug).
- Rotation/clearing = delete the namespace; no hidden copies.
