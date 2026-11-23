# Changelog

Notable changes to the kagent-gateway app (the container image), newest first. Entries are headed by the image version and its release date (KST).

## 0.3.1 (2026-11-18)

Dependency update for RUSTSEC advisories. No behavior change.

### Security

Only the crates with an advisory move, so the lockfile stays close to 0.3.0.

- url 2.5.9 (RUSTSEC-2026-0592), released after the first build of this version, so the image is rebuilt at the same version.
- rustls 0.23.46 (RUSTSEC-2026-0285, 0346, 0348, 0349), rustls-webpki 0.103.17 (RUSTSEC-2026-0382), webpki-roots 1.0.12 (RUSTSEC-2026-0381).
- tokio 1.54.1 for its unsound warning (RUSTSEC-2026-0386).
- The unmaintained warning for unsafe-libyaml remains, as it has no replacement release.
