# Development

Build, test, and package the addon from a local checkout.

## Build

```bash
make build          # debug binary into target/debug/
make test           # unit tests
make emulator-test  # EC2 and SSM tests against AWS Emulator (docker or podman)
make coverage       # enforce minimum line coverage (70%, cargo-llvm-cov)
make lint           # rustfmt check + clippy (pedantic and nursery as warnings, denied in CI)
make zigbuild       # static musl binaries for linux/amd64 and linux/arm64 (cargo-zigbuild)
make docker-build   # multi-arch scratch image from the zigbuild binaries
make crd            # regenerate both chart CRDs from src/autoscaler/*/crd.rs
```

## AWS Emulator

The EC2 side of the instance volume path runs against [AWS Emulator](https://github.com/aws/aws-emulator) through the real AWS SDK clients, with no AWS account. `make emulator-test` builds the emulator image from tests/aws-emulator/Dockerfile (the RPM is verified against the AWS publishing key), starts it on 127.0.0.1:9297, and runs the ignored tests in src/aws_emulator.rs. They cover instance discovery, the `modifying`, `optimizing`, `completed` states of a root volume resize, the `IncorrectModificationState` refusal while one is in progress, throughput and IOPS changes, SSM Run Command results, and a full reconcile pass that grows a tagged instance.

The emulated guest shell has no `df`, `growpart`, or `xfs_growfs`, so the reconcile test scripts the filesystem side and the measure and resize scripts themselves are not run. The tests refuse to start unless `AWS_ENDPOINT_URL` points at a loopback address, so they never reach a real account.

## Conclusion

The Makefile covers the whole local loop, from `make build` and `make test` to the static multi-arch scratch image built by `make zigbuild` and `make docker-build`. Run `make crd` after changing a type in src/autoscaler/*/crd.rs, and run `make lint` and `make coverage` before pushing, since clippy warnings are denied in CI and line coverage must stay at or above 70%.
