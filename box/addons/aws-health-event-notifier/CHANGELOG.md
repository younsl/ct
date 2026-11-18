# Changelog

Notable changes to the aws-health-event-notifier app (the container image), newest first. Entries are headed by the image version and its release date (KST).

## 0.4.1 (2026-11-19)

Dependency update for RUSTSEC advisories. No behavior change.

### Security

cargo audit reported 25 vulnerabilities and now reports 0.

- The AWS SDK crates drop their default features and keep only the hyper 1 HTTPS client. This removes the legacy hyper 0.14 TLS stack (rustls 0.21.12, rustls-webpki 0.101.7, h2 0.3.27) and the SigV4a signing crates (crypto-bigint 0.4.9 and 0.5.5), none of which has a patched release in its line (RUSTSEC-2026-0098, 0099, 0104, 0258, 0346, 0349, 0382, 0387).
- rustls 0.23.46 (RUSTSEC-2026-0285, 0346, 0348, 0349), rustls-webpki 0.103.17 (RUSTSEC-2026-0382), and webpki-roots 1.0.12 (RUSTSEC-2026-0381).
- h2 0.4.20 (RUSTSEC-2026-0258).
- quinn-proto 0.11.19 (RUSTSEC-2026-0185, 0338, 0339, 0340, 0342, 0523, 0524) and quinn-udp 0.5.16 (RUSTSEC-2026-0538).
- ctutils 0.4.8 (RUSTSEC-2026-0385).
- Unsound warnings: tokio 1.54.1 (RUSTSEC-2026-0386), anyhow 1.0.104 (RUSTSEC-2026-0190), event-listener 5.4.2 (RUSTSEC-2026-0221), memmap2 0.9.12 (RUSTSEC-2026-0186), and rustix 1.1.6 (RUSTSEC-2026-0486).
- The unmaintained warnings for chrono, pin-utils (through the AWS SDK), and unsafe-libyaml (through serde_yaml) remain, as does the unsound warning for faster-hex 0.10.0 (RUSTSEC-2026-0306), a build-time dependency of vergen-gix that is not linked into the binary. None has a replacement release in the current dependency set.

### Tests

- The AWS Health replay client comes from aws-smithy-http-client instead of aws-smithy-runtime, whose test-util feature pulled in hyper 0.14.
