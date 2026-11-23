# Changelog

Notable changes to the aurora-database-insights-exporter app (the container image), newest first. Entries are headed by the image version and its release date (KST).

## 0.1.3 (2026-11-18)

Dependency update for RUSTSEC advisories, with one error reporting fix.

### Security

Only the crates with an advisory move, so the lockfile stays close to 0.1.2.

- url 2.5.9 (RUSTSEC-2026-0592), released after the first build of this version, so the image is rebuilt at the same version.
- rustls 0.23.46 (RUSTSEC-2026-0285, 0346, 0348, 0349) and rustls-webpki 0.103.17 (RUSTSEC-2026-0098, 0099, 0104, 0382).
- h2 0.4.20 (RUSTSEC-2026-0258).
- prometheus 0.14.0, which moves protobuf to 3.7.2 (RUSTSEC-2024-0437).
- tokio 1.54.1 for its unsound warning (RUSTSEC-2026-0386), and fastrand 2.5.0 to replace a yanked release.
- The unmaintained warnings for chrono, pin-utils (through the AWS SDK), and unsafe-libyaml (through serde_yaml) remain, as none has a replacement release in the current dependency set.

### Fixed

- A failed `DescribeDBInstances` or `GetResourceMetrics` call reported only `service error`. The message now carries the service error code, so the log names the cause and the `error_kind` label of `aurora_dbinsights_pi_api_errors_total` classifies throttling and authorization failures instead of counting them as `other`.

### Tests

- `make test-emulator` runs the RDS and Performance Insights client code against a local AWS Emulator. See [development](docs/development.md).
