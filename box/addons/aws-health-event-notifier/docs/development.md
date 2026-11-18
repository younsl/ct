# Development

```bash
make build          # Debug build
make test           # Run tests
make test-emulator  # Run the STS, IAM, and AWS Health tests against a local AWS Emulator
make lint           # Clippy
make release        # Release build
make local-run      # Run daemon locally (cold-start suppress ON)
```

The crate is a library (`src/lib.rs`) with a thin binary on top, so integration tests in `tests/` reach the same modules the binary runs.

## AWS Emulator tests

`tests/aws_emulator.rs` calls the real STS, IAM, and AWS Health client code against [AWS Emulator](https://github.com/aws/aws-emulator) instead of AWS. The tests are ignored by plain `cargo test`, so the coverage gate is unchanged. `make test-emulator` builds the emulator image from `tests/aws-emulator/Dockerfile`, starts it with Podman or Docker (set `CONTAINER_ENGINE` to choose), runs the tests, and removes the container. It needs no AWS account and no AWS CLI, and the emulator runs with telemetry off.

| Test | Covers |
| --- | --- |
| `account_identity_resolves_the_id_and_follows_the_alias` | `GetCallerIdentity` and `ListAccountAliases`: account ID alone, then with an alias, then after the alias is deleted |
| `event_listing_reports_the_missing_support_plan` | `DescribeEvents` for the poll and reminder queries, with and without service and category filters |
| `event_hydration_reports_the_missing_support_plan` | `DescribeEventDetails` for an event known only by ARN |
| `catalog_lookups_report_the_missing_support_plan` | `DescribeEventTypes` by service code and by event type code |
| `empty_catalog_lookups_make_no_api_call` | An empty filter list skips the catalog lookup |
| `startup_aborts_with_the_service_error_code` | `server::run` with a service filter stops at catalog validation and names the cause |

### Emulator limits

- The emulated account has no Business or Enterprise Support plan, and the emulator has no setting to grant one, so every AWS Health operation returns `SubscriptionRequiredException`. This is what a real account without the plan sees. The emulator tests therefore cover request signing, endpoint resolution, and the failure path of each call, while the unit tests in `src/aws/health.rs` and `src/poller.rs` cover the success paths with a replayed HTTP transport.
- The emulator has no route to inject AWS Health events, so event delivery to Slack is not exercised against it.
- The emulator keeps one account alias for the whole process, so the alias checks live in a single test.

## Conclusion

`make test`, `make fmt`, and `make lint` are the checks a change has to pass, and the pre-commit hook runs them for changed crates. Run `make test-emulator` after touching `src/aws/`.
