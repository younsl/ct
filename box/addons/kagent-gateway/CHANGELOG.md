# Changelog

Notable changes to the kagent-gateway app (the container image), newest first. Entries are headed by the image version and its release date (KST).

## 0.4.1 (2026-11-25)

The typing indicator shows the agent's real progress instead of a generic "working" label.

### Fixed

- In channel threads Slack showed the indicator only as a generic "<App> working" label. The text now also goes out as the `loading_messages` entry, which Slack renders as the custom line.
- The status moved only every `CHAT_STATUS_INTERVAL`, so a turn of 5 to 15 seconds ended before its first refresh. Progress is now checked every second, and a change in the task state or running tools is shown on the next check. An unchanged status still refreshes every `CHAT_STATUS_INTERVAL`.
- Tools a model calls in parallel are all counted instead of only the last one. The indicator names the first tool and counts the rest, and the status message lists up to three.

### Changed

- Every indicator line names the agent in full and ends in one emoji tied to the phase (⏳ queued, 🚀 or 🚨 starting, 💬 or 🔍 working, 🔧 tools running, 🤔 unknown state).

## 0.4.0 (2026-11-25)

A mention is answered under Slack's native typing indicator instead of a status reply rewritten in place.

### Added

- The mention path shows Slack's typing indicator under the thread (`assistant.threads.setStatus`) while the agent works, and posts the answer as one fresh reply that clears it. Nothing lands in channel history during the turn, and the answer carries no "(edited)" label. The method needs only `chat:write`.
- The status names the tool the agent is running, read from the tool call parts kagent records in the task (1.0 and 0.x metadata keys). The legacy status message shows it too.
- `CHAT_STATUS_STYLE` (`assistant` by default, or `message`) selects the indicator or the legacy status message. A turn Slack refuses the indicator on falls back to the status message.

### Changed

- The default mention status moves from a rewritten status reply to the typing indicator. Set `CHAT_STATUS_STYLE=message` to keep the previous behavior.
- The indicator is refreshed at least every 60s, also while the turn waits for a slot, because Slack drops it two minutes after the last call.

## 0.3.1 (2026-11-18)

Dependency update for RUSTSEC advisories. No behavior change.

### Security

Only the crates with an advisory move, so the lockfile stays close to 0.3.0.

- url 2.5.9 (RUSTSEC-2026-0592), released after the first build of this version, so the image is rebuilt at the same version.
- rustls 0.23.46 (RUSTSEC-2026-0285, 0346, 0348, 0349), rustls-webpki 0.103.17 (RUSTSEC-2026-0382), webpki-roots 1.0.12 (RUSTSEC-2026-0381).
- tokio 1.54.1 for its unsound warning (RUSTSEC-2026-0386).
- The unmaintained warning for unsafe-libyaml remains, as it has no replacement release.
