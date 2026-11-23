# Changelog

Notable changes to the aws-health-event-notifier app (the container image), newest first. Entries are headed by the image version and its release date (KST).

## 0.4.1 (2026-11-19)

Dependency update for RUSTSEC advisories, with one error reporting fix.

### Security

cargo audit reported 25 vulnerabilities and now reports 0.

- url 2.5.9 (RUSTSEC-2026-0592), released after the first build of this version, so the image is rebuilt at the same version.
- vergen-gitcl replaces vergen-gix for the build metadata. It reads the commit from the git CLI, so the gix crates and the heapless 0.8.0 they pin leave the lockfile (RUSTSEC-2026-0574 to 0582). The binary carries the same short SHA and dirty flag.
- The AWS SDK crates drop their default features and keep only the hyper 1 HTTPS client. This removes the legacy hyper 0.14 TLS stack (rustls 0.21.12, rustls-webpki 0.101.7, h2 0.3.27) and the SigV4a signing crates (crypto-bigint 0.4.9 and 0.5.5), none of which has a patched release in its line (RUSTSEC-2026-0098, 0099, 0104, 0258, 0346, 0349, 0382, 0387).
- rustls 0.23.46 (RUSTSEC-2026-0285, 0346, 0348, 0349), rustls-webpki 0.103.17 (RUSTSEC-2026-0382), and webpki-roots 1.0.12 (RUSTSEC-2026-0381).
- h2 0.4.20 (RUSTSEC-2026-0258).
- quinn-proto 0.11.19 (RUSTSEC-2026-0185, 0338, 0339, 0340, 0342, 0523, 0524) and quinn-udp 0.5.16 (RUSTSEC-2026-0538).
- ctutils 0.4.8 (RUSTSEC-2026-0385).
- Unsound warnings: tokio 1.54.1 (RUSTSEC-2026-0386), anyhow 1.0.104 (RUSTSEC-2026-0190), event-listener 5.4.2 (RUSTSEC-2026-0221), memmap2 0.9.12 (RUSTSEC-2026-0186), and rustix 1.1.6 (RUSTSEC-2026-0486).
- The unmaintained warnings for chrono, pin-utils (through the AWS SDK), and unsafe-libyaml (through serde_yaml) remain, as none has a replacement release in the current dependency set.

### Fixed

- A failed AWS Health, STS, or IAM call reported only `service error`. The message now carries the service error code, so startup and poll logs name the cause, for example `SubscriptionRequiredException` on an account without a Business or Enterprise Support plan.

### Tests

- `make test-emulator` runs the STS, IAM, and AWS Health client code against a local AWS Emulator. See [development](docs/development.md).
- Unit tests now cover startup filter validation against the AWS Health catalog, the leader election preflight, account identity resolution, the `send` subcommand's event lookup, and the error path of every AWS Health call.
- The AWS Health replay client comes from aws-smithy-http-client instead of aws-smithy-runtime, whose test-util feature pulled in hyper 0.14.
