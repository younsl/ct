# Changelog

Notable changes to the argocd-canary-gate app (the container image), newest first. Entries are headed by the image version and its release date (KST).

## 0.1.1 (2026-11-19)

Dependency cleanup after a full advisory audit. No behavior change.

### Security

The audit (cargo audit and the OSV database over all 255 locked crates) found no vulnerability, only two unmaintained warnings.

- rustls-pemfile is removed (RUSTSEC-2025-0134). The webhook keypair is now parsed by the PEM reader in rustls-pki-types, which the crate already depends on through rustls.
- The unmaintained warning for unsafe-libyaml (RUSTSEC-2024-0449, through serde_yaml) remains, as no maintained replacement drops it.

## 0.1.0

Initial release.
