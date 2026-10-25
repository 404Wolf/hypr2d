# hypr2d extension protocol v1

Connect to `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/hypr2d.sock`.
The plugin creates a mode-0600 Unix stream socket and removes it on unload.
Each connection owns a session. Its annotations, subscriptions and reservations
vanish on disconnect. Keep the socket open for as long as that state is needed.
There is no agent-specific vocabulary or process discovery.

Send one UTF-8 JSON object per newline, at most 65,536 bytes including newline:

```json
{"v":1,"id":1,"op":"annotate","key":"build","target":{"kind":"cell","monitor":0,"x":2,"y":3},"annotation":{"label":"Build","badge":"2","dot":true,"colour":"#FFCB6B","tooltip":"Two jobs running"}}
```

Every request has `v:1` and an unsigned `id` for correlation. Responses are
`{"v":1,"id":1,"ok":true,"data":{}}`, or `ok:false` and an `error` string.
Unsolicited events have `event` and no `id`; they can precede a reply.
Wrong versions and invalid requests receive an error without changing state.
A state response exceeding 64 KiB returns an error; reduce annotation state
before requesting the aggregate snapshot.
Oversized/unterminated frames or slow readers are disconnected. Limits: 64
connections, 256 annotations and 32 regions per connection, 128 outstanding
placements. Transport queue exhaustion closes the service and all sessions;
reload the plugin to reopen it. Clients should reconnect after plugin reload.
No polling, timers or external commands are involved in the service.

Targets:

- `{"kind":"cell","monitor":0,"x":2,"y":3}`: zero-based grid coordinates and
  Hyprland monitor id, using that monitor's configured dimensions/numbering.
- `{"kind":"special","name":"special:chat"}`: addressable special name.
- `{"kind":"window","address":"0x1234"}`: native window address from state.
  Addresses are ephemeral; validate against current state before reusing one.

Operations:

- `annotate`: `key`, `target`, `annotation`. Replaces that session's key.
  Annotation fields are optional `label`, `badge`, `dot` (default false),
  `colour` (`#RRGGBB`) and `tooltip`. Text is plain, never markup; each field
  and key is at most 512 bytes without control characters. Multiple sessions
  may annotate a target; the most recently connected session with an annotation
  wins, then lexicographically last key within that session. Disconnect exposes
  the previous annotation. Native app labels return when all annotations clear.
  Overview badges have a separate footer (first four characters) so long labels
  cannot hide them or the dot; full values remain in protocol state/tooltips.
- `reserve`: `key`, `monitor`, `row`, `rows`, `title`, `skip` (default false).
  Owns complete consecutive grid rows and displays a titled band. Overlapping
  reservations are rejected, including within a session. `skip:true` makes
  directional move/carry commands skip those rows; explicit goto/drop/placement
  can still address them. At least one row must remain navigable. Reservations
  are declarative; they do not create workspaces or move existing windows.
- `clear`: `key`. Removes that session's annotation and/or reservation; idempotent.
- `place`: `window`, `target` (cell or existing special). Silently moves a
  mapped window without following it. A successful response is sent only after
  the compositor action succeeds. It does not create specials, warp the pointer,
  or promise ownership of a native window. No implicit swap or resize.
- `subscribe`: `events:["activate","drop"]`. Replaces the subscription set.
  Receives callbacks only for this connection's reserved rows. Event data has
  `workspace`, `window` (empty for activation) and matching reservation `regions`
  keys. Activation includes native workspace changes; drop includes overview
  drops and extension placement. Subscriptions disappear on disconnect.
- `state`: returns `data.annotations` and `data.regions`, including session
  `owner` ids and per-session keys. These are also exposed in `hypr2d state`
  as `extensions`; socket2 emits `hypr2d>>extensions` on state changes, allowing
  Waybar to refresh only on events.

Example reservation and callback:

```json
{"v":1,"id":2,"op":"reserve","key":"tools","monitor":0,"row":7,"rows":1,"title":"Tools","skip":true}
{"v":1,"id":3,"op":"subscribe","events":["activate","drop"]}
```

```json
{"v":1,"ok":true,"event":"drop","data":{"workspace":"561","window":"0x1234","regions":["tools"]}}
```

`hypr2d-client` provides typed `Target`, `Annotation`, `Operation`, `Request` and
`Response`, plus `Client::instance(runtime, signature)`, `request(operation)`
and blocking `next_event()`. `request()` preserves events received while waiting
for its matching reply. Check `Response.ok`; a protocol error is not an IO error.
Drop `Client` to end the session. Callers own reconnect policy and timeouts.
