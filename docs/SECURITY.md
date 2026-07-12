# Security

## Reporting a vulnerability

Do **not** open a public GitHub issue for security vulnerabilities, memory exploits, or protocol leaks. Report them confidentially via [GitHub Private Vulnerability Reporting](https://github.com/Void-next-gen-WebRTC/Void/security/advisories/new) (repository **Security** tab → **Report a vulnerability**). This keeps the report private between you and the maintainers until a fix ships.

## TLS certificate pinning

The desktop client (`apps/desktop/src-tauri/src/tls.rs`) uses a custom `rustls::client::danger::ServerCertVerifier` instead of the system trust store, for its first-party connections.

- **Pinned hosts**: `voidsfu.com`, `www.voidsfu.com`, `api.voidsfu.com`, and the VM's public IP. Only these are checked against the pinned SPKI hash — everything else (e.g. the Tauri auto-updater hitting GitHub) passes through the normal TLS verification path untouched.
- **Pin hashes** (`PRIMARY_PIN_HASH`, `BACKUP_PIN_HASH`) are injected at **compile time** via environment variables during the release build — they're baked into the binary, not read at runtime. A build with neither set falls back to a `"DEV_PIN"` placeholder and is treated as a dev build.
- **Bypass conditions**: pinning is skipped for non-pinned hosts (by design), and entirely bypassed in debug builds, test builds, and dev builds (`is_dev_build()`) — this prevents a stale pin baked into a developer's local build from breaking sign-in during day-to-day development. Release builds enforce the pin strictly: a mismatch is a hard connection failure, not a warning.
- **Why two pins**: `PRIMARY_PIN` and `BACKUP_PIN` allow rotating the production certificate's key without an app update causing an outage — the old pin keeps working until the new cert is confirmed live, then the primary is retired in a later release.

## Licensing & CLA

Void is distributed under **BSL-1.1**, converting to **GPL-3.0-or-later** on April 7, 2031 (see [LICENSE](../LICENSE)). All contributions require signing the [CLA](../CLA.md) — see [CONTRIBUTING.md](../CONTRIBUTING.md#legal--licensing) for the mechanics of the automated CLA check.

## Staging isolation

The staging environment (see [DEPLOYMENT.md](./DEPLOYMENT.md)) uses a separate JWT signing secret from production. A session token issued by staging is never valid against production infrastructure, and vice versa — this is deliberate, so that testing against staging can never leak into or affect real user sessions.
