# Changelog

Notable changes to the argocd-canary-gate app (the container image), newest first. Entries are headed by the image version and its release date (KST).

## 0.1.1 (2026-11-19)

Dependency update for RUSTSEC advisories. No behavior change.

### Security

The 0.1.0 image was built before these crates were updated in the lockfile, so 0.1.1 is the first image that carries the fixes.

- rustls 0.23.46 (RUSTSEC-2026-0285, 0346, 0348, 0349), rustls-webpki 0.103.17 (RUSTSEC-2026-0382), webpki-root-certs 1.0.12 (RUSTSEC-2026-0383).
- tokio 1.54.1 and rustix 1.1.6 for their unsound warnings (RUSTSEC-2026-0386, 0486).
- rustls-pemfile is removed (RUSTSEC-2025-0134). The webhook keypair is now parsed by the PEM reader in rustls-pki-types, which the crate already depends on through rustls.
- The unmaintained warning for unsafe-libyaml (RUSTSEC-2024-0449, through serde_yaml) remains, as no maintained replacement drops it.

## 0.1.0

Initial release.
