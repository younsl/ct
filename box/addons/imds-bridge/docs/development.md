# Development

```bash
make build          # debug build
make test           # unit tests
make emulator-test  # Harbor registry simulation against AWS Emulator (docker or podman)
make coverage       # enforce minimum line coverage (70%, cargo-llvm-cov)
make lint           # rustfmt check + clippy (pedantic and nursery as warnings, denied in CI)
make zigbuild       # static musl binaries for linux/amd64 and linux/arm64 (cargo-zigbuild)
make docker-build   # multi-arch scratch image from the zigbuild binaries
```

## AWS Emulator simulation

`make emulator-test` runs tests/aws-emulator/run.sh, which rebuilds the pod this component was written for, with no AWS account and no cluster. Four containers share one network namespace, the way containers of one pod do.

| Container | Role |
| --- | --- |
| [AWS Emulator](https://github.com/aws/aws-emulator) | S3 and STS. It rejects any request that is not signed with credentials it issued. |
| Stand-in Pod Identity Agent | tests/aws-emulator/agent.py. Serves STS session credentials in the agent response format, only to a request that carries the token file content. |
| imds-bridge | The image built from the Dockerfile of this component, configured through the same two variables the Pod Identity webhook injects. |
| Harbor registry | The unmodified goharbor/registry-photon image (2.16.0 by default, override with `REGISTRY_IMAGE`). Its config has no access key, and its only AWS setting is the instance metadata endpoint variable. |

The script then checks the following, and fails on the first one that does not hold.

1. An image pushed through the registry is pulled back byte for byte, its layer is served through a presigned redirect to S3, and the layer object exists in the bucket.
2. The registry asked imds-bridge for the role credentials.
3. After new credentials are minted behind the agent, imds-bridge loads them within its refresh interval.
4. Once the reported expiration passes (15 seconds, forced by a large expiry margin), the registry comes back for credentials by itself, and a second push and pull succeeds.
5. imds-bridge logged neither the secret key nor the token.

### Limits

- The Pod Identity Agent is a stand-in, so the Pod Identity webhook, the association, and the real agent at its link-local address are not exercised. Run the same Pod on EKS to cover them.
- Only the registry container is simulated. Harbor core, jobservice, and registryctl are not started, so garbage collection through registryctl is not covered.
- The emulator certificate only covers loopback, which is why every container joins the network namespace of the emulator instead of a shared bridge network.

## Conclusion

`make lint`, `make test`, and `make coverage` are the checks a change has to pass, and the pre-commit hook runs them for changed crates. Run `make emulator-test` after touching src/imds.rs or src/credential/, since it is the only check that runs a real AWS SDK client against the endpoint.
