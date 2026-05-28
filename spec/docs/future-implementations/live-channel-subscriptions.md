# Future implementation: Live Channel Subscriptions

> How a consumer (a home-screen pulse widget, a clinician's live dashboard, an alarm rule) subscribes to an OHD data channel and receives a **push the moment a new sample lands** — low-latency live observation instead of polling.

## Status — deferred, post-v1

**This is not a v1 deliverable.** v1 reads are request/response (OHDC `QueryEvents`) and the only "push" is the relay's silent tunnel-wake (`relay/spec/notifications.md`). That is enough to *fetch* current data; it is not a live feed.

**What this doc is for**: reserving the design space so v1 doesn't have to be re-architected to add a live feed later. The point is to confirm that:

- The OHDC **channel** model (event types such as `measurement.heart_rate`) is the right unit to subscribe to.
- The **relay** data plane (the QUIC tunnel + WS attach + push) is the right transport to carry a live stream, with no new trust primitive.
- **Grants** are the right scoping mechanism — a subscriber may only stream channels its grant already permits to read.
- Nothing in the local-first model breaks: a device with no network keeps working; this is purely the *connected* live-observation layer on top.

Treat the rest as a brainstorm, not a contract.

## Motivating example

A home-screen **pulse widget**: instead of polling `QueryEvents` every N seconds (latency + battery + staleness), the widget **subscribes to the `measurement.heart_rate` channel** and the source pushes each new sample as it is written. The widget shows near-real-time pulse with minimal latency and no busy-polling. The same primitive serves a clinician's live vitals dashboard and on-device alarm rules ("notify if SpO₂ < 90").

## Design-space sketch

- **Subscribe API** — an OHDC `Subscribe(channel_filter, scope)` that opens a server→client stream of new `Event`s matching the filter (one channel, a set, or an event-type prefix). Modeled on the existing server-streaming RPCs (`QueryEvents` already streams); a subscription is the open-ended, tailing variant.
- **Transport** — rides the **relay** for remote consumers (the same tunnel/attach path CORD uses), terminated end-to-end (relay sees ciphertext). For an on-device consumer (a widget against the local core) it is a direct in-process stream — no relay.
- **Delivery / wake** — while the consumer is attached, samples stream live. When it is detached (widget process asleep, phone dozing), the source coalesces and a **push wake** (the relay FCM path, `relay/spec/notifications.md`) nudges it to re-attach and drain — so low-latency when foreground, eventually-delivered when not. A subscription carries a freshness/coalescing policy (every sample vs latest-only vs rate-limited).
- **Scope** — a subscription is bound to a grant's read scope (`ShareScope`); it streams only channels the grant permits, and a mid-life suspend/revoke ends the stream. Self-session (the owner) may subscribe to everything.
- **Backpressure + lifecycle** — bounded buffering, single-use stream tokens, TTL'd subscriptions that the consumer renews; a dropped tunnel re-subscribes from a cursor so no sample is missed across a reconnect.

### Write-driven fan-out: single vs bulk

The write path already carries enough signal for the notification layer to default to sane fan-out, so the writer doesn't have to reason about subscribers and a backfill doesn't melt anyone's stream.

- **Single-event write** — a `put_event` (OHDC `PutEvents` carrying one event) is an interactive, here-and-now write, so notify **every subscriber of that individual sample**. Fine-grained: each value pushed.
- **Bulk write** — a `put_events` carrying many events with a **bulk flag** set is a backfill/import. By default, for a given metric/channel, push only **one final value** (e.g. the latest/aggregate sample for that channel in the batch) per subscriber — so a 17k-event Health Connect backfill does not fan out 17k pushes per subscriber.
- **Subscriber opt-in** — a subscriber that genuinely wants **every** sample from a bulk write (not just the last) opts in at subscribe time via a per-subscription "all samples vs latest-only" preference. The choice lives with the consumer, not the writer — the writer never decides fan-out granularity for everyone.

So: interactive writes default to per-sample push, bulk imports coalesce to latest-per-channel, and any subscriber can override toward all-samples. (Brainstorm — exact flag shape and the coalescing key live with the `PutEvents`/`Subscribe` design when this is picked up.)

## Cross-references

- OHDC channels + event model — [`../design/storage-format.md`](../design/storage-format.md)
- Relay data plane (tunnel, attach, push-wake) — [`../../../relay/spec/relay-protocol.md`](../../../relay/spec/relay-protocol.md), [`../../../relay/spec/notifications.md`](../../../relay/spec/notifications.md)
- Grant scoping — [`../design/privacy-access.md`](../design/privacy-access.md)
- Device pairing (the write side; this is the read/observe side) — [`device-pairing.md`](device-pairing.md)
