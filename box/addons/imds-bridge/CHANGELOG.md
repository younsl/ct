# Changelog

Notable changes to the imds-bridge app (the container image), newest first. Entries are headed by the image version and its release date (KST).

## 0.1.0

Initial release.

- Serves EKS Pod Identity credentials through an instance metadata endpoint on loopback (IMDSv2 token, role listing, role credentials).
- Background refresh from the container credentials endpoint, with the reported expiration pulled forward by a configurable margin.
- Health, readiness, and Prometheus metrics on a separate listener.
