import { useCallback, useEffect, useState } from 'react'
import { useAuth } from '../contexts/AuthContext'
import {
  createAlert,
  deleteAlert,
  getRegisteredClusters,
  listAlerts,
  previewAlert,
  testAlertDraft,
  updateAlert,
  type AlertListResponse,
  type RegisteredCluster,
} from '../api'
import type {
  AlertMatcherFields,
  AlertMatchers,
  AlertPreviewResult,
  AlertRule,
  AlertRuleInput,
  AlertSeverity,
} from '../types'
import AdminSubNav from '../components/AdminSubNav'
import { formatDate } from '../utils'
import styles from './AlertsPage.module.css'

type Trigger = 'component' | 'vulnerability'
type FixFilter = '' | 'true' | 'false'

const SEVERITIES: AlertSeverity[] = ['CRITICAL', 'HIGH', 'MEDIUM', 'LOW', 'UNKNOWN']

// Mirrors MAX_TERMS in src/alerts/matchers.rs and the CRD's maxItems.
const MAX_TERMS = 20

interface TermForm {
  trigger: Trigger
  packageName: string
  versionExpr: string
  clusters: string[]
  namespace: string
  excludeNamespaces: string
  excludePackages: string
  vulnIds: string
  excludeIds: string
  severities: AlertSeverity[]
  minScore: string
  fixAvailable: FixFilter
}

interface FormState {
  name: string
  description: string
  enabled: boolean
  terms: TermForm[]
  receiverName: string
  webhookUrl: string
  channel: string
  cooldownSecs: string
}

const emptyTerm = (): TermForm => ({
  trigger: 'component',
  packageName: '',
  versionExpr: '',
  clusters: [],
  namespace: '',
  excludeNamespaces: '',
  excludePackages: '',
  vulnIds: '',
  excludeIds: '',
  severities: [],
  minScore: '',
  fixAvailable: '',
})

const emptyForm = (): FormState => ({
  name: '',
  description: '',
  enabled: true,
  terms: [emptyTerm()],
  receiverName: 'slack-default',
  webhookUrl: '',
  channel: '',
  cooldownSecs: '3600',
})

const ruleTerms = (rule: AlertRule): AlertMatchers[] =>
  rule.matcher_terms && rule.matcher_terms.length > 0
    ? rule.matcher_terms
    : rule.matchers
      ? [rule.matchers]
      : []

const termFromMatchers = (m: AlertMatchers): TermForm => {
  const vuln = m.vulnerability
  return {
    trigger: vuln ? 'vulnerability' : 'component',
    packageName: m.package_name ?? '',
    versionExpr: m.version_expr ?? '',
    clusters: m.clusters ?? [],
    namespace: m.namespace ?? '',
    excludeNamespaces: (m.exclude_namespaces ?? []).join(', '),
    excludePackages: (m.exclude_packages ?? []).join(', '),
    vulnIds: (vuln?.ids ?? []).join(', '),
    excludeIds: (vuln?.exclude_ids ?? []).join(', '),
    severities: vuln?.severities ?? [],
    minScore: vuln?.min_score?.toString() ?? '',
    fixAvailable:
      vuln?.fix_available === true ? 'true' : vuln?.fix_available === false ? 'false' : '',
  }
}

const fromRule = (rule: AlertRule): FormState => {
  const slack = rule.receivers[0]?.slack
  const terms = ruleTerms(rule).map(termFromMatchers)
  return {
    name: rule.name,
    description: rule.description ?? '',
    enabled: rule.enabled,
    terms: terms.length > 0 ? terms : [emptyTerm()],
    receiverName: rule.receivers[0]?.name ?? 'slack-default',
    webhookUrl: slack?.webhook_url ?? '',
    channel: slack?.channel ?? '',
    cooldownSecs: rule.cooldown_secs?.toString() ?? '',
  }
}

const trimmed = (s: string) => (s.trim() === '' ? null : s.trim())

const splitList = (s: string) =>
  s
    .split(/[\s,]+/)
    .map((v) => v.trim())
    .filter((v) => v !== '')

const parseScore = (s: string): number | null => (s.trim() === '' ? null : Number(s))

const toMatchers = (t: TermForm): AlertMatchers => ({
  package_name: trimmed(t.packageName),
  version_expr: trimmed(t.versionExpr),
  clusters: t.clusters,
  namespace: trimmed(t.namespace),
  exclude_namespaces: splitList(t.excludeNamespaces),
  exclude_packages: splitList(t.excludePackages),
  vulnerability:
    t.trigger === 'vulnerability'
      ? {
          ids: splitList(t.vulnIds),
          exclude_ids: splitList(t.excludeIds),
          severities: t.severities,
          min_score: parseScore(t.minScore),
          fix_available: t.fixAvailable === '' ? null : t.fixAvailable === 'true',
        }
      : null,
})

// One term is sent as `matchers` so the stored object keeps the shape that
// kubectl's PACKAGE and VERSION columns read.
const toMatcherFields = (form: FormState): AlertMatcherFields =>
  form.terms.length === 1
    ? { matchers: toMatchers(form.terms[0]) }
    : { matcher_terms: form.terms.map(toMatchers) }

// A vulnerability term with no condition fires on every finding in scope,
// which is almost never meant, so the form asks for at least one.
const hasVulnerabilityCondition = (t: TermForm) =>
  t.packageName.trim() !== '' ||
  splitList(t.vulnIds).length > 0 ||
  t.severities.length > 0 ||
  t.minScore.trim() !== ''

const hasNarrowingMatcher = (t: TermForm) =>
  t.trigger === 'vulnerability'
    ? hasVulnerabilityCondition(t)
    : t.packageName.trim() !== '' || t.versionExpr.trim() !== ''

const scoreError = (t: TermForm): string | null => {
  const score = parseScore(t.minScore)
  if (t.trigger !== 'vulnerability' || score === null) return null
  return Number.isFinite(score) && score >= 0 && score <= 10
    ? null
    : 'minimum CVSS score must be a number from 0 to 10'
}

const termProblem = (t: TermForm): string | null => {
  if (t.trigger === 'component' && !t.packageName.trim()) return 'package name is required'
  if (t.trigger === 'vulnerability' && !hasVulnerabilityCondition(t))
    return 'set a vulnerability ID, severity, minimum CVSS score, or package name'
  const badScore = scoreError(t)
  if (badScore) return badScore
  if (t.clusters.length === 0) return 'select at least one cluster'
  return null
}

const termLabel = (form: FormState, i: number) =>
  form.terms.length > 1 ? `Term ${i + 1}: ` : ''

const termSummary = (m: AlertMatchers): string => {
  const parts: string[] = []
  const v = m.vulnerability
  if (v) {
    if (v.ids?.length) parts.push(v.ids.join(', '))
    if (v.severities?.length) parts.push(v.severities.join('|'))
    if (v.min_score != null) parts.push(`CVSS >= ${v.min_score}`)
    if (v.fix_available != null) parts.push(v.fix_available ? 'fixable' : 'no fix')
    if (v.exclude_ids?.length) parts.push(`not ${v.exclude_ids.join(', ')}`)
  }
  if (m.package_name) parts.push(m.package_name)
  if (m.version_expr) parts.push(m.version_expr)
  if (m.exclude_packages?.length) parts.push(`not ${m.exclude_packages.join(', ')}`)
  if (m.exclude_namespaces?.length) parts.push(`not ns ${m.exclude_namespaces.join(', ')}`)
  return parts.length > 0 ? parts.join(' / ') : 'any'
}

const toInput = (form: FormState): AlertRuleInput => ({
  name: form.name.trim(),
  description: form.description,
  enabled: form.enabled,
  ...toMatcherFields(form),
  receivers: [
    {
      name: form.receiverName.trim() || 'slack-default',
      slack: {
        webhook_url: form.webhookUrl.trim(),
        channel: trimmed(form.channel),
        title: null,
      },
    },
  ],
  cooldown_secs:
    form.cooldownSecs.trim() === '' ? null : Number(form.cooldownSecs),
})

export default function AlertsPage() {
  const { permissions } = useAuth()
  const canManage = !!permissions?.can_manage_alerts
  const [meta, setMeta] = useState<AlertListResponse | null>(null)
  const [rules, setRules] = useState<AlertRule[]>([])
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [editing, setEditing] = useState<AlertRule | null>(null)
  const [form, setForm] = useState<FormState | null>(null)
  const [clusterOptions, setClusterOptions] = useState<RegisteredCluster[]>([])

  useEffect(() => {
    let cancelled = false
    getRegisteredClusters()
      .then((cs) => {
        if (!cancelled) setClusterOptions(cs)
      })
      .catch(() => {
        if (!cancelled) setClusterOptions([])
      })
    return () => {
      cancelled = true
    }
  }, [])

  const refresh = useCallback(async () => {
    setLoading(true)
    setError(null)
    try {
      const res = await listAlerts()
      setMeta(res)
      setRules(res.items)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    refresh()
  }, [refresh])

  const startCreate = () => {
    setEditing(null)
    setForm(emptyForm())
  }
  const startEdit = (rule: AlertRule) => {
    setEditing(rule)
    setForm(fromRule(rule))
  }
  const cancel = () => {
    setEditing(null)
    setForm(null)
    setError(null)
  }

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!form) return
    setError(null)
    const input = toInput(form)
    if (!input.name) return setError('Name is required')
    for (const [i, t] of form.terms.entries()) {
      const problem = termProblem(t)
      if (problem) return setError(`${termLabel(form, i)}${problem}`)
    }
    const webhookUrl = input.receivers[0]?.slack?.webhook_url ?? ''
    if (!webhookUrl) return setError('Slack webhook URL is required')
    if (!webhookUrl.startsWith('https://hooks.slack.com/'))
      return setError(
        'Webhook URL must start with https://hooks.slack.com/ — only the canonical Slack endpoint is allowed.',
      )
    try {
      if (editing) {
        await updateAlert(editing.name, input)
      } else {
        await createAlert(input)
      }
      cancel()
      refresh()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const remove = async (rule: AlertRule) => {
    if (!confirm(`Delete alert rule "${rule.name}"?`)) return
    try {
      await deleteAlert(rule.name)
      refresh()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const showForm = form !== null

  return (
    <div className={styles.container}>
      <AdminSubNav />

      <div className={styles.header}>
        <div>
          <h2 className={styles.headerTitle}>Alert Rules</h2>
          {meta && (
            <div className={styles.headerMeta}>
              <code>{meta.resource}.{meta.api_version.split('/')[0]}</code> in namespace{' '}
              <code>{meta.namespace}</code> · {meta.total} rule(s)
            </div>
          )}
        </div>
        {canManage && !showForm && (
          <div className={styles.headerActions}>
            <button className="btn-secondary" onClick={startCreate}>
              New rule
            </button>
          </div>
        )}
      </div>

      {error && <div className={styles.errorBanner}>{error}</div>}

      {form && (
        <RuleForm
          form={form}
          setForm={setForm}
          onCancel={cancel}
          onSubmit={submit}
          editing={!!editing}
          canManage={canManage}
          clusterOptions={clusterOptions}
        />
      )}

      {loading ? (
        <div className={styles.loadingState}>Loading…</div>
      ) : rules.length === 0 ? (
        showForm ? null : (
          <div className={styles.empty}>No alert rules configured.</div>
        )
      ) : (
        <div className={styles.tableCard}>
          <table className={styles.table}>
            <thead>
              <tr>
                <th>Name</th>
                <th>Trigger</th>
                <th>Match</th>
                <th>Receivers</th>
                <th>Updated</th>
                <th>Status</th>
                <th></th>
              </tr>
            </thead>
            <tbody>
              {rules.map((r) => (
                <tr key={r.name}>
                  <td>
                    <div className={styles.ruleName}>{r.name}</div>
                    {r.description && <div className={styles.ruleDescription}>{r.description}</div>}
                  </td>
                  <td>
                    <div className={styles.matchSummary}>
                      {ruleTerms(r).some((m) => !m.vulnerability) && (
                        <span className={styles.typePillSbom}>Component</span>
                      )}
                      {ruleTerms(r).some((m) => m.vulnerability) && (
                        <span className={styles.typePillCve}>Vulnerability</span>
                      )}
                    </div>
                  </td>
                  <td className={styles.codeCell}>
                    {ruleTerms(r).map((m, i) => (
                      <div key={i}>
                        {i > 0 && <span className={styles.termOr}>OR </span>}
                        <code>{termSummary(m)}</code>
                      </div>
                    ))}
                  </td>
                  <td>
                    {r.receivers.map((rc) => (
                      <div key={rc.name} className={styles.receiverLine}>
                        {rc.name}{rc.slack?.channel ? ` → ${rc.slack.channel}` : ''}
                      </div>
                    ))}
                  </td>
                  <td>{formatDate(r.updated_at ?? r.created_at)}</td>
                  <td>
                    <span className={r.enabled ? styles.statusOn : styles.statusOff}>
                      {r.enabled ? 'on' : 'off'}
                    </span>
                  </td>
                  <td>
                    {canManage ? (
                      <div className={styles.actionsCell}>
                        <button className="btn-secondary btn-sm" onClick={() => startEdit(r)}>Edit</button>
                        <button className="btn-secondary btn-sm" onClick={() => remove(r)}>Delete</button>
                      </div>
                    ) : (
                      <span className={styles.readonlyTag}>read-only</span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  )
}

interface RuleFormProps {
  form: FormState
  setForm: (f: FormState) => void
  onCancel: () => void
  onSubmit: (e: React.FormEvent) => void
  editing: boolean
  canManage: boolean
  clusterOptions: RegisteredCluster[]
}

function RuleForm({
  form,
  setForm,
  onCancel,
  onSubmit,
  editing,
  canManage,
  clusterOptions,
}: RuleFormProps) {
  const set = <K extends keyof FormState>(k: K, v: FormState[K]) =>
    setForm({ ...form, [k]: v })
  const setTerm = (i: number, t: TermForm) =>
    setForm({ ...form, terms: form.terms.map((old, j) => (j === i ? t : old)) })
  const addTerm = () => setForm({ ...form, terms: [...form.terms, emptyTerm()] })
  const removeTerm = (i: number) =>
    setForm({ ...form, terms: form.terms.filter((_, j) => j !== i) })

  const [testing, setTesting] = useState(false)
  const [testStatus, setTestStatus] = useState<
    | { kind: 'ok' | 'partial' | 'error'; message: string }
    | null
  >(null)

  // Mirror the submit-time required-field check so the Test button can't
  // dispatch a half-filled draft. Reasons surfaced via tooltip.
  const missingRequired: string[] = []
  if (!form.name.trim()) missingRequired.push('Name')
  form.terms.forEach((t, i) => {
    const problem = termProblem(t)
    if (problem) missingRequired.push(`${termLabel(form, i)}${problem}`)
  })
  if (!form.webhookUrl.trim()) missingRequired.push('Webhook URL')
  const canTest = canManage && !testing && missingRequired.length === 0
  const testDisabledReason =
    missingRequired.length > 0
      ? `Required to send a test: ${missingRequired.join(', ')}`
      : 'Send a test alert to the configured Slack receiver without saving'

  const handleTest = async () => {
    setTestStatus(null)
    setTesting(true)
    try {
      const input = toInput(form)
      const res = await testAlertDraft(input)
      const failed = res.results.filter((r) => !r.success)
      if (failed.length === 0) {
        setTestStatus({
          kind: 'ok',
          message: `Test alert delivered to ${res.succeeded}/${res.total} receiver(s).`,
        })
      } else if (res.succeeded > 0) {
        setTestStatus({
          kind: 'partial',
          message: `Delivered to ${res.succeeded}/${res.total}. Failed: ${failed
            .map((f) => `${f.receiver_name}${f.error ? ` (${f.error})` : ''}`)
            .join('; ')}`,
        })
      } else {
        setTestStatus({
          kind: 'error',
          message: `All ${res.total} receiver(s) failed: ${failed
            .map((f) => `${f.receiver_name}${f.error ? ` (${f.error})` : ''}`)
            .join('; ')}`,
        })
      }
    } catch (err) {
      setTestStatus({
        kind: 'error',
        message: err instanceof Error ? err.message : String(err),
      })
    } finally {
      setTesting(false)
    }
  }

  return (
    <form className={styles.form} onSubmit={onSubmit}>
      <div className={styles.formBadgeSbom}>
        Fires when any term matches a finding in a workload's SBOM or vulnerability report
      </div>

      <div className={styles.formSectionTitle}>Identity</div>
      <div className={styles.grid}>
        <Field label="Name" required>
          <input
            className={styles.input}
            value={form.name}
            disabled={editing}
            onChange={(e) => set('name', e.target.value)}
            placeholder="deprecated-crypto"
            required
          />
        </Field>
        <Field label="Description">
          <input
            className={styles.input}
            value={form.description}
            onChange={(e) => set('description', e.target.value)}
            placeholder="Detects use of deprecated component"
          />
        </Field>
        <Field label="Enabled">
          <div className={styles.toggleRow}>
            <button
              type="button"
              role="switch"
              aria-checked={form.enabled}
              className={`${styles.toggleSwitch} ${form.enabled ? styles.toggleSwitchOn : ''}`}
              onClick={() => set('enabled', !form.enabled)}
            />
            <span
              className={`${styles.toggleLabel} ${form.enabled ? styles.toggleLabelOn : styles.toggleLabelOff}`}
            >
              {form.enabled ? 'Active' : 'Paused'}
            </span>
          </div>
        </Field>
      </div>

      <div className={styles.formSectionTitle}>Matchers</div>
      {form.terms.map((t, i) => (
        <div key={i}>
          {i > 0 && <div className={styles.termDivider}>OR</div>}
          <TermEditor
            index={i}
            term={t}
            onChange={(next) => setTerm(i, next)}
            onRemove={form.terms.length > 1 ? () => removeTerm(i) : undefined}
            clusterOptions={clusterOptions}
            showHeader={form.terms.length > 1}
          />
        </div>
      ))}
      <div className={styles.termActions}>
        <button
          type="button"
          className="btn-secondary btn-sm"
          onClick={addTerm}
          disabled={form.terms.length >= MAX_TERMS}
        >
          Add OR term
        </button>
        <span className={styles.fieldHint}>
          Terms are ORed. Fields inside a term are ANDed, and exclusions veto a match.
        </span>
      </div>

      <div className={styles.grid}>
        <Field label="Cooldown (seconds)" hint="Per-target re-fire interval">
          <input
            className={styles.input}
            value={form.cooldownSecs}
            onChange={(e) => set('cooldownSecs', e.target.value.replace(/[^0-9]/g, ''))}
            placeholder="3600"
          />
        </Field>
      </div>

      <div className={styles.formSectionTitle}>Slack receiver</div>
      <div className={styles.grid}>
        <Field label="Receiver name">
          <input
            className={styles.input}
            value={form.receiverName}
            onChange={(e) => set('receiverName', e.target.value)}
            placeholder="sec-team"
          />
        </Field>
        <Field label="Webhook URL" required>
          <input
            className={styles.input}
            type="url"
            value={form.webhookUrl}
            onChange={(e) => set('webhookUrl', e.target.value)}
            placeholder="https://hooks.slack.com/services/…"
            required
          />
        </Field>
        <Field
          label="Channel override"
          hint="Sends to this channel instead of the webhook default. Leave blank to use the webhook's preset channel. Only legacy / custom-app webhooks honor this; standard incoming webhooks ignore it."
        >
          <input
            className={styles.input}
            value={form.channel}
            onChange={(e) => set('channel', e.target.value)}
            placeholder="#sec-alerts (optional)"
          />
        </Field>
      </div>

      <PreviewSection form={form} />

      {testStatus && (
        <div
          className={
            testStatus.kind === 'ok'
              ? styles.testBannerOk
              : testStatus.kind === 'partial'
                ? styles.testBannerPartial
                : styles.testBannerError
          }
        >
          {testStatus.message}
          <button
            type="button"
            className={styles.testBannerDismiss}
            onClick={() => setTestStatus(null)}
            aria-label="Dismiss"
          >
            ×
          </button>
        </div>
      )}

      <div className={styles.formActions}>
        <button type="submit" className="btn-primary" disabled={!canManage}>
          {editing ? 'Save changes' : 'Create rule'}
        </button>
        <button
          type="button"
          className="btn-secondary"
          onClick={handleTest}
          disabled={!canTest}
          title={testDisabledReason}
        >
          {testing ? 'Sending…' : 'Send test alert'}
        </button>
        <button type="button" className="btn-secondary" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  )
}

interface TermEditorProps {
  index: number
  term: TermForm
  onChange: (t: TermForm) => void
  onRemove?: () => void
  clusterOptions: RegisteredCluster[]
  showHeader: boolean
}

function TermEditor({ index, term, onChange, onRemove, clusterOptions, showHeader }: TermEditorProps) {
  const set = <K extends keyof TermForm>(k: K, v: TermForm[K]) => onChange({ ...term, [k]: v })
  const isVuln = term.trigger === 'vulnerability'

  return (
    <div className={styles.termCard}>
      {showHeader && (
        <div className={styles.termHeader}>
          <span className={styles.termTitle}>Term {index + 1}</span>
          {onRemove && (
            <button type="button" className="btn-secondary btn-sm" onClick={onRemove}>
              Remove
            </button>
          )}
        </div>
      )}
      <div className={styles.grid}>
        <Field label="Trigger" hint="Which report this term watches">
          <select
            className={styles.select}
            value={term.trigger}
            onChange={(e) => set('trigger', e.target.value as Trigger)}
          >
            <option value="component">SBOM component present</option>
            <option value="vulnerability">Vulnerability found</option>
          </select>
        </Field>
        {isVuln && (
          <>
            <Field
              label="Vulnerability IDs"
              hint="Comma or space separated. Leave blank to match any ID."
            >
              <input
                className={styles.input}
                value={term.vulnIds}
                onChange={(e) => set('vulnIds', e.target.value)}
                placeholder="CVE-2021-44228, GHSA-jfh8-c2jp-5v3q"
              />
            </Field>
            <Field label="Minimum CVSS score" hint="Inclusive, 0 to 10. Unscored findings never match.">
              <input
                className={styles.input}
                inputMode="decimal"
                value={term.minScore}
                onChange={(e) => set('minScore', e.target.value.replace(/[^0-9.]/g, ''))}
                placeholder="9.0"
              />
            </Field>
            <Field label="Severities" hint="Click to toggle. None selected matches any severity.">
              <div className={styles.severityToggleRow}>
                {SEVERITIES.map((sev) => {
                  const on = term.severities.includes(sev)
                  return (
                    <button
                      key={sev}
                      type="button"
                      aria-pressed={on}
                      className={`${styles.severityPill} ${styles[`severity${sev}`]} ${styles.severityToggle} ${on ? '' : styles.severityToggleOff}`}
                      onClick={() =>
                        set(
                          'severities',
                          on ? term.severities.filter((s) => s !== sev) : [...term.severities, sev],
                        )
                      }
                    >
                      {sev}
                    </button>
                  )
                })}
              </div>
            </Field>
            <Field label="Fix available">
              <select
                className={styles.select}
                value={term.fixAvailable}
                onChange={(e) => set('fixAvailable', e.target.value as FixFilter)}
              >
                <option value="">Any</option>
                <option value="true">Only with a fixed version</option>
                <option value="false">Only without a fix</option>
              </select>
            </Field>
            <Field label="Exclude vulnerability IDs" hint="Accepted risks. Comma or space separated.">
              <input
                className={styles.input}
                value={term.excludeIds}
                onChange={(e) => set('excludeIds', e.target.value)}
                placeholder="CVE-2023-44487"
              />
            </Field>
          </>
        )}
        <Field
          label={isVuln ? 'Package name (optional)' : 'Package name'}
          required={!isVuln}
        >
          <input
            className={styles.input}
            value={term.packageName}
            onChange={(e) => set('packageName', e.target.value)}
            placeholder="log4j-core"
          />
        </Field>
        <Field label="Version expression" hint="<2.17.0  or  >=1.0.0,<2.0.0">
          <input
            className={styles.input}
            value={term.versionExpr}
            onChange={(e) => set('versionExpr', e.target.value)}
            placeholder="<1.0.0"
          />
        </Field>
        <Field label="Exclude packages" hint="Comma or space separated, case-insensitive.">
          <input
            className={styles.input}
            value={term.excludePackages}
            onChange={(e) => set('excludePackages', e.target.value)}
            placeholder="linux-libc-dev"
          />
        </Field>
        <Field
          label="Clusters"
          hint="Click to toggle. At least one cluster must be selected."
          required
          wide
        >
          <ClusterMultiSelect
            value={term.clusters}
            options={clusterOptions}
            onChange={(next) => set('clusters', next)}
          />
        </Field>
        <Field label="Namespace (optional)">
          <input
            className={styles.input}
            value={term.namespace}
            onChange={(e) => set('namespace', e.target.value)}
            placeholder="default"
          />
        </Field>
        <Field label="Exclude namespaces" hint="Comma or space separated.">
          <input
            className={styles.input}
            value={term.excludeNamespaces}
            onChange={(e) => set('excludeNamespaces', e.target.value)}
            placeholder="kube-system, monitoring"
          />
        </Field>
      </div>
    </div>
  )
}

function PreviewSection({ form }: { form: FormState }) {
  const [result, setResult] = useState<AlertPreviewResult | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  const ready = form.terms.every((t) => hasNarrowingMatcher(t) && scoreError(t) === null)
  const isVuln = form.terms.some((t) => t.trigger === 'vulnerability')
  const body = toMatcherFields(form)
  const matchersKey = JSON.stringify(body)

  useEffect(() => {
    if (!ready) {
      setResult(null)
      setError(null)
      setLoading(false)
      return
    }
    let cancelled = false
    const handle = setTimeout(async () => {
      setLoading(true)
      setError(null)
      try {
        const r = await previewAlert(body)
        if (!cancelled) setResult(r)
      } catch (e) {
        if (!cancelled) {
          setError(e instanceof Error ? e.message : String(e))
          setResult(null)
        }
      } finally {
        if (!cancelled) setLoading(false)
      }
    }, 350)
    return () => {
      cancelled = true
      clearTimeout(handle)
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [matchersKey, ready])

  return (
    <div className={styles.previewSection}>
      <div className={styles.previewHeader}>
        <span className={styles.previewTitle}>Preview · matches in current data</span>
        <span className={styles.previewMeta}>
          {!ready
            ? 'awaiting input'
            : loading
              ? 'evaluating…'
              : result
                ? `${result.total} match(es) across ${result.scanned_reports} scanned report(s)`
                : '—'}
        </span>
      </div>

      {!ready && (
        <div className={styles.previewEmpty}>
          Give every term a package name, version expression, or vulnerability condition to see
          which workloads would fire.
        </div>
      )}

      {ready && error && <div className={styles.previewError}>{error}</div>}

      {ready && !error && result && result.items.length === 0 && (
        <div className={styles.previewEmpty}>
          No current reports match these matchers. The rule will start firing once a matching
          report is received.
        </div>
      )}

      {ready && !error && result && result.items.length > 0 && (
        <>
          <table className={styles.previewTable}>
            <thead>
              <tr>
                <th>Status</th>
                <th>Cluster</th>
                <th>Namespace</th>
                <th>Workload</th>
                {isVuln && <th>Vulnerability</th>}
                {isVuln && <th>Severity</th>}
                <th>Package</th>
                <th>Version</th>
                {isVuln && <th>Fixed in</th>}
              </tr>
            </thead>
            <tbody>
              {result.items.map((m, i) => (
                <tr
                  key={`${m.cluster}|${m.namespace}|${m.name}|${m.vulnerability_id ?? ''}|${m.package}|${m.version}|${i}`}
                >
                  <td>
                    <span className={styles.firingBadge}>FIRING</span>
                  </td>
                  <td>{m.cluster}</td>
                  <td>{m.namespace}</td>
                  <td>{m.name}</td>
                  {isVuln && <td>{m.vulnerability_id ?? '—'}</td>}
                  {isVuln && (
                    <td>
                      {m.vulnerability_id ? (
                        <span
                          className={`${styles.severityPill} ${styles[`severity${m.severity ?? ''}`] ?? ''}`}
                        >
                          {m.severity || 'UNKNOWN'}
                          {m.score != null ? ` ${m.score.toFixed(1)}` : ''}
                        </span>
                      ) : (
                        '—'
                      )}
                    </td>
                  )}
                  <td>{m.package}</td>
                  <td>{m.version}</td>
                  {isVuln && <td>{m.vulnerability_id ? m.fixed_version || 'no fix' : '—'}</td>}
                </tr>
              ))}
            </tbody>
          </table>
          {result.truncated && (
            <div className={styles.truncatedNotice}>
              Showing the first {result.items.length} of {result.total} matches.
            </div>
          )}
        </>
      )}
    </div>
  )
}

interface ClusterMultiSelectProps {
  value: string[]
  options: RegisteredCluster[]
  onChange: (v: string[]) => void
}

function ClusterMultiSelect({ value, options, onChange }: ClusterMultiSelectProps) {
  const [search, setSearch] = useState('')

  const known = new Set(options.map((c) => c.name))
  const orphaned = value.filter((n) => !known.has(n))
  const merged: { name: string; server: string | null }[] = [
    ...options.map((c) => ({ name: c.name, server: c.server })),
    ...orphaned.map((n) => ({ name: n, server: null })),
  ]

  if (merged.length === 0) {
    return (
      <div className={styles.clusterSelectEmpty}>
        No registered clusters yet — add one in <strong>Admin → Clusters</strong>.
      </div>
    )
  }

  const q = search.trim().toLowerCase()
  const visible = q
    ? merged.filter(
        (c) =>
          c.name.toLowerCase().includes(q) ||
          (c.server ?? '').toLowerCase().includes(q),
      )
    : merged

  const visibleNames = visible.map((c) => c.name)
  const visibleSelectedCount = visibleNames.filter((n) => value.includes(n)).length
  const allVisibleSelected =
    visibleNames.length > 0 && visibleSelectedCount === visibleNames.length

  const toggle = (name: string) => {
    onChange(value.includes(name) ? value.filter((n) => n !== name) : [...value, name])
  }

  const selectAllVisible = () => {
    const next = new Set(value)
    visibleNames.forEach((n) => next.add(n))
    onChange(Array.from(next))
  }

  const clearVisible = () => {
    if (q) {
      onChange(value.filter((n) => !visibleNames.includes(n)))
    } else {
      onChange([])
    }
  }

  return (
    <div className={styles.clusterPicker}>
      <div className={styles.clusterPickerToolbar}>
        <input
          type="text"
          className={styles.clusterPickerSearch}
          placeholder="Search clusters…"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
        <button
          type="button"
          className={
            allVisibleSelected
              ? styles.clusterPickerToolbarBtnDanger
              : styles.clusterPickerToolbarBtn
          }
          onClick={allVisibleSelected ? clearVisible : selectAllVisible}
          disabled={visible.length === 0}
        >
          {allVisibleSelected ? 'Clear' : 'Select all'}
        </button>
      </div>

      <div className={styles.clusterPickerList}>
        {visible.length === 0 ? (
          <div className={styles.clusterPickerNoResults}>No clusters match "{search}"</div>
        ) : (
          visible.map((c) => {
            const checked = value.includes(c.name)
            return (
              <label
                key={c.name}
                className={
                  checked ? styles.clusterPickerRowSelected : styles.clusterPickerRow
                }
              >
                <input
                  type="checkbox"
                  className={styles.clusterPickerCheckbox}
                  checked={checked}
                  onChange={() => toggle(c.name)}
                />
                <div className={styles.clusterPickerInfo}>
                  <span className={styles.clusterPickerName}>
                    {c.name}
                    {c.server === null && (
                      <span className={styles.clusterPickerOrphanTag}>unregistered</span>
                    )}
                  </span>
                  {c.server && <span className={styles.clusterPickerServer}>{c.server}</span>}
                </div>
              </label>
            )
          })
        )}
      </div>

      <div className={styles.clusterPickerFooter}>
        <span>
          {value.length} of {merged.length} selected
        </span>
        <span>{value.length === 0 ? 'matching all clusters' : null}</span>
      </div>
    </div>
  )
}

function Field({
  label,
  required,
  hint,
  wide,
  children,
}: {
  label: string
  required?: boolean
  hint?: string
  wide?: boolean
  children: React.ReactNode
}) {
  return (
    <label className={wide ? styles.fieldFull : styles.field}>
      <span className={styles.fieldLabel}>
        {label}
        {required && <span className={styles.fieldRequired}>*</span>}
      </span>
      {children}
      {hint && <span className={styles.fieldHint}>{hint}</span>}
    </label>
  )
}
