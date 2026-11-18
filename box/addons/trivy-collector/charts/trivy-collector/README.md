# trivy-collector

![Version: 0.16.0](https://img.shields.io/badge/Version-0.16.0-informational?style=flat-square) ![Type: application](https://img.shields.io/badge/Type-application-informational?style=flat-square) ![AppVersion: 1.8.3](https://img.shields.io/badge/AppVersion-1.8.3-informational?style=flat-square)

Multi-cluster Trivy report collector and viewer

**Homepage:** <https://github.com/younsl/ct>

## Installation

### List available versions

This chart is distributed via OCI registry, so you need to use [crane](https://github.com/google/go-containerregistry/blob/main/cmd/crane/README.md) instead of `helm search repo` to discover available versions:

```console
crane ls ghcr.io/younsl/charts/trivy-collector
```

If you need to install crane on macOS, you can easily install it using [Homebrew](https://brew.sh/), the package manager.

```bash
brew install crane
```

### Install the chart

Install the chart with the release name `trivy-collector`:

```console
helm install trivy-collector oci://ghcr.io/younsl/charts/trivy-collector
```

Install with custom values:

```console
helm install trivy-collector oci://ghcr.io/younsl/charts/trivy-collector -f values.yaml
```

Install a specific version:

```console
helm install trivy-collector oci://ghcr.io/younsl/charts/trivy-collector --version 0.16.0
```

### Install from local chart

Download trivy-collector chart and install from local directory:

```console
helm pull oci://ghcr.io/younsl/charts/trivy-collector --untar --version 0.16.0
helm install trivy-collector ./trivy-collector
```

The `--untar` option downloads and unpacks the chart files into a directory for easy viewing and editing.

## Upgrade

```console
helm upgrade trivy-collector oci://ghcr.io/younsl/charts/trivy-collector
```

## Uninstall

```console
helm uninstall trivy-collector
```

## Configuration

The following table lists the configurable parameters and their default values.

## Values

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| crds | object | `{"additionalLabels":{},"annotations":{},"install":true,"keep":true}` | CustomResourceDefinitions for alert rules |
| crds.install | bool | `true` | Install and upgrade CRDs |
| crds.keep | bool | `true` | Keep CRDs on chart uninstall |
| crds.annotations | object | `{}` | Additional annotations to be added to all CRDs |
| crds.additionalLabels | object | `{}` | Additional labels to be added to all CRDs |
| nameOverride | string | `""` | Override the name of the chart |
| fullnameOverride | string | `""` | Override the full name of the chart |
| replicaCount | int | `1` | Deprecated, use server.replicaCount. The scraper always runs one replica |
| revisionHistoryLimit | int | `10` | Number of old ReplicaSets to retain for rollback |
| image | object | `{"pullPolicy":"IfNotPresent","registry":"ghcr.io","tag":""}` | Image settings shared by both components. Repository and digest are set per component |
| image.registry | string | `"ghcr.io"` | Container image registry host |
| image.pullPolicy | string | `"IfNotPresent"` | Image pull policy |
| image.tag | string | `""` | Image tag for both components (defaults to chart appVersion) |
| imagePullSecrets | list | `[]` | Image pull secrets for private registries |
| serviceAccount | object | `{"annotations":{},"automount":true,"create":true,"name":""}` | ServiceAccount configuration |
| serviceAccount.create | bool | `true` | Create a ServiceAccount |
| serviceAccount.automount | bool | `true` | Automount service account token |
| serviceAccount.annotations | object | `{}` | Annotations to add to the ServiceAccount |
| serviceAccount.name | string | `""` | Name of the ServiceAccount (auto-generated if empty) |
| podAnnotations | object | `{}` | Annotations to add to the pod |
| podLabels | object | `{}` | Labels to add to the pod |
| podSecurityContext | object | `{"fsGroup":1000,"runAsGroup":1000,"runAsNonRoot":true,"runAsUser":1000}` | Pod security context configuration |
| podSecurityContext.runAsNonRoot | bool | `true` | Run as non-root user |
| podSecurityContext.runAsUser | int | `1000` | User ID to run as |
| podSecurityContext.runAsGroup | int | `1000` | Group ID to run as |
| podSecurityContext.fsGroup | int | `1000` | Filesystem group ID |
| securityContext | object | `{"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]},"readOnlyRootFilesystem":true}` | Container security context configuration |
| securityContext.allowPrivilegeEscalation | bool | `false` | Disallow privilege escalation |
| securityContext.capabilities | object | `{"drop":["ALL"]}` | Capabilities to drop |
| securityContext.readOnlyRootFilesystem | bool | `true` | Mount root filesystem as read-only |
| clusterName | string | `"local"` | Cluster name identifier recorded on the Hub's own reports. |
| scraper | object | `{"affinity":{},"collect":{"sbomReports":true,"vulnerabilityReports":true},"extraPodSpec":{},"image":{"digest":"","repository":"younsl/trivy-collector-scraper"},"namespaces":[],"nodeSelector":{},"readinessProbe":{"failureThreshold":60,"httpGet":{"path":"/readyz","port":"health"},"initialDelaySeconds":5,"periodSeconds":10,"timeoutSeconds":5},"resizePolicy":[{"resourceName":"cpu","restartPolicy":"NotRequired"},{"resourceName":"memory","restartPolicy":"RestartContainer"}],"resources":{"limits":{"ephemeral-storage":"2Gi","memory":"512Mi"},"requests":{"cpu":"50m","ephemeral-storage":"2Gi","memory":"256Mi"}},"serviceMonitor":{"additionalLabels":{},"annotations":{},"enabled":false,"interval":"30s","scrapeTimeout":""},"storage":{"medium":"","mountPath":"/data","sizeLimit":"2Gi"},"strategy":{"rollingUpdate":{"maxSurge":1,"maxUnavailable":0},"type":"RollingUpdate"},"tolerations":[],"topologySpreadConstraints":[],"watchLocal":true}` | Scraper pod: watchers, the report database, the internal API, and alert evaluation. Runs one replica |
| scraper.image | object | `{"digest":"","repository":"younsl/trivy-collector-scraper"}` | Scraper image. Registry, tag, and pull policy come from `image`. |
| scraper.image.repository | string | `"younsl/trivy-collector-scraper"` | Image repository path without registry prefix |
| scraper.image.digest | string | `""` | Image digest (`sha256:...`), appended as `tag@digest` |
| scraper.namespaces | list | `[]` | Namespaces the local watcher scans on the Hub cluster (empty = all) |
| scraper.watchLocal | bool | `true` | Watch the Hub cluster's own reports. Disable if Trivy Operator is not installed there |
| scraper.collect | object | `{"sbomReports":true,"vulnerabilityReports":true}` | Report kinds watched on every cluster. Disabling SbomReports shrinks the database most |
| scraper.collect.vulnerabilityReports | bool | `true` | Watch VulnerabilityReports. |
| scraper.collect.sbomReports | bool | `true` | Watch SbomReports. |
| scraper.storage | object | `{"medium":"","mountPath":"/data","sizeLimit":"2Gi"}` | emptyDir holding the report database, rebuilt from the watched clusters on every start |
| scraper.storage.mountPath | string | `"/data"` | Where the database lives inside the container. |
| scraper.storage.sizeLimit | string | `"2Gi"` | emptyDir sizeLimit. Covers the database plus WAL churn, keep it equal to the ephemeral-storage limit |
| scraper.storage.medium | string | `""` | emptyDir medium. "Memory" counts against the pod memory limit |
| scraper.strategy | object | `{"rollingUpdate":{"maxSurge":1,"maxUnavailable":0},"type":"RollingUpdate"}` | Deployment strategy. The outgoing pod serves reads until the new one is hydrated |
| scraper.readinessProbe | object | `{"failureThreshold":60,"httpGet":{"path":"/readyz","port":"health"},"initialDelaySeconds":5,"periodSeconds":10,"timeoutSeconds":5}` | Readiness probe. Fails until every cluster finishes its initial sync |
| scraper.resources | object | `{"limits":{"ephemeral-storage":"2Gi","memory":"512Mi"},"requests":{"cpu":"50m","ephemeral-storage":"2Gi","memory":"256Mi"}}` | Resource requests and limits. ephemeral-storage covers the emptyDir database |
| scraper.resizePolicy | list | `[{"resourceName":"cpu","restartPolicy":"NotRequired"},{"resourceName":"memory","restartPolicy":"RestartContainer"}]` | In-place resize policy. CPU resizes without a restart, memory restarts the container |
| scraper.nodeSelector | object | `{}` | Node selector. |
| scraper.tolerations | list | `[]` | Tolerations. |
| scraper.affinity | object | `{}` | Affinity rules. |
| scraper.extraPodSpec | object | `{}` | Extra Pod spec fields such as runtimeClassName. Fields the chart renders are rejected |
| scraper.topologySpreadConstraints | list | `[]` | Topology spread constraints. |
| scraper.serviceMonitor | object | `{"additionalLabels":{},"annotations":{},"enabled":false,"interval":"30s","scrapeTimeout":""}` | ServiceMonitor for scraper (requires Prometheus Operator). |
| scraper.serviceMonitor.enabled | bool | `false` | Create a ServiceMonitor for the scraper pod. |
| scraper.serviceMonitor.interval | string | `"30s"` | Scrape interval. |
| scraper.serviceMonitor.scrapeTimeout | string | `""` | Scrape timeout (defaults to Prometheus global when empty). |
| scraper.serviceMonitor.additionalLabels | object | `{}` | Additional labels to attach to the ServiceMonitor. |
| scraper.serviceMonitor.annotations | object | `{}` | Annotations to attach to the ServiceMonitor. |
| server | object | `{"affinity":{},"auth":{"mode":"none","rbac":{"defaultPolicy":"role:readonly","policy":"p, role:readonly, reports, get, allow\np, role:readonly, clusters, get, allow\np, role:readonly, stats, get, allow\np, role:readonly, tokens, get, allow\np, role:readonly, tokens, create, allow\np, role:admin, *, *, allow\ng, admin, role:admin\n"},"sso":{"clientId":{"key":"client-id","name":"","value":""},"clientSecret":{"key":"client-secret","name":"","value":""},"issuer":"","redirectUrl":"","scopes":["openid","profile","email","groups"]}},"externalUrl":"","extraPodSpec":{},"gateway":{"enabled":false,"hostnames":["trivy.example.com"],"name":"","parentRefs":[{"group":"gateway.networking.k8s.io","kind":"Gateway","name":"main-gateway","namespace":"gateway-system","sectionName":"https"}],"rules":[{"backendRefs":[{"group":"","kind":"Service","name":"","port":3000,"weight":1}],"filters":[],"matches":[{"path":{"type":"PathPrefix","value":"/"}}]}]},"image":{"digest":"","repository":"younsl/trivy-collector-server"},"mcp":{"allowedHosts":[],"enabled":false,"stateless":false},"nodeSelector":{},"podDisruptionBudget":{"enabled":true,"maxUnavailable":1,"minAvailable":""},"port":3000,"replicaCount":1,"resizePolicy":[{"resourceName":"cpu","restartPolicy":"NotRequired"},{"resourceName":"memory","restartPolicy":"RestartContainer"}],"resources":{"limits":{"memory":"96Mi"},"requests":{"cpu":"20m","memory":"48Mi"}},"serviceMonitor":{"additionalLabels":{},"annotations":{},"enabled":false,"interval":"30s","scrapeTimeout":""},"tolerations":[],"topologySpreadConstraints":[]}` | Server pod: UI, REST API, and MCP server. Stateless, scale freely |
| server.image | object | `{"digest":"","repository":"younsl/trivy-collector-server"}` | Server image. Registry, tag, and pull policy come from `image`. |
| server.image.repository | string | `"younsl/trivy-collector-server"` | Image repository path without registry prefix |
| server.image.digest | string | `""` | Image digest (`sha256:...`), appended as `tag@digest` |
| server.replicaCount | int | `1` | Deprecated, use server.replicaCount. The scraper always runs one replica |
| server.podDisruptionBudget | object | `{"enabled":true,"maxUnavailable":1,"minAvailable":""}` | PodDisruptionBudget for the server, created only with more than one replica |
| server.podDisruptionBudget.enabled | bool | `true` | Create a PodDisruptionBudget. |
| server.podDisruptionBudget.minAvailable | string | `""` | Minimum available pods. Takes precedence when set. |
| server.podDisruptionBudget.maxUnavailable | int | `1` | Maximum unavailable pods. |
| server.port | int | `3000` | HTTP server port |
| server.externalUrl | string | `""` | Base URL for notification deep links. Empty derives it from gateway.hostnames |
| server.resources | object | `{"limits":{"memory":"96Mi"},"requests":{"cpu":"20m","memory":"48Mi"}}` | Resource requests and limits |
| server.resizePolicy | list | `[{"resourceName":"cpu","restartPolicy":"NotRequired"},{"resourceName":"memory","restartPolicy":"RestartContainer"}]` | In-place resize policy. CPU resizes without a restart, memory restarts the container |
| server.nodeSelector | object | `{}` | Node selector. |
| server.tolerations | list | `[]` | Tolerations. |
| server.affinity | object | `{}` | Affinity rules. |
| server.extraPodSpec | object | `{}` | Extra Pod spec fields such as runtimeClassName. Fields the chart renders are rejected |
| server.topologySpreadConstraints | list | `[]` | Topology spread constraints. |
| server.mcp | object | `{"allowedHosts":[],"enabled":false,"stateless":false}` | Embedded MCP server at `/mcp`, gated by the REST API's auth and RBAC |
| server.mcp.enabled | bool | `false` | Mount the MCP endpoint at `/mcp`. |
| server.mcp.allowedHosts | list | `[]` | Allowed `Host` values for `/mcp`. Empty disables the check, required for Service DNS access |
| server.mcp.stateless | bool | `false` | Serve MCP without sessions. Set true when server.replicaCount > 1. |
| server.serviceMonitor | object | `{"additionalLabels":{},"annotations":{},"enabled":false,"interval":"30s","scrapeTimeout":""}` | ServiceMonitor for server (requires Prometheus Operator). |
| server.serviceMonitor.enabled | bool | `false` | Create a ServiceMonitor for the server pod. |
| server.serviceMonitor.interval | string | `"30s"` | Scrape interval. |
| server.serviceMonitor.scrapeTimeout | string | `""` | Scrape timeout (defaults to Prometheus global when empty). |
| server.serviceMonitor.additionalLabels | object | `{}` | Additional labels to attach to the ServiceMonitor. |
| server.serviceMonitor.annotations | object | `{}` | Annotations to attach to the ServiceMonitor. |
| server.gateway | object | `{"enabled":false,"hostnames":["trivy.example.com"],"name":"","parentRefs":[{"group":"gateway.networking.k8s.io","kind":"Gateway","name":"main-gateway","namespace":"gateway-system","sectionName":"https"}],"rules":[{"backendRefs":[{"group":"","kind":"Service","name":"","port":3000,"weight":1}],"filters":[],"matches":[{"path":{"type":"PathPrefix","value":"/"}}]}]}` | Gateway API HTTPRoute, the chart's only ingress path |
| server.gateway.enabled | bool | `false` | Enable HTTPRoute |
| server.gateway.name | string | `""` | HTTPRoute name (defaults to fullname) |
| server.gateway.parentRefs | list | `[{"group":"gateway.networking.k8s.io","kind":"Gateway","name":"main-gateway","namespace":"gateway-system","sectionName":"https"}]` | Parent Gateway references |
| server.gateway.hostnames | list | `["trivy.example.com"]` | Hostnames for the route |
| server.gateway.rules | list | `[{"backendRefs":[{"group":"","kind":"Service","name":"","port":3000,"weight":1}],"filters":[],"matches":[{"path":{"type":"PathPrefix","value":"/"}}]}]` | HTTP route rules |
| server.gateway.rules[0].filters | list | `[]` | HTTPRoute filters, rendered verbatim |
| server.gateway.rules[0].backendRefs | list | `[{"group":"","kind":"Service","name":"","port":3000,"weight":1}]` | HTTPBackendRefs rendered verbatim. Empty `name` uses the server Service |
| server.auth | object | `{"mode":"none","rbac":{"defaultPolicy":"role:readonly","policy":"p, role:readonly, reports, get, allow\np, role:readonly, clusters, get, allow\np, role:readonly, stats, get, allow\np, role:readonly, tokens, get, allow\np, role:readonly, tokens, create, allow\np, role:admin, *, *, allow\ng, admin, role:admin\n"},"sso":{"clientId":{"key":"client-id","name":"","value":""},"clientSecret":{"key":"client-secret","name":"","value":""},"issuer":"","redirectUrl":"","scopes":["openid","profile","email","groups"]}}` | Authentication configuration (server mode only) |
| server.auth.mode | string | `"none"` | Authentication mode: "none" (anonymous) or "keycloak" (OIDC) |
| server.auth.rbac | object | `{"defaultPolicy":"role:readonly","policy":"p, role:readonly, reports, get, allow\np, role:readonly, clusters, get, allow\np, role:readonly, stats, get, allow\np, role:readonly, tokens, get, allow\np, role:readonly, tokens, create, allow\np, role:admin, *, *, allow\ng, admin, role:admin\n"}` | RBAC configuration (ArgoCD-style CSV policy) |
| server.auth.rbac.policy | string | See values | RBAC policy CSV (ArgoCD format) |
| server.auth.rbac.defaultPolicy | string | `"role:readonly"` | Default RBAC policy for authenticated users without explicit group bindings |
| server.auth.sso | object | `{"clientId":{"key":"client-id","name":"","value":""},"clientSecret":{"key":"client-secret","name":"","value":""},"issuer":"","redirectUrl":"","scopes":["openid","profile","email","groups"]}` | SSO configuration (only used when auth.mode is "keycloak") |
| server.auth.sso.issuer | string | `""` | OIDC issuer URL (e.g., https://keycloak.example.com/realms/trivy) |
| server.auth.sso.clientId | object | `{"key":"client-id","name":"","value":""}` | OIDC client ID configuration |
| server.auth.sso.clientId.value | string | `""` | Plaintext client ID value (used if set, takes precedence over secret reference) |
| server.auth.sso.clientId.name | string | `""` | Secret name containing client ID |
| server.auth.sso.clientId.key | string | `"client-id"` | Secret key for client ID |
| server.auth.sso.clientSecret | object | `{"key":"client-secret","name":"","value":""}` | OIDC client secret configuration |
| server.auth.sso.clientSecret.value | string | `""` | Plaintext client secret value (used if set, takes precedence over secret reference) |
| server.auth.sso.clientSecret.name | string | `""` | Secret name containing client secret |
| server.auth.sso.clientSecret.key | string | `"client-secret"` | Secret key for client secret |
| server.auth.sso.redirectUrl | string | `""` | OIDC redirect URL (full callback URL, e.g., https://trivy.example.com/auth/callback) |
| server.auth.sso.scopes | list | `["openid","profile","email","groups"]` | OIDC scopes |
| service | object | `{"annotations":{},"port":3000,"type":"ClusterIP"}` | Service configuration |
| service.type | string | `"ClusterIP"` | Service type |
| service.port | int | `3000` | Service port |
| service.annotations | object | `{}` | Annotations to add to the Service |
| internal | object | `{"existingSecret":"","networkPolicy":{"enabled":true,"extraFrom":[]},"port":8081,"secretKey":"token","token":""}` | Internal API from the scraper to the server pods. Never exposed outside the namespace |
| internal.port | int | `8081` | Port the scraper serves the internal read API on. |
| internal.token | string | `""` | Shared internal token. Empty generates one on install, set it explicitly under GitOps |
| internal.existingSecret | string | `""` | Use an existing Secret for the token instead of managing one. |
| internal.secretKey | string | `"token"` | Key inside the token Secret. |
| internal.networkPolicy | object | `{"enabled":true,"extraFrom":[]}` | NetworkPolicy limiting the internal port to the server pods |
| internal.networkPolicy.enabled | bool | `true` | Create the NetworkPolicy |
| internal.networkPolicy.extraFrom | list | `[]` | Additional `from` selectors allowed to reach the internal port. |
| health | object | `{"port":8080}` | Health check configuration |
| health.port | int | `8080` | Health check server port |
| logging | object | `{"format":"json","level":"info"}` | Logging configuration |
| logging.format | string | `"json"` | Log format: "json" or "pretty" |
| logging.level | string | `"info"` | Log level: trace, debug, info, warn, error |
| livenessProbe | object | `{"failureThreshold":3,"httpGet":{"path":"/healthz","port":"health"},"initialDelaySeconds":10,"periodSeconds":30,"timeoutSeconds":5}` | Liveness probe configuration |
| livenessProbe.initialDelaySeconds | int | `10` | Initial delay before starting probes |
| livenessProbe.periodSeconds | int | `30` | Probe interval |
| livenessProbe.timeoutSeconds | int | `5` | Probe timeout |
| livenessProbe.failureThreshold | int | `3` | Number of failures before marking unhealthy |
| readinessProbe | object | `{"failureThreshold":3,"httpGet":{"path":"/readyz","port":"health"},"initialDelaySeconds":5,"periodSeconds":10,"timeoutSeconds":5}` | Readiness probe configuration |
| readinessProbe.initialDelaySeconds | int | `5` | Initial delay before starting probes |
| readinessProbe.periodSeconds | int | `10` | Probe interval |
| readinessProbe.timeoutSeconds | int | `5` | Probe timeout |
| readinessProbe.failureThreshold | int | `3` | Number of failures before marking not ready |
| dnsPolicy | string | "" | DNS policy for the pod (ClusterFirst, ClusterFirstWithHostNet, Default, None) |
| dnsConfig | object | {} | DNS configuration for the pod |
| migration | object | `{"exportState":{"affinity":{},"backoffLimit":2,"dbPath":"/data/trivy.db","dryRun":false,"enabled":false,"existingClaim":"","extraPodSpec":{},"nodeSelector":{},"resources":{"limits":{"memory":"128Mi"},"requests":{"cpu":"50m","memory":"64Mi"}},"tolerations":[],"ttlSecondsAfterFinished":3600}}` | One-shot export of API tokens and notes from a legacy PVC into a Secret and a ConfigMap |
| migration.exportState.enabled | bool | `false` | Run the export as a pre-install/pre-upgrade hook Job. |
| migration.exportState.existingClaim | string | `""` | Name of the existing PVC holding the legacy database. |
| migration.exportState.dbPath | string | `"/data/trivy.db"` | Path to the legacy database inside that volume. |
| migration.exportState.dryRun | bool | `false` | Report what would be written without writing it. |
| migration.exportState.backoffLimit | int | `2` | Job backoff limit. |
| migration.exportState.ttlSecondsAfterFinished | int | `3600` | Seconds to keep the finished Job. |
| migration.exportState.nodeSelector | object | `{}` | Node selector. Defaults to the scraper's, since the legacy PVC is ReadWriteOnce |
| migration.exportState.tolerations | list | `[]` | Tolerations. Defaults to the scraper's |
| migration.exportState.affinity | object | `{}` | Affinity rules. Defaults to the scraper's. |
| migration.exportState.extraPodSpec | object | `{}` | Extra Pod spec fields such as runtimeClassName. Fields the chart renders are rejected |
| migration.exportState.resources | object | `{"limits":{"memory":"128Mi"},"requests":{"cpu":"50m","memory":"64Mi"}}` | Resource requests and limits. |
| extraObjects | list | [] | Extra Kubernetes objects to deploy alongside the chart |

## Source Code

* <https://github.com/younsl/ct/tree/main/box/addons/trivy-collector>

## Maintainers

| Name | Email | Url |
| ---- | ------ | --- |
| younsl | <cysl@kakao.com> | <https://github.com/younsl> |

## License

This chart is licensed under the Apache License 2.0. See the [LICENSE](https://github.com/younsl/ct/blob/main/LICENSE) file for details.

## Contributing

This repository does not accept external contributions. Pull requests and issues are disabled.

----------------------------------------------
Autogenerated from chart metadata using [helm-docs v1.14.2](https://github.com/norwoodj/helm-docs/releases/v1.14.2)
