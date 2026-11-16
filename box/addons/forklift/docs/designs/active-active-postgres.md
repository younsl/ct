# Active-active API pods on PostgreSQL

| Status | Category |
| --- | --- |
| implemented, unreleased (see [Implementation status](#implementation-status)) | storage, topology |

This design lets every forklift pod serve requests at once by adding PostgreSQL as an opt-in metadata backend, and moves background jobs to a single worker in the style of a compactor. SQLite stays the default and keeps today's leader-based layouts unchanged.

## Why

forklift scales to one active pod. Metadata lives in embedded SQLite, which accepts one writer, so a Lease elects a leader and only the leader receives traffic. The standby adds availability, not capacity.

Blobs are not the limit. They are content addressed by SHA-256 and immutable, so with the s3 backend any pod can read and write them without coordination. Only the metadata store prevents a second active pod.

## Goals

- Every API pod is Ready and serves reads and writes against the same storage.
- No external dependency for the default install. PostgreSQL is opt-in.
- Package client URLs and the `/api/v1` contract do not change.
- One binary and one image. Roles are selected by a flag.

## Non-goals

- A separate web pod. The embedded SPA stays in the API pod, see [Web pod](#web-pod).
- A hash ring or any pod-to-pod membership protocol, see [Why no hash ring](#why-no-hash-ring).
- Sharding metadata by repository.
- Replacing the SQLite layouts or the WAL shipping path.

## Decision

| Option | Result |
| --- | --- |
| PostgreSQL metadata plus a shared blob store | Chosen. Same shape as Nexus, Artifactory, Harbor and Grafana HA. |
| Standby serves reads, writes forwarded to the leader | Rejected. Write throughput stays at one pod, and replication lag breaks publish-then-fetch. |
| Per-repository SQLite shards owned through a hash ring | Rejected. `blobs.ref_count` is global, search and group repositories span shards, and one hot proxy repository still lands on one pod. |
| SQLite file on a shared RWX volume | Rejected. WAL mode needs shared memory on one host. |

Nexus and Artifactory both shipped clustering on an embedded database and later dropped it for an external one. No surveyed project runs active-active on embedded SQLite.

## Architecture

<p align="center"><img src="../assets/active-active-postgres/architecture.png" alt="PostgreSQL mode architecture" width="760"></p>

Source: [architecture.svg](../assets/active-active-postgres/architecture.svg).

```
SQLite mode (default, unchanged):
client  ->  Service  ->  leader pod (target=all)  ->  SQLite + blobs
                         standby pod              (takes over on failover)

PostgreSQL mode:
client  ->  Service  ->  api pod 1..N (target=api)  --+-->  PostgreSQL (metadata)
                                                      +-->  S3 or RWX volume (blobs)
                         worker pod (target=worker) --+
                         (Lease holder runs the background jobs)
```

### Installation mode

One chart value, `installationMode`, selects the topology. Today the topology is implied by the combination of `replicaCount`, `ha.enabled`, `replication.enabled`, and `storage.backend`, and PostgreSQL adds a fifth axis. Six combinations are valid out of dozens, and nothing rejects the rest before render.

Setting the mode alone gives a working install. The mode supplies the settings that depend on it, and the existing keys still apply.

| `installationMode` | Server pods | Worker | Metadata | Lease |
| --- | --- | --- | --- | --- |
| `standalone` | 1, whatever `replicaCount` says | same pod | SQLite | none |
| `activePassive` | `replicaCount`, raised to at least 2 | leader pod | SQLite | gates traffic |
| `activeActive` | `replicaCount` API pods | `worker.replicaCount` dedicated pods | PostgreSQL, bundled unless `database.type=external` | elects the worker that runs background jobs |
| `""` (default) | derived from the existing keys | | | |

The blob store is not part of the preset. It follows `storage.backend`, `seaweedfs.enabled` and `persistence`, so with chart defaults every mode uses the bundled SeaweedFS over S3. A subchart is switched by its own `enabled` value, which a mode cannot derive.

- The empty default keeps the current derivation, so existing installs render the same manifests without touching their values. This was checked by diffing renders of four existing topologies.
- `database.type` defaults to empty and resolves in `_helpers.tpl`: `internal` for `activeActive`, `sqlite` otherwise. It is the only new key with a mode-dependent default. `replicaCount` and the Lease are resolved by helpers that the templates call instead of reading the values directly.
- Invalid mode and database combinations fail in `values.schema.json` through `if` and `then` rules, so `helm lint` reports them before any template renders: `activeActive` with `database.type=sqlite`, `activePassive` with PostgreSQL, PostgreSQL or `standalone` with `replication.enabled`. Checks that need derived values (a shared blob store for several pods, a host for `external`) fail in the templates with a message.
- The mode exists only in the chart. It is translated into `FORKLIFT_TARGET` and the existing per-axis settings, so the binary keeps one setting per axis and runs outside the chart in any valid combination.
- The three `activePassive` storage layouts stay settings and not modes. Splitting them would duplicate `storage.backend` and `replication.enabled` and bring back mismatches.
- `ha.enabled` is ignored when a mode is set.
- Changing the mode must not change the database by accident. With `database.type` empty, going from `activeActive` to another mode resolves to SQLite, which would start forklift on an empty metadata store. The chart looks up the release's database StatefulSet and the password key in its Secret, and refuses the render until `database.type` is set either way. The lookup needs cluster access, so renders without it are not covered and should pin `database.type`.
- PostgreSQL without a mode is allowed. With `database.type=external` and `replicaCount: 2` both pods run target `all`: both serve, and the Lease picks which one runs the background jobs.

### Metadata backends

| Metadata | Blobs | Active pods | Notes |
| --- | --- | --- | --- |
| SQLite | PV or S3 | 1 | Today's layouts |
| PostgreSQL | S3 | N | Recommended |
| PostgreSQL | RWX volume | N | Works without S3 |
| PostgreSQL | RWO volume | 1 | Rejected at startup when replicas exceed one |

PostgreSQL mode needs a blob store that every pod shares. S3 is the usual choice and is not strictly required.

### Targets

A `--target` flag with an env fallback selects what a process runs.

| Target | Serves requests | Runs background jobs | Used by |
| --- | --- | --- | --- |
| `all` (default) | yes | yes, when it holds the Lease | SQLite mode, small PostgreSQL installs |
| `api` | yes | no | PostgreSQL mode |
| `worker` | not routed. It runs the same server, and no Service selects it | yes, when it holds the Lease | PostgreSQL mode |

Background jobs are the work the leadership callbacks own today: retention and cache eviction, blob garbage collection, the idle reaper, OCI upload and tag pruning, coverage scans, security insights, license and vulnerability scans, notifications, audit pruning, and applying declarative state (bootstrap admin, chart RBAC policy, seeded repositories). In PostgreSQL mode the Lease no longer gates readiness or routing. It only picks which worker runs these jobs.

### Why no hash ring

A hash ring assigns ownership of state that lives inside a pod. In PostgreSQL mode pods hold no state, so there is nothing to own.

| Ring duty in Loki or Mimir | Replacement here |
| --- | --- |
| Pick the pod that owns a key | Any pod serves any request |
| Prevent conflicting writes | Transactions, unique constraints, `ON CONFLICT` |
| Elect a singleton | The existing Lease |
| Spread jobs over several workers | A job table read with `FOR UPDATE SKIP LOCKED`, only if one worker stops being enough |

A ring becomes relevant again only if pod-local state returns, such as per-repository SQLite shards or a pod-local blob cache that wants cache-aware routing.

### Web pod

The SPA is static, already precompressed, and costs almost nothing to serve, so splitting it does not help capacity. Argo CD, Grafana, Zot, Nexus and Artifactory all keep the UI with the API. The split stays possible later since the coupling is one module (`crates/forklift/src/webui.rs`) and one fallback route, and the frontend calls only relative paths. [fe-be-pod-split.md](fe-be-pod-split.md) describes an older Next.js variant and is not part of this design.

## Required changes

### Store abstraction

The store keeps one implementation. `meta::Store` stays a concrete type, and its roughly 235 methods are written once against a connection facade that runs on both databases.

An earlier draft of this design had a `Store` trait with a SQLite and a PostgreSQL implementation. Measuring the code changed that: the methods use ten rusqlite calls in total (`execute`, `query_row`, `prepare`, `query_map`, `transaction`, `commit`, `optional`, `execute_batch`, `exists`, `last_insert_rowid`) and about twenty statements that only SQLite parses. Two implementations would have meant writing every method again and keeping both in step forever, which is the first risk in the table below. A facade over those ten calls removes it.

| Concern | How the facade handles it |
| --- | --- |
| API shape | `Connection`, `Transaction`, `Statement`, `Row`, `params!` and `OptionalExtension` mirror rusqlite, so store methods changed only their imports |
| Placeholders | `?` and `?N` are rewritten to `$N` for PostgreSQL |
| `LIKE` | Rewritten to `ILIKE`, since SQLite's is case-insensitive for ASCII |
| Parameter types | A value is encoded as the type PostgreSQL inferred for the parameter, which reproduces SQLite's affinity (an integer bound to a text parameter, a boolean stored as an integer) |
| Result types | `BOOL`, `INT4`, `NUMERIC` (what `SUM` over `BIGINT` returns) and text types decode into SQLite's storage classes |
| Errors | Missing row and constraint violation are classified the same on both, so `NotFound` and `Conflict` keep working |
| Transactions | Issued as `BEGIN`, `COMMIT`, `ROLLBACK` by the facade, with the advisory locks described under write contention |

Statements that differed were moved to SQL both databases accept where possible (`RETURNING id` for `last_insert_rowid`, `ON CONFLICT DO UPDATE` for `INSERT OR REPLACE`, table-qualified columns in upserts, a quoted `"offset"` column). Four sites branch on the backend: timestamp comparison (`julianday` against a `timestamptz` cast), JSON array expansion (`json_each` against `json_array_elements_text`), a planner hint, and a catalog query in a test.

The store bootstraps itself: it creates the configured schema when missing, then the tables, under a session advisory lock so pods starting together take turns. Only the database and the user have to exist beforehand.

PostgreSQL has its own migration list. The first file is the schema the 49 SQLite migrations reach, generated from a migrated SQLite database: integers are `BIGINT`, timestamps stay RFC 3339 text and booleans stay integers, so both backends bind and read the same Rust types. A schema change adds one file to each list.

The same test suite runs on both. `Store::open_temp` opens a fresh PostgreSQL schema per test when `FORKLIFT_TEST_POSTGRES_URL` is set, so every store, engine and API test exercises PostgreSQL without a second copy of the tests.

### Workspace layout

```
crates/
  common/          forklift-common    existing. Logging, shutdown signal, build version
  metadb/          forklift-metadb    new. Connection facade, both database drivers
  forklift/        forklift           existing. Server library and binary, targets all, api, worker
  forklift-mcp/    forklift-mcp       existing. Unchanged
```

| Crate | Depends on | Image |
| --- | --- | --- |
| `forklift-metadb` | rusqlite, tokio-postgres, rustls | none |
| `forklift` | `forklift-metadb`, `forklift-common` | `forklift` |
| `forklift-mcp` | `forklift-common` | `forklift-mcp` |

- The drivers live in `forklift-metadb`. Store methods reach a database only through the facade, and SQLite-only machinery (metadata sync, WAL shipping, PV replication, storage migration) asks for the SQLite connection explicitly with `as_sqlite`, which fails on PostgreSQL.
- The PostgreSQL driver is `tokio-postgres` driven synchronously on the blocking thread pool, the way the store already runs SQLite, with `tokio-postgres-rustls` for TLS. No openssl.
- Both backends are always compiled in. A Cargo feature per backend would produce two binaries and break the one-image goal.
- `meta` stays a module of `forklift`. Moving it to a crate of its own is a pure move with no behavior attached and can follow later.
- `forklift-metadb` carries no version, Dockerfile, or release entry, like `forklift-common`. It ships when the `forklift` image version label is bumped.

### Pod-local state

| State | Today | Change |
| --- | --- | --- |
| OCI chunked upload sessions | Bytes staged under `<data_dir>/oci-uploads/<session>` on the pod | With the s3 backend each chunk is stored as an object under `<prefix>/oci-uploads/<session>/<offset>` and the parts are stitched on finish. On an RWX volume the directory is already shared |
| UI upload staging | Staged as content-addressed blobs with a database lease | None |
| Token `last_used_at` | Written on every token-authenticated request | Written at most once per 5 minutes per token, like artifact access |
| Audit log and download counters | Batched (256 events or 5 seconds) | None |
| Scan and license queues | In memory, drained by leader jobs | Every pod drains its own queue. Backfill and rescans stay on the worker |
| Coverage scanner settings, mutes and last result | In memory, loaded at boot and on save | Every pod reloads them from the store once a minute, so a scan run by the worker shows up on API pods and a setting saved through one API pod reaches the worker |
| OSV lookup cache, regex and semaphores | In memory per pod | None. Concurrency limits become per pod, so the total is N times the setting |
| RBAC and repository config read by handlers | Read from the store | Any in-memory copy needs a short TTL or `LISTEN/NOTIFY` |
| Concurrent proxy cache fill of one artifact | Cannot happen with one active pod | Accepted. Blobs are content addressed and the index write is an upsert, so the only cost is a duplicate upstream fetch |

### Write contention between blobs and metadata

Two guards that hold today are local to one process and stop working with several active pods.

| Guard today | With several active pods |
| --- | --- |
| Atomic blob write (temp file, then rename or one PutObject to the digest key) | Holds. Two pods writing one digest write identical bytes |
| Bytes first, metadata second | Holds |
| `UNIQUE(repo_id, path)` with upsert | Holds |
| `ref_count = ref_count + ?` in one statement | Holds |
| 24 hour GC grace on `unreferenced_since` and `created_at` | Holds, not sufficient alone |
| Guarded record delete before the byte delete | Holds |
| `gc_mu` RwLock: four write paths take the read guard, the sweeper takes the write guard per digest | Broken. The lock lives in one process |
| SQLite's single writer serializing every write | Gone |

Two races follow.

- GC against upload, which loses data. The worker deletes the record of a digest that has been unreferenced past the grace period. An API pod then stores the same content, finds or writes the bytes, and inserts a fresh record with a reference. The worker then deletes the bytes, and a live artifact points at nothing.
- Concurrent publish of one path, which corrupts reference counts. Two pods overwrite the same `(repo_id, path)`, both read the old blob, and both decrement it. The old blob drops to zero while another path still references it and becomes a GC candidate.

PostgreSQL mode replaces the two broken guards.

| Safeguard | Replaces | How |
| --- | --- | --- |
| GC in one transaction | `gc_mu` write guard | The sweeper runs the guarded `DELETE`, deletes the bytes, then commits. The row lock covers the window between the two deletes, and a failed byte delete rolls the record back |
| Reserve after write, then verify | `gc_mu` read guard | Every blob write goes through `storage::ReservingStore`: store the bytes, upsert the blob row with `unreferenced_since` reset to now, then check the bytes still exist. A reservation that commits first makes the sweeper's guard miss. A sweeper that locks first makes the reservation wait until the bytes are gone, the check fails, and the request errors instead of recording a reference to nothing |
| Database-wide advisory lock | Single writer | Every multi-statement write transaction takes one exclusive `pg_advisory_xact_lock`, so transactions that relied on SQLite's writer for isolation keep it without an audit of each one |
| Per-path advisory lock | Single writer, hot path | `put_artifact` takes the database-wide lock in shared mode and an exclusive lock on a hash of `(repo_id, path)`. Different artifacts are written in parallel, and the same path or any other transaction is still excluded |

The reservation runs after the bytes are stored and not before, which keeps `BlobStore::put` unchanged and covers all twelve call sites through one wrapper. The cost is that the rare losing writer gets an error and retries, instead of re-uploading transparently.

Audit batches take no advisory lock, since they only insert rows and bump counters atomically.

As a backstop the worker compares every blob's count with the rows referencing it once an hour and exports the result as `forklift_blob_ref_drift`. An undercount is raised at once inside a database-wide locked transaction that recounts, since it is the direction that lets the sweeper delete bytes in use. An overcount only leaks storage and is reported for an operator.

The Lease is not a strict fence, so two workers can briefly both believe they lead. The guarded `DELETE` is atomic in PostgreSQL, the second worker matches no row, and it leaves the bytes alone.

### Not needed in PostgreSQL mode

Metadata sync (`objstore/metasync`, WAL shipping), PV replication, snapshot restore on promotion, S3 fencing, and leader pod labels are skipped. The failover data-loss window of the SQLite layouts does not apply, since durability is PostgreSQL's.

### Chart

- `installationMode` selects the topology and supplies the defaults below, see [Installation mode](#installation-mode).
- A `database.type` value selects the metadata backend: `sqlite`, `internal`, or `external`.
- A worker Deployment rendered in `activeActive`, with its own selector labels so the server Deployment's immutable selector is unchanged.
- In `activeActive` the Service selects all API pods and the leader label and headless Service are not rendered.
- Schema validation rejects PostgreSQL with `replication.enabled` and rejects more than one replica on an RWO volume.
- `values.schema.json` is updated in the same commit.

### Bundled PostgreSQL

The chart ships its own PostgreSQL templates under `templates/database/`, the way the Harbor chart does, instead of a subchart. A subchart ties the release to a third-party chart's image and licensing policy, and forklift needs only a small part of what such a chart offers.

| `database.type` | Rendered | Use |
| --- | --- | --- |
| `sqlite` | Nothing. Today's layouts | Default |
| `internal` | StatefulSet with one replica, PVC, Service, and a password key in the chart Secret | Evaluation and small installs |
| `external` | Nothing. forklift connects to `database.external.*` | Production |

- `internal` runs the official PostgreSQL image at a pinned version (`postgres:17.12`). An empty password generates one into the chart Secret and is kept across upgrades, like the session secret. Under a GitOps tool that renders without cluster access it must be set explicitly, since the server reads it only when its volume is first initialized.
- `internal` is one instance and is not highly available. With several API pods it is the single point of failure, so availability can be worse than the SQLite leader layouts. The chart notes say so.
- `external` reads the password from an existing Secret, or from `database.external.password` stored in the chart Secret, and exposes host, port, database, user, and TLS mode.
- The bundled server runs as the image's `postgres` user (999) with a read-only root filesystem and writable `emptyDir` mounts for its socket and temp directories.
- The bundled SeaweedFS stays a subchart, so the chart mixes both styles on purpose.

## Implementation status

| Step | State |
| --- | --- |
| Throttle `touch_token` | done |
| Connection facade (`forklift-metadb`) and store on it | done |
| PostgreSQL schema, pools, reconnect, migration lock | done |
| `--target` (`FORKLIFT_TARGET`) with api and worker | done |
| Blob reservation and single-transaction GC | done |
| OCI chunk staging in the bucket | done |
| Chart: `installationMode`, `database`, `worker`, bundled PostgreSQL templates | done |
| SQLite to PostgreSQL data migration (`forklift migrate-database`, chart Job) | done |
| Reference count check on the worker (`forklift_blob_ref_drift`), repairing undercounts | done |
| Coverage scanner state synced between pods | done. Each pod reloads settings and the last result from the store once a minute |
| `meta` as its own crate | not started, optional |
| Worker metrics scrape (headless Service and ServiceMonitor) | done |
| `cargo-zigbuild` static build of the new dependencies | verified for `aarch64-unknown-linux-musl` with zig 0.15.2. The release workflow itself has not run |
| Run on a cluster with the chart | verified on kind (Kubernetes 1.37), see Validation. Not yet run on a multi-node cluster or under load |

## Rollout

1. Release the image. SQLite installs behave as before, since nothing changes without `FORKLIFT_DATABASE_HOST`.
2. Release the chart with `installationMode` empty by default. Existing values render the same manifests.
3. Try `installationMode=activeActive` on a new install.
4. Move an existing install with `database.migration.enabled`, after a dry run.

## Risks

| Risk | Mitigation |
| --- | --- |
| Two implementations drift | There is one implementation. The remaining drift point is the two migration lists, which CI should cover by running the suite on PostgreSQL |
| Writes are serialized database-wide outside artifact puts | Accepted for now. Each transaction is short, and the hot path locks per path. Narrow more transactions if write latency shows it |
| PostgreSQL availability is now forklift's availability | `external` is the production path and is the operator's responsibility (managed service or an operator such as CloudNativePG). `internal` is documented as a single instance |
| Connection count grows with pods | Bounded pool per pod, PgBouncer for large installs |
| Query plans differ from SQLite on large indexes | Load test with a production-sized artifact index before release |
| A transaction that relied on the single writer for isolation | The database-wide advisory lock keeps that isolation for every transaction that did not opt out |
| Two artifact writes swapping the same two blobs deadlock | PostgreSQL aborts one, the request fails and the client retries. Needs two paths exchanging content at the same moment |

## Open questions

- Whether the chart always renders a dedicated worker in `activeActive`, as it does now, or lets small installs run target `all`.
- How the 70% line coverage gate counts `forklift-metadb`, whose PostgreSQL arm only runs when a server is available.
- Whether CI should start a PostgreSQL service and run the suite with `FORKLIFT_TEST_POSTGRES_URL`.
- Whether holding a transaction open across one blob delete per digest is acceptable on a slow object store, or whether the sweeper needs a tombstone state instead.

## Validation

Done locally against PostgreSQL 17:

- The whole `forklift` suite passes on both backends: 882 unit tests, 101 integration tests. SQLite-only tests (snapshots, change watches, the single write connection) skip on PostgreSQL.
- Three processes (two `api`, one `worker`) started together on an empty database all became ready, and only the worker reported leadership.
- A session cookie issued by one API process was accepted by the other, and an artifact published through one was served by the other.
- 40 concurrent overwrites of one path, split across two processes, left every `ref_count` equal to the number of artifact rows referencing the blob.
- A reservation issued while a reclaim transaction held the digest waited until the reclaim committed (`reservation_waits_for_an_in_flight_reclaim`).
- A chunked OCI push completed with every request seeing an empty upload directory (`oci_chunked_push_survives_changing_pods`).
- The chart renders the four existing topologies byte for byte as before, renders the new modes, and rejects seven invalid combinations with a message.

Done on a kind cluster with `installationMode=activeActive` (two API pods, one worker, bundled PostgreSQL, bundled SeaweedFS), using a statically linked image built with cargo-zigbuild:

- All five pods became ready with no restart. The forklift pods retried until PostgreSQL and the bucket accepted connections.
- The bundled PostgreSQL ran as uid 999 with a read-only root filesystem.
- 300 sequential and 400 concurrent requests through the Service were split between the two API pods (162 and 138, then 217 and 183). The worker received none.
- A session from one API pod was valid on the other, and an artifact published through one was served by the other from the bucket.
- 60 concurrent overwrites of one path through the Service left no reference count mismatch.
- A chunked OCI push completed with every request sent to the other pod than the one before, and again through the Service. This first failed: a pod that had not opened the session had no scratch directory for the chunk. Fixed, with the unit test changed to cover it.
- Deleting the worker moved the Lease to its replacement while 240 requests through the Service all succeeded.
- A rolling upgrade to a new image kept the data.
- `helm upgrade` with only `installationMode` changed was refused by the guard, and passed once `database.type` was set.

Still to do:

- Run on a multi-node cluster, where API pods sit on different nodes.
- Load test with a production-sized artifact index.
- Run the release workflow, which builds on Linux runners.
