# Development

## Workspace layout

trivy-collector is a Cargo workspace. Two binaries ship as two images, and the crate graph decides what each one links: a crate a binary does not depend on is code its image does not carry.

| Crate | Holds | Linked by |
|---|---|---|
| `core` | Trivy report types, the `ReportStore` contract, the internal API wire format, authored-state formats, the Hub cluster registry | both |
| `alertrule-crd` | The `AlertRule` custom resource types and `crdgen` | both |
| `alerts` | Rule matching, evaluation, readiness controller, Slack delivery | both |
| `runtime` | CLI basics, logging, health and metrics listener, Kubernetes client, lifecycle, build info | both |
| `database` | The SQLite report store | scraper |
| `scraper` | Report watchers, internal API, `export-state`, scraper config and metrics. Builds `trivy-collector-scraper` | scraper image |
| `server` | Web UI, REST API, MCP server, authentication, remote store, notes, tokens, server config and metrics. Builds `trivy-collector-server` | server image |

`cargo tree -e normal -p trivy-collector-server` must not list `trivy-collector-database` or `sqlx-sqlite`, and the scraper tree must not list `trivy-collector-server`, `rmcp`, or `rust-embed`.

## Build and run

```bash
make build          # both binaries, debug
make release        # both binaries, release
make frontend       # build the UI into static/, embedded by the server
make dev-scraper    # scraper: watchers, database, internal API on :8081
make dev            # server: UI on :3000, reads from the scraper on :8081
make dev-all        # both in one process group, Ctrl+C stops both
make crd            # regenerate the chart CRD from crates/alertrule-crd
```

The scraper and the server must share `INTERNAL_TOKEN`. `make dev-all` sets it for both.

## Checks

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo llvm-cov --workspace --fail-under-lines 70
```

Run them at the workspace root. The release workflow does the same, so a library crate's tests gate both images.

## Images

```bash
make docker-build   # both images for the native arch
make podman-build   # the same with Podman
```

Each image has its own Dockerfile, `crates/scraper/Dockerfile` and `crates/server/Dockerfile`, and its own version label. A release triggers on push to `main` when that label changes, and publishes `ghcr.io/younsl/trivy-collector-scraper` or `ghcr.io/younsl/trivy-collector-server` for `linux/amd64` and `linux/arm64`.

## Conclusion

The workspace splits by what each binary needs, so the server image carries no database and the scraper image carries no UI or API. Run fmt, clippy, tests, and coverage at the workspace root, and bump each Dockerfile's version label to release its image.
