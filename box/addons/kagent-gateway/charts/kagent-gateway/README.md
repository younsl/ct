# kagent-gateway

![Version: 0.6.1](https://img.shields.io/badge/Version-0.6.1-informational?style=flat-square) ![Type: application](https://img.shields.io/badge/Type-application-informational?style=flat-square) ![AppVersion: 0.4.1](https://img.shields.io/badge/AppVersion-0.4.1-informational?style=flat-square)

Posts Alertmanager alerts to Slack and replies in-thread with an analysis from a kagent agent over A2A

**Homepage:** <https://github.com/younsl/ct>

## Installation

### List available versions

This chart is distributed via OCI registry, so you need to use [crane](https://github.com/google/go-containerregistry/blob/main/cmd/crane/README.md) instead of `helm search repo` to discover available versions:

```console
crane ls ghcr.io/younsl/charts/kagent-gateway
```

If you need to install crane on macOS, you can easily install it using [Homebrew](https://brew.sh/), the package manager.

```bash
brew install crane
```

### Install the chart

Install the chart with the release name `kagent-gateway`:

```console
helm install kagent-gateway oci://ghcr.io/younsl/charts/kagent-gateway
```

Install with custom values:

```console
helm install kagent-gateway oci://ghcr.io/younsl/charts/kagent-gateway -f values.yaml
```

Install a specific version:

```console
helm install kagent-gateway oci://ghcr.io/younsl/charts/kagent-gateway --version 0.6.1
```

### Install from local chart

Download kagent-gateway chart and install from local directory:

```console
helm pull oci://ghcr.io/younsl/charts/kagent-gateway --untar --version 0.6.1
helm install kagent-gateway ./kagent-gateway
```

The `--untar` option downloads and unpacks the chart files into a directory for easy viewing and editing.

## Upgrade

```console
helm upgrade kagent-gateway oci://ghcr.io/younsl/charts/kagent-gateway
```

## Uninstall

```console
helm uninstall kagent-gateway
```

## Configuration

The following table lists the configurable parameters and their default values.

## Values

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| nameOverride | string | `""` | Override the chart name |
| fullnameOverride | string | `""` | Override the full release name |
| image.registry | string | `"ghcr.io"` | Image registry host |
| image.repository | string | `"younsl/kagent-gateway"` | Image repository without the registry |
| image.tag | string | `""` | Image tag. Empty uses the chart appVersion |
| image.digest | string | `""` | Image digest (`sha256:...`), pinned as `tag@digest` when set |
| image.pullPolicy | string | `"IfNotPresent"` | Image pull policy |
| imagePullSecrets | list | `[]` | Image pull secrets |
| crds.install | bool | `true` | Install and upgrade the AgentAccessPolicy CRD |
| crds.keep | bool | `true` | Keep the CRD (and every policy) on `helm uninstall` |
| crds.annotations | object | `{}` | Extra CRD annotations |
| crds.additionalLabels | object | `{}` | Extra CRD labels |
| replicaCount | int | `1` | Replicas. Deduplication is in memory per replica, so more than one can analyse an alert group twice |
| revisionHistoryLimit | int | `3` | Old ReplicaSets kept for rollback |
| strategy | object | `{"rollingUpdate":{"maxSurge":"25%","maxUnavailable":"25%"},"type":"RollingUpdate"}` | Deployment update strategy |
| strategy.rollingUpdate.maxSurge | string|int | `"25%"` | Max extra Pods during an update |
| strategy.rollingUpdate.maxUnavailable | string|int | `"25%"` | Max unavailable Pods during an update |
| slack.parentMode | string | `"lookup"` | `lookup` threads under the notification Alertmanager posted (needs the history and read scopes). `post` publishes the alert itself |
| slack.lookupWindow | string | `"15m"` | How far back `lookup` searches channel history |
| slack.lookupAttempts | int | `3` | `lookup` attempts, since the notification can land after the webhook |
| slack.investigatingReaction | string | `"telescope"` | Reaction on the alert while the agent investigates. Needs `reactions:write`. Empty disables it |
| slack.completedReaction | string | `"white_check_mark"` | Reaction that replaces `investigatingReaction` once the analysis is posted. Empty disables it |
| slack.defaultChannel | string | `""` | Channel used when the routing label is absent. A conversation ID skips the `channels:read` lookup |
| slack.channelLabel | string | `"slack_channel"` | Alert label that selects the channel |
| slack.channelMap | object | `{}` | Routing label value to channel, for values that are not channel names. Ignored under `chat.accessPolicies` |
| slack.maxTextLength | int | `8000` | Max characters per message, longer text is truncated |
| slack.apiURL | string | `"https://slack.com/api"` | Slack Web API base URL |
| slack.existingSecret | string | `""` | Existing Secret with the tokens. Empty creates one from `slack.token` and `slack.appToken` |
| slack.existingSecretKey | string | `"SLACK_BOT_TOKEN"` | Bot token key in the Secret |
| slack.token | string | `""` | Bot token (`xoxb-`), used when `slack.existingSecret` is empty |
| slack.existingSecretAppTokenKey | string | `"SLACK_APP_TOKEN"` | App-level token key in the Secret, read when `chat.enabled` |
| slack.appToken | string | `""` | App-level token (`xapp-`, `connections:write`) for Socket Mode, used when `slack.existingSecret` is empty |
| chat.enabled | bool | `false` | Answer `@bot` mentions in threads. Needs an app-level token, `app_mentions:read`, Socket Mode, and the `app_mention` event |
| chat.agent | string | `""` | Agent that answers mentions. Empty uses `kagent.agent` |
| chat.agentMap | object | `{}` | Channel (name or ID) to Agent. Unlisted channels use `chat.agent`. Ignored under `chat.accessPolicies` |
| chat.channels | list | `[]` | Channels allowed to invoke the bot. Empty allows every channel it is in. Ignored under `chat.accessPolicies` |
| chat.accessPolicies.enabled | bool | `false` | Route every mention and alert through AgentAccessPolicies and drop what none matches. Replaces the env routing maps and adds a Role in `kagent.namespace` |
| chat.instructions | string | `""` | Instructions appended to every mention prompt. Empty uses the built-in ones |
| chat.userID | string | `""` | X-User-Id for mention turns. Empty uses `kagent.userID`. Set it to keep agent memory of the two paths apart |
| chat.timeout | string | `"180s"` | Deadline for one turn including queueing. The kagent controller caps a turn at 3m |
| chat.sessionTTL | string | `"2h"` | How long a thread keeps its agent session after the last turn. `0s` starts every mention fresh |
| chat.statusInterval | string | `"10s"` | Refresh interval of an unchanged status (at most 60s for the typing indicator). Changes show within a second |
| chat.statusStyle | string | `"assistant"` | `assistant` shows Slack's typing indicator and posts the answer as one reply. `message` rewrites a status reply into the answer |
| chat.threadHint | string | `nil` | Ephemeral hint for a mention outside a thread. `null` uses the built-in text, empty drops it silently |
| chat.deniedHint | string | `nil` | Ephemeral hint for a mention in a channel not served. `null` uses the built-in text, empty drops it silently |
| chat.maxConcurrent | int | `2` | Max mention turns at once, separate from `analysis.maxConcurrent` |
| agentAccessPolicies | object | `{}` | AgentAccessPolicies by name: `enabled` (default `true`) and the CR `spec`. Needs `chat.accessPolicies.enabled` |
| kagent.url | string | `"http://kagent-controller.kagent:8083"` | kagent controller base URL |
| kagent.namespace | string | `"kagent"` | Namespace of the Agent resources |
| kagent.agent | string | `"alert-triage-agent"` | Agent used when the routing label maps to none |
| kagent.agentRoutingLabel | string | `""` | Alert label that selects the agent. Empty uses `slack.channelLabel` |
| kagent.agentRoutingMap | object | `{}` | Routing label value to Agent. Unlisted values use `kagent.agent`. Ignored under `chat.accessPolicies` |
| kagent.userID | string | `"gateway@kagent.dev"` | X-User-Id that owns the sessions the gateway creates |
| kagent.timeout | string | `"120s"` | Deadline for one analysis including queueing. The task is cancelled on expiry |
| kagent.requestTimeout | string | `"30s"` | Timeout of one controller call |
| kagent.pollInterval | string | `"5s"` | Task status poll interval, which bounds how fast progress shows |
| analysis.severities | string | `"critical"` | Comma-separated severities to analyse. Empty analyses every alert |
| analysis.label | string | `"analyze"` | Alert label that opts a rule in (`true`) or out (`false`) regardless of severity |
| analysis.resolved | bool | `false` | Analyse resolved notifications too |
| analysis.dedupeTTL | string | `"12h"` | Suppression after an analysis, covering the Alertmanager `repeat_interval`. `0s` disables it |
| analysis.maxAlerts | int | `5` | Max alerts per prompt |
| analysis.maxConcurrent | int | `2` | Max analyses at once |
| analysis.instructions | string | `""` | Instructions appended to every analysis prompt. Empty uses the built-in ones |
| webhook.path | string | `"/alert"` | Path Alertmanager posts to |
| webhook.existingSecret | string | `""` | Existing Secret with the bearer token. Empty creates one from `webhook.token` |
| webhook.existingSecretKey | string | `"WEBHOOK_BEARER_TOKEN"` | Bearer token key in the Secret |
| webhook.token | string | `""` | Bearer token required on the webhook. Empty disables authentication |
| log.level | string | `"info"` | Log level: debug, info, warn, error |
| log.format | string | `"json"` | Log format: json or text |
| extraEnv | list | `[]` | Extra container env entries |
| extraEnvFrom | list | `[]` | Extra container envFrom entries |
| ports.http | int | `8080` | Port for the webhook, `/healthz`, and `/readyz` |
| ports.metrics | int | `8081` | Port for `/metrics` |
| serviceAccount.create | bool | `true` | Create a ServiceAccount |
| serviceAccount.name | string | `""` | ServiceAccount name. Empty uses the full name |
| serviceAccount.annotations | object | `{}` | ServiceAccount annotations |
| serviceAccount.automountServiceAccountToken | bool | `false` | Automount the ServiceAccount token |
| serviceAccount.imagePullSecrets | list | `[]` | Image pull secrets on the ServiceAccount |
| service.enabled | bool | `true` | Create a Service for the webhook and metrics ports |
| service.type | string | `"ClusterIP"` | Service type |
| service.trafficDistribution | string | `""` | Service `trafficDistribution`, e.g. PreferClose. Empty omits it |
| serviceMonitor.enabled | bool | `false` | Create a ServiceMonitor |
| serviceMonitor.interval | string | `"30s"` | Scrape interval |
| serviceMonitor.scrapeTimeout | string | `"10s"` | Scrape timeout |
| serviceMonitor.labels | object | `{}` | Extra ServiceMonitor labels |
| serviceMonitor.honorLabels | bool | `false` | Keep the metric's labels when they collide with the target's |
| serviceMonitor.relabelings | list | `[]` | Relabelings applied before scraping |
| serviceMonitor.metricRelabelings | list | `[]` | Metric relabelings applied before ingestion |
| prometheusRule.enabled | bool | `false` | Create a PrometheusRule |
| prometheusRule.namespace | string | `""` | PrometheusRule namespace. Empty uses the release namespace |
| prometheusRule.labels | object | `{}` | Extra PrometheusRule labels, e.g. the one Prometheus selects rules by |
| prometheusRule.annotations | object | `{}` | PrometheusRule annotations |
| prometheusRule.groups | list | `[]` | Rule groups as written. Empty creates no rule |
| resources | object | `{"limits":{"memory":"128Mi"},"requests":{"cpu":"25m","memory":"64Mi"}}` | Container resources |
| resizePolicy | list | `[{"resourceName":"cpu","restartPolicy":"NotRequired"},{"resourceName":"memory","restartPolicy":"RestartContainer"}]` | Container resize policy for in-place resizing. Empty omits it |
| terminationGracePeriodSeconds | int | `240` | Shutdown grace period. Keep it above the longer of `kagent.timeout` and `chat.timeout` plus 30s |
| podAnnotations | object | `{}` | Extra Pod annotations |
| podLabels | object | `{}` | Extra Pod labels |
| podSecurityContext | object | `{"fsGroup":65532,"runAsGroup":65532,"runAsNonRoot":true,"runAsUser":65532,"seccompProfile":{"type":"RuntimeDefault"}}` | Pod security context |
| securityContext | object | `{"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]},"readOnlyRootFilesystem":true}` | Container security context |
| nodeSelector | object | `{}` | Node selector |
| tolerations | list | `[]` | Tolerations |
| affinity | object | `{}` | Affinity |
| extraPodSpec | object | `{}` | Extra Pod spec fields, e.g. runtimeClassName. Fields the chart renders are rejected |
| topologySpreadConstraints | list | `[]` | Topology spread constraints. A missing `labelSelector` uses the chart's selector labels |
| dnsPolicy | string | `""` | Pod DNS policy. Empty omits it |
| dnsConfig | object | `{}` | Pod DNS config. Empty omits it |
| extraObjects | list | `[]` | Extra manifests rendered as written |

## Source Code

* <https://github.com/younsl/ct/tree/main/box/addons/kagent-gateway>

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
