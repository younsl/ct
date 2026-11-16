{{/* Expand the name of the chart. */}}
{{- define "forklift.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/* Fully qualified app name. */}}
{{- define "forklift.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Fully qualified container image reference, joining registry, repository and tag.
The registry is optional: when empty the repository is used as-is (so it may
itself carry a host). Tag defaults to the chart appVersion.
*/}}
{{- define "forklift.image" -}}
{{- $tag := .Values.image.tag | default .Chart.AppVersion -}}
{{- if .Values.image.registry -}}
{{- printf "%s/%s:%s" .Values.image.registry .Values.image.repository $tag -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository $tag -}}
{{- end -}}
{{- with .Values.image.digest -}}
@{{ . }}
{{- end -}}
{{- end }}

{{- define "forklift.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "forklift.labels" -}}
helm.sh/chart: {{ include "forklift.chart" . }}
{{ include "forklift.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "forklift.selectorLabels" -}}
app.kubernetes.io/name: {{ include "forklift.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "forklift.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "forklift.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
s3Enabled is the effective S3 toggle: the operator explicitly set the s3 backend,
OR the bundled SeaweedFS subchart is enabled (which is wired to the s3 backend
automatically). Used by every template that branches on object-storage mode so
"seaweedfs.enabled" alone is enough to switch the whole chart to S3.
*/}}
{{- define "forklift.s3Enabled" -}}
{{- if or (eq .Values.storage.backend "s3") .Values.seaweedfs.enabled -}}true{{- else -}}false{{- end -}}
{{- end -}}

{{/*
Service of the bundled SeaweedFS all-in-one server, matching the subchart's
fullname logic. Honors seaweedfs.fullnameOverride.
*/}}
{{- define "forklift.seaweedfsService" -}}
{{- $full := "" -}}
{{- if .Values.seaweedfs.fullnameOverride -}}
{{- $full = .Values.seaweedfs.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else if contains "seaweedfs" .Release.Name -}}
{{- $full = .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $full = printf "%s-seaweedfs" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- printf "%s-all-in-one" $full -}}
{{- end -}}

{{- define "forklift.s3Endpoint" -}}
{{- if .Values.storage.s3.endpoint -}}
{{- .Values.storage.s3.endpoint -}}
{{- else if .Values.seaweedfs.enabled -}}
{{- printf "http://%s:8333" (include "forklift.seaweedfsService" .) -}}
{{- end -}}
{{- end -}}

{{- define "forklift.s3CredSecret" -}}
{{- if .Values.storage.s3.existingSecret -}}
{{- .Values.storage.s3.existingSecret -}}
{{- else if .Values.seaweedfs.enabled -}}
{{- include "forklift.fullname" . -}}
{{- end -}}
{{- end -}}

{{- define "forklift.s3Region" -}}
{{- if .Values.storage.s3.region -}}
{{- .Values.storage.s3.region -}}
{{- else if .Values.seaweedfs.enabled -}}
us-east-1
{{- end -}}
{{- end -}}

{{/*
Migration Job name. Argo CD renders every sync as revision 1, so the name hashes
the settings instead: a changed setting creates a new Job rather than editing an
immutable one, and an unchanged one leaves the finished Job alone.
*/}}
{{- define "forklift.migrationJobName" -}}
{{- $m := .Values.storage.migration -}}
{{- $key := dict "migration" $m "image" (include "forklift.image" .) "target" (list (include "forklift.s3Endpoint" .) (include "forklift.s3Bucket" .) .Values.storage.s3.prefix (include "forklift.s3CredSecret" .)) "ha" (gt (int (include "forklift.replicaCount" .)) 1) "scheduling" (list .Values.nodeSelector .Values.tolerations .Values.podSecurityContext .Values.securityContext) -}}
{{- printf "%s-migrate-%s" (include "forklift.fullname" . | trunc 45 | trimSuffix "-") (toJson $key | sha256sum | trunc 10) -}}
{{- end -}}

{{/* migrating is true while a migration Job runs, which keeps every forklift pod stopped. */}}
{{- define "forklift.migrating" -}}
{{- if or .Values.storage.migration.enabled .Values.database.migration.enabled -}}true{{- else -}}false{{- end -}}
{{- end -}}

{{- define "forklift.replicas" -}}
{{- if eq (include "forklift.migrating" .) "true" }}0{{ else }}{{ include "forklift.replicaCount" . }}{{ end }}
{{- end -}}

{{/*
Database migration Job name. Hashes the settings for the same reason as the
storage migration Job: Argo CD renders every sync as revision 1.
*/}}
{{- define "forklift.databaseMigrationJobName" -}}
{{- $key := dict "migration" .Values.database.migration "image" (include "forklift.image" .) "database" (list (include "forklift.databaseType" .) .Values.database.external.host .Values.database.external.name .Values.database.schema) "source" (list (include "forklift.s3Endpoint" .) .Values.storage.s3.bucket .Values.storage.s3.prefix) -}}
{{- printf "%s-migrate-db-%s" (include "forklift.fullname" . | trunc 42 | trimSuffix "-") (toJson $key | sha256sum | trunc 10) -}}
{{- end -}}

{{/*
installationMode is the topology preset: "" keeps the values-derived layout,
standalone is one pod, activePassive is a SQLite leader with standbys, and
activeActive is API pods plus a worker on PostgreSQL.
*/}}
{{- define "forklift.mode" -}}
{{- .Values.installationMode | default "" -}}
{{- end -}}

{{/*
replicaCount resolves the server pod count for the mode: one for standalone, at
least two for activePassive, and replicaCount as given otherwise (the API pods
in activeActive).
*/}}
{{- define "forklift.replicaCount" -}}
{{- $mode := include "forklift.mode" . -}}
{{- if eq $mode "standalone" -}}
1
{{- else if eq $mode "activePassive" -}}
{{- max 2 (int .Values.replicaCount) -}}
{{- else -}}
{{- int .Values.replicaCount -}}
{{- end -}}
{{- end -}}

{{/*
databaseType resolves the metadata backend: an explicit database.type wins,
activeActive defaults to the bundled PostgreSQL, and everything else is SQLite.
*/}}
{{- define "forklift.databaseType" -}}
{{- if .Values.database.type -}}
{{- .Values.database.type -}}
{{- else if eq (include "forklift.mode" .) "activeActive" -}}
internal
{{- else -}}
sqlite
{{- end -}}
{{- end -}}

{{- define "forklift.postgres" -}}
{{- if ne (include "forklift.databaseType" .) "sqlite" -}}true{{- else -}}false{{- end -}}
{{- end -}}

{{- define "forklift.database.fullname" -}}
{{- printf "%s-database" (include "forklift.fullname" . | trunc 54 | trimSuffix "-") -}}
{{- end -}}

{{- define "forklift.database.selectorLabels" -}}
app.kubernetes.io/name: {{ include "forklift.name" . }}-database
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "forklift.database.labels" -}}
helm.sh/chart: {{ include "forklift.chart" . }}
{{ include "forklift.database.selectorLabels" . }}
app.kubernetes.io/version: {{ .Values.database.internal.image.tag | quote }}
app.kubernetes.io/component: database
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "forklift.database.image" -}}
{{- $i := .Values.database.internal.image -}}
{{- if $i.registry -}}
{{- printf "%s/%s:%s" $i.registry $i.repository $i.tag -}}
{{- else -}}
{{- printf "%s:%s" $i.repository $i.tag -}}
{{- end -}}
{{- with $i.digest -}}
@{{ . }}
{{- end -}}
{{- end }}

{{/* Secret and key holding the PostgreSQL password forklift connects with. */}}
{{- define "forklift.database.secretName" -}}
{{- if and (eq (include "forklift.databaseType" .) "external") .Values.database.external.existingSecret -}}
{{- .Values.database.external.existingSecret -}}
{{- else -}}
{{- include "forklift.fullname" . -}}
{{- end -}}
{{- end -}}

{{- define "forklift.database.secretKey" -}}
{{- if and (eq (include "forklift.databaseType" .) "external") .Values.database.external.existingSecret -}}
{{- .Values.database.external.existingSecretKey -}}
{{- else -}}
database-password
{{- end -}}
{{- end -}}

{{/* PostgreSQL connection environment, shared by the server and the worker. */}}
{{- define "forklift.databaseEnv" -}}
{{- if eq (include "forklift.postgres" .) "true" }}
{{- $internal := eq (include "forklift.databaseType" .) "internal" }}
{{- $ext := .Values.database.external }}
- name: FORKLIFT_DATABASE_HOST
  value: {{ ternary (include "forklift.database.fullname" .) $ext.host $internal | quote }}
- name: FORKLIFT_DATABASE_PORT
  value: {{ ternary 5432 (int $ext.port) $internal | quote }}
- name: FORKLIFT_DATABASE_NAME
  value: {{ ternary "forklift" $ext.name $internal | quote }}
- name: FORKLIFT_DATABASE_USER
  value: {{ ternary "forklift" $ext.user $internal | quote }}
- name: FORKLIFT_DATABASE_SSLMODE
  value: {{ ternary "disable" $ext.sslMode $internal | quote }}
- name: FORKLIFT_DATABASE_MAX_CONNECTIONS
  value: {{ .Values.database.maxConnections | quote }}
{{- with .Values.database.schema }}
- name: FORKLIFT_DATABASE_SCHEMA
  value: {{ . | quote }}
{{- end }}
- name: FORKLIFT_DATABASE_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ include "forklift.database.secretName" . }}
      key: {{ include "forklift.database.secretKey" . }}
{{- end }}
{{- end }}

{{- define "forklift.worker.selectorLabels" -}}
app.kubernetes.io/name: {{ include "forklift.name" . }}-worker
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "forklift.worker.labels" -}}
helm.sh/chart: {{ include "forklift.chart" . }}
{{ include "forklift.worker.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/component: worker
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "forklift.s3Provider" -}}
{{- if .Values.storage.s3.provider -}}
{{- .Values.storage.s3.provider -}}
{{- else if .Values.seaweedfs.enabled -}}
seaweedfs
{{- end -}}
{{- end -}}

{{- define "forklift.s3AdminEndpoint" -}}
{{- if .Values.storage.s3.adminEndpoint -}}
{{- .Values.storage.s3.adminEndpoint -}}
{{- else if .Values.seaweedfs.enabled -}}
{{- printf "http://%s:9333" (include "forklift.seaweedfsService" .) -}}
{{- end -}}
{{- end -}}

{{/*
Effective S3 bucket: an explicit storage.s3.bucket wins. With the bundled
SeaweedFS it falls back to the first createBuckets entry, then to "forklift",
which forklift creates at boot. Fails when none applies.
*/}}
{{- define "forklift.s3Bucket" -}}
{{- $buckets := ((.Values.seaweedfs.allInOne).s3).createBuckets -}}
{{- if .Values.storage.s3.bucket -}}
{{- .Values.storage.s3.bucket -}}
{{- else if and .Values.seaweedfs.enabled $buckets -}}
{{- (first $buckets).name -}}
{{- else if .Values.seaweedfs.enabled -}}
forklift
{{- else -}}
{{- fail "storage.s3.bucket is required when storage.backend is s3 without the bundled SeaweedFS" -}}
{{- end -}}
{{- end -}}

{{/*
Validate mutually exclusive storage/HA modes. The s3 backend (including the
bundled SeaweedFS) already shares blobs and snapshots metadata to S3, so PV-based
peer replication is redundant and would compete for the metadata snapshot.
*/}}
{{- define "forklift.validateStorage" -}}
{{- if and (eq (include "forklift.s3Enabled" .) "true") .Values.replication.enabled -}}
{{- fail "object-storage mode (storage.backend=s3 or seaweedfs.enabled=true) is incompatible with replication.enabled=true; disable one (s3 mode shares blobs and snapshots metadata to S3, so peer replication is unnecessary)" -}}
{{- end -}}
{{- if .Values.storage.migration.enabled -}}
{{- if ne (include "forklift.s3Enabled" .) "true" -}}
{{- fail "storage.migration.enabled copies into object storage; set storage.backend=s3 or seaweedfs.enabled=true" -}}
{{- end -}}
{{- if not .Values.storage.migration.sourceObjectStorage.bucket -}}
{{- fail "storage.migration.sourceObjectStorage.bucket is required when storage.migration.enabled is true" -}}
{{- end -}}
{{- end -}}
{{- if not (has .Values.storage.backend (list "fs" "s3")) -}}
{{- fail (printf "storage.backend must be \"fs\" or \"s3\", got %q" .Values.storage.backend) -}}
{{- end -}}
{{- $mode := include "forklift.mode" . -}}
{{- $db := include "forklift.databaseType" . -}}
{{- $postgres := eq (include "forklift.postgres" .) "true" -}}
{{- if and (eq $mode "activeActive") (not $postgres) -}}
{{- fail "installationMode=activeActive needs PostgreSQL; set database.type to internal or external, or leave it empty for the bundled one" -}}
{{- end -}}
{{- if and (eq $mode "activePassive") $postgres -}}
{{- fail "installationMode=activePassive is the SQLite leader layout; use installationMode=activeActive with PostgreSQL" -}}
{{- end -}}
{{- if and (eq $mode "standalone") .Values.replication.enabled -}}
{{- fail "installationMode=standalone runs one pod and cannot use replication.enabled=true" -}}
{{- end -}}
{{- if and $postgres .Values.replication.enabled -}}
{{- fail "PostgreSQL (database.type) is incompatible with replication.enabled=true; PV replication copies the SQLite file" -}}
{{- end -}}
{{- if and $postgres .Values.storage.migration.enabled -}}
{{- fail "storage.migration.enabled moves a SQLite snapshot and is not supported with PostgreSQL (database.type)" -}}
{{- end -}}
{{- /*
Leaving PostgreSQL by accident. database.type is derived from the mode when
empty, so changing only installationMode away from activeActive would start
forklift on an empty SQLite database and drop the bundled server. When the
release already runs on PostgreSQL, the move to SQLite has to be spelled out.
lookup returns nothing without cluster access (helm template, Argo CD), so
this guard only acts on installs and upgrades that reach the cluster.
*/ -}}
{{- if and (not $postgres) (not .Values.database.type) -}}
{{- $sts := lookup "apps/v1" "StatefulSet" .Release.Namespace (include "forklift.database.fullname" .) -}}
{{- $secret := lookup "v1" "Secret" .Release.Namespace (include "forklift.fullname" .) -}}
{{- if or $sts (and $secret $secret.data (hasKey $secret.data "database-password")) -}}
{{- fail "this release stores its metadata in PostgreSQL, and these values would switch it to an empty SQLite database. Set database.type=internal (or external) to keep PostgreSQL, or database.type=sqlite to confirm the switch" -}}
{{- end -}}
{{- end -}}
{{- if and .Values.database.migration.enabled (not $postgres) -}}
{{- fail "database.migration.enabled copies SQLite metadata into PostgreSQL; set database.type to internal or external" -}}
{{- end -}}
{{- if and .Values.database.migration.enabled (ne (include "forklift.s3Enabled" .) "true") -}}
{{- fail "database.migration.enabled reads the SQLite metadata from the bucket of the s3 backend; for a volume install run forklift migrate-database -from-sqlite by hand" -}}
{{- end -}}
{{- if and (eq $db "external") (not .Values.database.external.host) -}}
{{- fail "database.external.host is required when database.type is external" -}}
{{- end -}}
{{- $several := or (eq $mode "activeActive") (gt (int (include "forklift.replicaCount" .)) 1) -}}
{{- $shared := or (eq (include "forklift.s3Enabled" .) "true") (has "ReadWriteMany" .Values.persistence.accessModes) -}}
{{- if and $postgres $several (not $shared) -}}
{{- fail "several pods on PostgreSQL need a blob store they all share: set storage.backend=s3 (or seaweedfs.enabled=true) or a ReadWriteMany persistence.accessModes" -}}
{{- end -}}
{{- end -}}

{{/*
labelRouting is true when traffic is routed to the leader via the
forklift.io/role label (every pod stays Ready) instead of gating readiness on
leadership: replication mode, or the s3 backend in HA. The main Service selects
the leader label and the leader gets RBAC to patch pod labels.
*/}}
{{- define "forklift.labelRouting" -}}
{{- if eq (include "forklift.postgres" .) "true" -}}false
{{- else if or .Values.replication.enabled (and (eq (include "forklift.s3Enabled" .) "true") (eq (include "forklift.haEnabled" .) "true")) -}}true{{- else -}}false{{- end -}}
{{- end -}}

{{/* haEnabled resolves the HA toggle: explicit value, else replicaCount > 1. */}}
{{- define "forklift.haEnabled" -}}
{{- $mode := include "forklift.mode" . -}}
{{- if eq $mode "standalone" }}
{{- false }}
{{- else if or (eq $mode "activePassive") (eq $mode "activeActive") }}
{{- true }}
{{- else if kindIs "bool" .Values.ha.enabled }}
{{- .Values.ha.enabled }}
{{- else }}
{{- gt (int .Values.replicaCount) 1 }}
{{- end }}
{{- end }}

{{- define "forklift.leaseName" -}}
{{- default (printf "%s-leader" (include "forklift.fullname" .)) .Values.ha.leaseName }}
{{- end }}

{{- define "forklift.headlessServiceName" -}}
{{ include "forklift.fullname" . }}-headless
{{- end }}

{{/*
Container environment shared by the Deployment (shared RWX volume mode) and the
StatefulSet (PV-based replication mode).
*/}}
{{- define "forklift.env" -}}
- name: POD_NAME
  valueFrom:
    fieldRef:
      fieldPath: metadata.name
- name: POD_NAMESPACE
  valueFrom:
    fieldRef:
      fieldPath: metadata.namespace
- name: FORKLIFT_DATA_DIR
  value: /data
{{- if eq (include "forklift.s3Enabled" .) "true" }}
- name: FORKLIFT_STORAGE_BACKEND
  value: "s3"
- name: FORKLIFT_STORAGE_S3_BUCKET
  value: {{ include "forklift.s3Bucket" . | quote }}
{{- with .Values.storage.s3.prefix }}
- name: FORKLIFT_STORAGE_S3_PREFIX
  value: {{ . | quote }}
{{- end }}
{{- with (include "forklift.s3Region" .) }}
- name: FORKLIFT_STORAGE_S3_REGION
  value: {{ . | quote }}
{{- end }}
{{- with (include "forklift.s3Endpoint" .) }}
- name: FORKLIFT_STORAGE_S3_ENDPOINT
  value: {{ . | quote }}
{{- end }}
- name: FORKLIFT_STORAGE_S3_FORCE_PATH_STYLE
  value: {{ or .Values.storage.s3.forcePathStyle .Values.seaweedfs.enabled | quote }}
{{- with (include "forklift.s3Provider" .) }}
- name: FORKLIFT_STORAGE_S3_PROVIDER
  value: {{ . | quote }}
{{- end }}
{{- with (include "forklift.s3AdminEndpoint" .) }}
- name: FORKLIFT_STORAGE_S3_ADMIN_ENDPOINT
  value: {{ . | quote }}
{{- end }}
- name: FORKLIFT_STORAGE_S3_CREATE_BUCKET
  value: {{ or .Values.storage.s3.createBucket .Values.seaweedfs.enabled | quote }}
- name: FORKLIFT_STORAGE_META_SYNC_INTERVAL
  value: {{ .Values.storage.s3.metaSyncInterval | quote }}
- name: FORKLIFT_STORAGE_META_COMPRESSION
  value: {{ .Values.storage.s3.metaCompression | quote }}
- name: FORKLIFT_STORAGE_META_SYNC_MODE
  value: {{ .Values.storage.s3.metaSyncMode | quote }}
{{- /*
S3 credentials. An explicit existingSecret wins. Otherwise, when SeaweedFS is
bundled, read the admin credentials forklift mirrors into its own Secret. With
neither, no static keys are set and the app falls back to the AWS default
credential chain (EKS IRSA / Pod Identity).
*/}}
{{- with (include "forklift.s3CredSecret" .) }}
- name: FORKLIFT_STORAGE_S3_ACCESS_KEY_ID
  valueFrom:
    secretKeyRef:
      name: {{ . }}
      key: access-key-id
- name: FORKLIFT_STORAGE_S3_SECRET_ACCESS_KEY
  valueFrom:
    secretKeyRef:
      name: {{ . }}
      key: secret-access-key
{{- end }}
{{- if .Values.storage.s3.existingSecret }}
- name: FORKLIFT_STORAGE_S3_ADMIN_TOKEN
  valueFrom:
    secretKeyRef:
      name: {{ .Values.storage.s3.existingSecret }}
      key: admin-token
      optional: true
{{- else if .Values.storage.s3.adminToken }}
- name: FORKLIFT_STORAGE_S3_ADMIN_TOKEN
  valueFrom:
    secretKeyRef:
      name: {{ include "forklift.fullname" . }}
      key: s3-admin-token
{{- end }}
{{- end }}
- name: FORKLIFT_LOG_LEVEL
  value: {{ .Values.log.level | quote }}
- name: FORKLIFT_LOG_FORMAT
  value: {{ .Values.log.format | quote }}
- name: FORKLIFT_ANONYMOUS_READ
  value: {{ .Values.auth.anonymousRead | quote }}
- name: FORKLIFT_SEED_DEFAULT_REPOS
  value: {{ .Values.seedDefaultRepos | quote }}
- name: FORKLIFT_AUDIT_ENABLED
  value: {{ .Values.audit.enabled | quote }}
- name: FORKLIFT_AUDIT_RETENTION
  value: {{ .Values.audit.retention | quote }}
- name: FORKLIFT_AUDIT_DOWNLOAD_RETENTION
  value: {{ .Values.audit.downloadRetention | quote }}
- name: FORKLIFT_SESSION_TTL
  value: {{ .Values.auth.sessionTTL | quote }}
{{- if eq (include "forklift.haEnabled" .) "true" }}
- name: FORKLIFT_HA_ENABLED
  value: "true"
- name: FORKLIFT_HA_LEASE_NAME
  value: {{ include "forklift.leaseName" . }}
- name: FORKLIFT_HA_LEASE_DURATION
  value: {{ .Values.ha.leaseDuration | quote }}
- name: FORKLIFT_HA_RENEW_DEADLINE
  value: {{ .Values.ha.renewDeadline | quote }}
- name: FORKLIFT_HA_RETRY_PERIOD
  value: {{ .Values.ha.retryPeriod | quote }}
{{- end }}
{{- include "forklift.databaseEnv" . }}
{{- if .Values.replication.enabled }}
- name: FORKLIFT_REPLICATION_ENABLED
  value: "true"
- name: FORKLIFT_REPLICATION_PEER_SERVICE
  value: "{{ include "forklift.headlessServiceName" . }}.{{ .Release.Namespace }}.svc.cluster.local"
- name: FORKLIFT_REPLICATION_INTERVAL
  value: {{ .Values.replication.interval | quote }}
- name: FORKLIFT_REPLICATION_TOKEN
  valueFrom:
    secretKeyRef:
      name: {{ include "forklift.fullname" . }}
      key: replication-token
{{- end }}
{{- if .Values.auth.oidc.enabled }}
- name: FORKLIFT_OIDC_ENABLED
  value: "true"
- name: FORKLIFT_OIDC_ISSUER_URL
  value: {{ .Values.auth.oidc.issuerURL | quote }}
- name: FORKLIFT_OIDC_CLIENT_ID
  value: {{ .Values.auth.oidc.clientID | quote }}
- name: FORKLIFT_OIDC_REDIRECT_URL
  value: {{ .Values.auth.oidc.redirectURL | quote }}
- name: FORKLIFT_OIDC_USERNAME_CLAIM
  value: {{ .Values.auth.oidc.usernameClaim | quote }}
- name: FORKLIFT_OIDC_GROUPS_CLAIM
  value: {{ .Values.auth.oidc.groupsClaim | quote }}
- name: FORKLIFT_OIDC_CLIENT_SECRET
  valueFrom:
    secretKeyRef:
      name: {{ include "forklift.fullname" . }}
      key: oidc-client-secret
{{- end }}
- name: FORKLIFT_SESSION_SECRET
  valueFrom:
    secretKeyRef:
      name: {{ include "forklift.fullname" . }}
      key: session-secret
- name: FORKLIFT_BOOTSTRAP_ADMIN_USER
  value: {{ .Values.auth.bootstrap.adminUser | quote }}
- name: FORKLIFT_BOOTSTRAP_ADMIN_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ include "forklift.fullname" . }}
      key: bootstrap-admin-password
{{- if .Values.auth.rbac.enabled }}
- name: FORKLIFT_RBAC_POLICY_FILE
  value: /etc/forklift/rbac/policy.csv
- name: FORKLIFT_RBAC_DEFAULT_ROLE
  value: {{ .Values.auth.rbac.policyDefault | quote }}
{{- if .Values.auth.rbac.accounts }}
- name: FORKLIFT_RBAC_ACCOUNTS_DIR
  value: /etc/forklift/accounts
{{- end }}
{{- end }}
{{- with .Values.externalUrl }}
- name: FORKLIFT_EXTERNAL_URL
  value: {{ . | quote }}
{{- end }}
{{- /*
Forklift coverage: the GitLab instance to scan. Only the connection is set
here; the scope, schedule, exclusions and report receivers are edited by an
administrator in the console. The token is read from a Secret so it never
reaches the metadata database or an API response, and an existing Secret is
supported so it does not have to live in values.yaml either.
*/}}
{{- if .Values.coverageScanning.gitlab.enabled }}
{{- if not .Values.coverageScanning.gitlab.url }}
{{- fail "coverageScanning.gitlab.url is required when coverageScanning.gitlab.enabled is true" }}
{{- end }}
{{- if and (not .Values.coverageScanning.gitlab.token) (not .Values.coverageScanning.gitlab.existingSecret) }}
{{- fail "coverageScanning.gitlab.token or coverageScanning.gitlab.existingSecret is required when coverageScanning.gitlab.enabled is true" }}
{{- end }}
- name: FORKLIFT_COVERAGE_ENABLED
  value: "true"
- name: FORKLIFT_COVERAGE_GITLAB_URL
  value: {{ .Values.coverageScanning.gitlab.url | quote }}
- name: FORKLIFT_COVERAGE_GITLAB_TOKEN
  valueFrom:
    secretKeyRef:
      name: {{ .Values.coverageScanning.gitlab.existingSecret | default (include "forklift.fullname" .) }}
      key: {{ .Values.coverageScanning.gitlab.existingSecretKey | quote }}
{{- end }}
{{- include "forklift.outboundEnv" . }}
{{- with .Values.extraEnv }}
{{ toYaml . }}
{{- end }}
{{- end }}

{{/*
Outbound proxy and private CA environment, shared by the server and the
migration Job. NO_PROXY always lists the in-cluster names forklift dials
(its own Services and the bundled SeaweedFS, addressed by short name), so
setting a proxy never routes replication or storage through it.
*/}}
{{- define "forklift.outboundEnv" -}}
{{- $o := .Values.outbound }}
{{- if or $o.proxy.https $o.proxy.http }}
{{- $noProxy := list "localhost" "127.0.0.1" "::1" ".svc" ".cluster.local" (include "forklift.fullname" .) (include "forklift.headlessServiceName" .) }}
{{- if .Values.seaweedfs.enabled }}
{{- $noProxy = append $noProxy (include "forklift.seaweedfsService" .) }}
{{- end }}
{{- $noProxy = concat $noProxy ($o.proxy.noProxy | default list) }}
{{- with $o.proxy.https }}
- name: HTTPS_PROXY
  value: {{ . | quote }}
{{- end }}
{{- with $o.proxy.http }}
- name: HTTP_PROXY
  value: {{ . | quote }}
{{- end }}
- name: NO_PROXY
  value: {{ join "," $noProxy | quote }}
{{- end }}
{{- if $o.caBundle.configMap }}
- name: FORKLIFT_EXTRA_CA_FILE
  value: {{ printf "/etc/forklift/ca/%s" $o.caBundle.key | quote }}
{{- end }}
{{- end }}

{{- define "forklift.outboundVolumeMounts" -}}
{{- if .Values.outbound.caBundle.configMap }}
- name: extra-ca
  mountPath: /etc/forklift/ca
  readOnly: true
{{- end }}
{{- end }}

{{- define "forklift.outboundVolumes" -}}
{{- with .Values.outbound.caBundle }}
{{- if .configMap }}
- name: extra-ca
  configMap:
    name: {{ .configMap }}
    items:
      - key: {{ .key }}
        path: {{ .key }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Container args: vulnerability (OSV) and license (deps.dev) scanning are
configured via CLI flags rather than env. An empty URL is still passed so it
explicitly disables the corresponding scanner (overriding the binary default).
*/}}
{{- define "forklift.args" -}}
{{- /*
Older releases have no uiUpload key in their stored values. In particular,
`helm upgrade --reuse-values` does not merge newly introduced chart defaults.
Read every field through dig so those releases render with the new defaults,
while an explicit enabled=false remains false (unlike the `default` function,
which treats false as empty).
*/ -}}
{{- $uiUpload := (get .Values "uiUpload") | default dict -}}
- --osv-url={{ .Values.vuln.osvUrl }}
- --vuln-rescan-interval={{ .Values.vuln.rescanInterval }}
- --vuln-ttl={{ .Values.vuln.ttl }}
- --vuln-workers={{ .Values.vuln.workers }}
- --deps-dev-url={{ .Values.license.depsDevUrl }}
- --license-rescan-interval={{ .Values.license.rescanInterval }}
- --license-ttl={{ .Values.license.ttl }}
- --license-workers={{ .Values.license.workers }}
- --ui-upload-enabled={{ dig "enabled" true $uiUpload }}
- --ui-upload-max-duration={{ dig "maxDuration" "30m" $uiUpload }}
- --ui-upload-max-concurrent={{ dig "maxConcurrent" 4 $uiUpload }}
- --ui-upload-max-concurrent-user={{ dig "maxConcurrentUser" 2 $uiUpload }}
- --ui-upload-max-assets={{ dig "maxAssets" 16 $uiUpload }}
- --ui-upload-max-file-bytes={{ dig "maxFileBytes" "256MiB" $uiUpload }}
- --ui-upload-max-batch-bytes={{ dig "maxBatchBytes" "512MiB" $uiUpload }}
- --ui-upload-go-max-zip-bytes={{ dig "goMaxZipBytes" "500MiB" $uiUpload }}
{{- with .Values.extraArgs }}
{{ toYaml . }}
{{- end }}
{{- end }}

{{/*
RBAC volume mounts: the policy.csv ConfigMap and, when local accounts are
declared, the per-account password Secret projected as files named by username.
*/}}
{{- define "forklift.rbacVolumeMounts" -}}
{{- if .Values.auth.rbac.enabled }}
- name: rbac-policy
  mountPath: /etc/forklift/rbac
  readOnly: true
{{- if .Values.auth.rbac.accounts }}
- name: rbac-accounts
  mountPath: /etc/forklift/accounts
  readOnly: true
{{- end }}
{{- end }}
{{- end }}

{{- define "forklift.rbacVolumes" -}}
{{- if .Values.auth.rbac.enabled }}
- name: rbac-policy
  configMap:
    name: {{ include "forklift.fullname" . }}-rbac
{{- if .Values.auth.rbac.accounts }}
- name: rbac-accounts
  secret:
    secretName: {{ include "forklift.fullname" . }}
    items:
      {{- range .Values.auth.rbac.accounts }}
      - key: {{ printf "local-user-%s-password" .name }}
        path: {{ .name }}
      {{- end }}
{{- end }}
{{- end }}
{{- end }}

{{/*
MCP server image reference. forklift-mcp is its own image with an independent
release line, so the tag never falls back to the chart appVersion.
*/}}
{{- define "forklift.mcp.image" -}}
{{- if .Values.mcp.image.registry -}}
{{- printf "%s/%s:%s" .Values.mcp.image.registry .Values.mcp.image.repository .Values.mcp.image.tag -}}
{{- else -}}
{{- printf "%s:%s" .Values.mcp.image.repository .Values.mcp.image.tag -}}
{{- end -}}
{{- with .Values.mcp.image.digest -}}
@{{ . }}
{{- end -}}
{{- end }}

{{- define "forklift.mcp.selectorLabels" -}}
app.kubernetes.io/name: {{ include "forklift.name" . }}-mcp
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "forklift.mcp.labels" -}}
helm.sh/chart: {{ include "forklift.chart" . }}
{{ include "forklift.mcp.selectorLabels" . }}
app.kubernetes.io/version: {{ .Values.mcp.image.tag | quote }}
app.kubernetes.io/component: mcp
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Upstream forklift management API base URL the MCP server proxies to.
Defaults to this release's in-cluster Service.
*/}}
{{- define "forklift.mcp.upstreamURL" -}}
{{- if .Values.mcp.upstreamURL -}}
{{- .Values.mcp.upstreamURL -}}
{{- else -}}
{{- printf "http://%s:%d" (include "forklift.fullname" .) (int .Values.service.port) -}}
{{- end -}}
{{- end }}
