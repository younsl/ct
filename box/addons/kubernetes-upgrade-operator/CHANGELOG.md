# Changelog

Notable changes to the kubernetes-upgrade-operator app (the container image), newest first. Entries are headed by the image version and its release date (KST).

## 0.6.1 (2026-11-18)

Dependency update for RUSTSEC advisories. No behavior change.

### Security

Only the crates with an advisory move, so the lockfile stays close to 0.6.0.

- aws-lc-rs 1.19.0 with aws-lc-sys 0.47.0 (RUSTSEC-2026-0044, 0045, 0046, 0047, 0048).
- rustls 0.23.46 (RUSTSEC-2026-0285, 0346, 0348, 0349), rustls-webpki 0.103.17 (RUSTSEC-2026-0049, 0098, 0099, 0104, 0382), webpki-roots 1.0.12 (RUSTSEC-2026-0381).
- h2 0.4.20 (RUSTSEC-2026-0258).
- rmcp 3.11.0 for the MCP server (RUSTSEC-2026-0457, 0461, 0462). `ServerInfo` is deprecated in favor of `ServerConfig`, so the MCP handler uses the new name.
- tokio 1.54.1, anyhow 1.0.104, and event-listener 5.4.2 for their unsound warnings (RUSTSEC-2026-0386, 0190, 0221), and chacha20 0.10.2 to replace a yanked release.
- The unmaintained warnings for chrono, pin-utils, and unsafe-libyaml remain, as none has a replacement release in the current dependency set.
