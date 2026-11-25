# Slack status indicator

| Status | Category |
| --- | --- |
| implemented in 0.4.1 | integration |

## Summary

While a mention turn runs, the gateway shows Slack's native typing indicator under the thread instead of a status reply rewritten in place. The indicator names the agent, what it is doing (the task state or the tool it runs), and the elapsed time, and ends in one emoji. Nothing lands in channel history during the turn, and the answer is posted as one fresh reply that clears the indicator. It replaces the status message described in [Live status](slack-mention-invocation.md#live-status) as the default, which stays available as `CHAT_STATUS_STYLE=message` and as the fallback.

![Slack status indicator](../assets/slack-status-indicator.svg)

## Goals

- Show that a mention was taken the moment it is accepted, before a slot is held.
- Show live progress: the queued wait, the A2A task state, and the tool the agent runs.
- Leave nothing in channel history while the agent works, and post the answer without an "(edited)" label.
- Need no scope beyond `chat:write`, which the answer already requires.
- Keep the alert analysis path unchanged.

## Non-goals

- Signalling at channel level. The indicator lives in a thread, so the alert path keeps its reactions and investigating note.
- Streaming the answer text. The answer is posted once, when the turn ends.

## Slack API

The indicator is `assistant.threads.setStatus` with `channel_id`, `thread_ts`, `status`, and `loading_messages`.

| Behavior | Consequence for the gateway |
| --- | --- |
| In a channel thread Slack shows a bare `status` only as a generic "<App> working" label | The text also goes out as the single `loading_messages` entry, which Slack renders as the custom line |
| Slack drops the indicator two minutes after the last call | Refreshed at least every 60s (`INDICATOR_KEEPALIVE`), also while the turn waits for a slot |
| A reply from the app clears the indicator | The answer post is the normal clear. A reply that fails to post clears it explicitly with an empty `status` |
| The method needs only `chat:write` in channel threads | No new scope or app setting |
| `loading_messages` takes 1 to 10 entries. A line of 51 characters or more is refused with `invalid_arguments`, which the reference does not document | One entry per call, kept under 50 characters, see [Line format](#line-format) |

## Lifecycle

| Phase | Indicator | Slack call |
| --- | --- | --- |
| Mention accepted | `{agent} 슬롯 대기 중 ⏳` | `setStatus`. A refusal falls back to the status message for this turn |
| Waiting for a slot | Same line | `setStatus` every 60s |
| Agent running | Working line, see below | `setStatus` on a change, and every `CHAT_STATUS_INTERVAL` (capped at 60s) when unchanged |
| Answer ready | Cleared by the reply | `chat.postMessage` |
| Reply failed | Cleared | `setStatus` with an empty `status` |

The ticker looks at the agent's progress every second (`PROGRESS_CHECK`). A change in the task state or the running tools is shown on the next look, so a turn of 5 to 15 seconds still shows the tools it ran. An unchanged line is refreshed only once `CHAT_STATUS_INTERVAL` has passed, which moves the elapsed time and keeps the indicator alive. The ticker owns the Slack call, so the A2A polling task never waits on Slack. A failed refresh is logged as `failed to refresh the thread status` and costs only that refresh.

## Progress source

The A2A task carries the state (`submitted`, `working`, ...) and the tool call parts kagent records. The gateway reads the latest message that holds tool parts, takes its `function_call` names, and drops the ones a `function_response` in the same message already answered. The part type is matched under the 1.0 metadata key `kagent.dev/a2a/part-type` and the 0.x keys `kagent_type` and `adk_type`. A model that calls several tools in parallel yields all of them, deduplicated, in call order.

## Line format

`{agent} {phrase} ({elapsed}) {emoji}`, at most 49 UTF-16 units (`INDICATOR_MAX_LEN`).

- The agent is named in full, so a reader knows which agent answers without opening the policy.
- Every phrase is a short noun phrase ending in `중`, and the line closes with one Unicode emoji tied to the phase.
- Running tools name the first one and count the rest (`MAX_INDICATED_TOOLS = 1`). The status message still lists up to three (`MAX_LISTED_TOOLS`).
- A line over the limit drops the elapsed time first, then cuts the tool name with `…`, then cuts the text before the emoji, so the emoji always closes the line. Units are UTF-16 because an emoji outside the BMP counts twice there, the stricter reading of how Slack counts.

| Activity | Alert thread | Line |
| --- | --- | --- |
| Waiting for a slot | either | `aws-alert-triage-agent 슬롯 대기 중 ⏳` |
| Tools running | either | `aws-alert-triage-agent k8s_get_resources 실행 중 🔧` |
| Several tools running | either | `aws-alert-triage-agent prometheus_q… 외 2개 실행 중 🔧` |
| `submitted` | yes | `aws-alert-triage-agent 알람 분석 시작 중 (3초) 🚨` |
| `submitted` | no | `aws-alert-triage-agent 작업 시작 중 (3초) 🚀` |
| `working` | yes | `aws-alert-triage-agent 알람 확인 중 (1분 35초) 🔍` |
| `working` | no | `aws-alert-triage-agent 답변 준비 중 (1분 35초) 💬` |
| any other state | either | `aws-alert-triage-agent input-required 상태 확인 중 🤔` |

The raw task state is a protocol token, so only an unrecognised one is printed, where it helps debugging. The poll count is left out, since it is the elapsed time divided by `KAGENT_POLL_INTERVAL`.

## Fallback

`CHAT_STATUS_STYLE=message` keeps the 0.3 behavior: a `:mag:` status reply posted at once, rewritten with `chat.update`, and ending as the answer. With `assistant` (the default), a turn whose first `setStatus` Slack refuses falls back to the same status message, so the asker always sees the question was taken. A refusal on a later refresh does not switch styles mid-turn.

## Configuration

| Variable | Default | Effect |
| --- | --- | --- |
| `CHAT_STATUS_STYLE` | `assistant` | `assistant` for the indicator, `message` for the rewritten status reply |
| `CHAT_STATUS_INTERVAL` | `10s` | Refresh cadence of an unchanged line. The indicator caps it at 60s |

## Length limit

The method reference documents no length for `status` or a `loading_messages` entry. The limit comes from live calls on a workspace with an agent named `aws-alert-triage-agent`:

| Line | Characters | Result |
| --- | --- | --- |
| `... 답변을 준비하는 중... (경과 1초)` | 50 | accepted |
| `... 답변을 준비하는 중... (경과 11초)` | 51 | `invalid_arguments` |
| Any working line with a trailing emoji | 52 or more | `invalid_arguments` on every refresh, so only the queued line stayed up |

A byte limit does not fit: the accepted 50 character line is 82 bytes in UTF-8. Another Slack integration reports the same refusal with the detail "must be less than 51 characters". Lines are kept at 49 units to leave one unit of margin.
