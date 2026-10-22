# Architecture

agent-mobile is a Rust CLI and MCP server. A small driver runs on each device and serves the screen's accessibility tree and actions over local HTTP.

## Diagram

```
  AI agent                                  You
  (MCP, stdio)                              (CLI)
       |                                      |
       v                                      v
+-----------------------------------------------------------+
| agent-mobile (Rust)                                       |
|                                                           |
|  CLI              MCP server                              |
|                                                           |
|  core                                                     |
|    - formatter                                            |
|    - error registry                                       |
|    - device discovery                                     |
|    - serve                                                |
|    - token store                                          |
|    - wire client                                          |
+-----------------------------------------------------------+
  HTTP/1.1 + JSON, protocol v1, bearer token
               |                                  |
               v                                  v
+-----------------------------+    +-----------------------------+
| iOS driver                  |    | Android driver              |
| XCUITest runner             |    | APK: AccessibilityService   |
| + HTTP server               |    | + HTTP server               |
| tree: XCUIElement snapshots |    | tree: rootInActiveWindow    |
| 127.0.0.1 on simulator      |    | actions: performAction,     |
| LAN on physical iPhone      |    |   dispatchGesture           |
| port 8770                   |    | one owned adb forward       |
|                             |    | + loopback bridge           |
+-----------------------------+    +-----------------------------+
               |                                  |
               v                                  v
         iOS apps                          Android apps
```

## One call, step by step

1. The agent calls a verb through the CLI or the MCP server.
2. agent-mobile starts a driver if none runs. It boots the device, installs the driver, and saves the session.
3. agent-mobile sends `POST /<verb>` to the driver.
4. The driver acts on the app.
5. The driver reads the tree again until it settles.
6. The driver returns the new tree with fresh refs.
7. agent-mobile prints the tree.

## Components

| Part | Location | Owns |
|---|---|---|
| Driver | `drivers/ios`, `drivers/android` | The tree, the refs, the settle, and the auth. The live tree is on the device, so refs resolve there. |
| Core | `crates/core` | The contract, the text format, process control, the wire client, the state file, and the iOS host side. The core only formats a tree. |
| Android adapter | `crates/android` | ADB discovery, boot, install, token, forward, and bridge. |
| CLI and MCP server | `src/` | The CLI, the MCP server, and `src/platform/`, which normalizes platforms. `src/surface/verbs.rs` is the verb registry for both the CLI and MCP. |

- One driver process and one port per device. The state file maps each device to its port.
- The iOS host app is a minimal shell that the UI-test target needs. The driver does not use it.
- A physical iPhone needs the Mac alive for the whole session.

## Design rules

- One contract on iOS and Android: one envelope, one node schema, and `@<snapshot_id>:eN` refs.
- Refs fail loudly. A stale or ambiguous ref is an error. The driver never guesses.
- Every wait has a limit.
- No hidden retry, no fuzzy match, and no self-healing locators.
- Local by default. The simulator and all Android endpoints use loopback. A physical iPhone uses the LAN.
- The CLI is stateless. The driver is the long-lived process. There is no daemon or hub.

Out of scope: cloud device farms, WebView or CDP bridges, games and canvas UIs, tunnel logic in product code (a tunnel is an adapter script such as `drivers/ios/tunnel-cloudflared.sh`), and app store distribution.

## Wire protocol

Each call is an HTTP/1.1 `POST /<verb>` with a JSON body and these headers:

- `Authorization: Bearer <token>`. A missing or wrong token returns 401.
- `X-Agent-Mobile-Version: 1`. Both drivers refuse a different version with `BAD_REQUEST`.
- `Connection: close`.

A settled snapshot has this shape: `data: {app, snapshot_id, ref_count, complete, settled, reads, settle_ms, text, tree}`. Each action returns one, so an action needs no extra round trip.

| Verb | Body | Returns | Notes |
|---|---|---|---|
| `status` | none | `app, snapshot_id, device, os` | No tree and no settle step. |
| `launch` | `bundle_id` | settled snapshot | Cold-launches the app. It becomes the active app. |
| `terminate` | none | `terminated: app id` | The only verb that does not return a settled snapshot. See the Android notes below. |
| `snapshot` | `app?` | settled snapshot | iOS targets the named app. Android only checks that the package is already in front; `launch` changes apps. |
| `tap` | `ref`, or `x` and `y` | settled snapshot | `x,y` are points from the top-left corner of the app frame, not a 0 to 1 fraction. |
| `type` | `text`, `ref?` | settled snapshot | Taps `ref` first if given. Otherwise types into the current focus. |
| `swipe` | `direction`, `ref?` | settled snapshot | `direction` is up, down, left, or right, else `BAD_REQUEST`. Swipes `ref` if given, else the whole app. |
| `home` | none | settled snapshot | Goes to SpringBoard on iOS, or the active launcher on Android. |
| `doubletap` | `ref`, or `x` and `y` | settled snapshot | Target required. Same shapes as `tap`. |
| `pinch` | `ref`, `scale`, `velocity?` | settled snapshot | `ref` required. `scale` must be positive, finite, and `\|scale - 1\| >= 0.01`, else `BAD_REQUEST`. `velocity` defaults to 1.0 with the sign of the zoom. |
| `hold` | `ref` or `x` and `y`, `duration?` | settled snapshot | Target required. `duration` defaults to 1.0 and must be 0 < d <= 10, so the press and the settle fit the wire time limit, else `BAD_REQUEST`. A duration of the wrong type is `BAD_REQUEST`, not the default. |
| `back` | none | settled snapshot | A system edge swipe, with no target. Works in navigation stacks. |
| `twofinger` | `ref` | settled snapshot | `ref` required. Elements that XCTest cannot address (for example widgets) fail as `DRIVER_ERROR`. The driver stays up. |
| `center` | `which` | settled snapshot | `which` takes only `notification`. It opens the iOS Notification Center or the Android notification shade. Any other value is `BAD_REQUEST`. Old app refs stop working. |
| `screenshot` | none | `png_base64` | No settle step. Not a tree snapshot. |

`scroll_until_visible` has no wire verb (see CLI to wire).

**Android notes**

- `launch` and `terminate` run through the host bridge, with serial-scoped `cmd package`, `am force-stop`, and `am start`.
- `terminate` refuses the driver itself, System UI, and the current launcher.
- `terminate` stops the package that its authenticated `status` check saw. The check and the force-stop are not atomic. If the foreground app changes after the check, the request does not follow it.
- If System UI blocks the foreground, try `back` first and judge the returned snapshot. Use `launch` only as a cold-restart fallback, because it loses the app's transient state.

**Envelope**

- Success: `{version, ok, command, elapsed_ms, data}`.
- Failure: the same shape with `ok:false` and `error:{code, message}` in place of `data`. The error object has only `code` and `message`.
- A 401 for a bad token has no `command` and no `elapsed_ms`, because it returns before the command runs.
- `version` is `1` on both platforms. It changes only with a breaking contract change.
- Error codes: `STALE_REF, AMBIGUOUS_TARGET, BAD_REQUEST, UNKNOWN_COMMAND, UNAUTHORIZED, DRIVER_ERROR`. See Errors for the status and the agent action for each code.

**Node**

`role, name, value, ref_id, states, available_actions, native_id {kind, value}, bounds {x, y, width, height}, children`

- Each node has a `ref_id`, because mobile trees have tappable containers with no name. The text listing prints only named or interactive nodes, so this does not add tokens.
- `native_id.kind` is `ax_identifier` on iOS and `resource_id` on Android.

**Text listing**

With `Accept: text/plain`, the driver returns a header line plus one line per named or interactive node: `@id:eN role "name" value="..." at=x,y size=WxH [states]`. This helps when you call the driver directly (`am.sh`, curl). The core owns the text format. The CLI always asks for JSON and prints the same line format itself, also for `status`, `terminate`, and `screenshot`. Example from Calendar:

```
app=com.apple.mobilecal snapshot=@upii2see refs=127 settled=true reads=2 elapsed_ms=5101
@upii2see:e1 application "Calendar" at=0,0 size=440x956
```

**Settle**

After each action or snapshot, the driver reads the tree again every 150 ms. It hashes each node's type, integer frame, label, identifier, and value. It stops when three reads in a row match, or after 3 s. `settled` says which one happened. `reads` says how many reads it took. A `settled: false` reply still returns the last read, and that read is usable.

**Refs**

- Each snapshot makes refs as `@<snapshot_id>:e<N>` and drops all earlier refs. A ref works only against the snapshot that made it.
- An action finds its ref again on the live tree by element type, accessibility identifier, label, and frame. x, y, width, and height can differ by at most 1 pt.
- A snapshot-id mismatch or no live match returns `STALE_REF`. More than one live match returns `AMBIGUOUS_TARGET`. The driver never guesses, and nothing heals itself.
- Example: a ref to the empty Title field stops working after you type into it. The label changes from the placeholder to the identifier, and the width gets smaller. This is the contract at work. Take a new snapshot after any action that changes the target.

## CLI to wire

The verb registry in `src/surface/verbs.rs` defines all 21 commands. Each one sends the driver verb with the same name, with these exceptions:

| Command | Sends |
|---|---|
| `devices`, `serve`, `mcp`, `release`, `skills` | nothing; they run on the host |
| `stop` | `terminate` |
| `launch <app_id>` | `launch` with the wire field `bundle_id` |
| `scroll_until_visible` | `snapshot` and `swipe` in a loop. Exact name match, direction up by default, stops at the end of the list or after 15 swipes. |

- `tap`, `doubletap`, `hold`: one argument is a `ref`, two are `x y`.
- `type`: a first argument shaped like `@<id>:eN` is the `ref`. The other arguments join into `text`.
- `pinch` and `hold` check `scale` and `duration` on the host, in the CLI and in the MCP server, before the call. The CLI sets the wire time limit of `hold` from the duration.
- `--max-depth` runs in the core: it drops deeper nodes and sets `complete: false`.
- A transport failure with no envelope becomes a `DRIVER_ERROR`, exit code `1`.

## Errors

The HTTP status for each code comes from the driver's response builder.

| Code | HTTP | Meaning | Agent action |
|---|---|---|---|
| `STALE_REF` | 409 | The ref's snapshot id is not the current one, or no live element matches it. | Take a new snapshot, then retry with a new ref. |
| `AMBIGUOUS_TARGET` | 409 | More than one live element matches the ref. | Take a snapshot with a narrower root, or use a ref with a firmer `native_id`, then retry. |
| `BAD_REQUEST` | 409 | A required body field is missing, or a value such as `direction` is not valid. | Fix the request. Do not retry it unchanged. |
| `UNKNOWN_COMMAND` | 409 | The path is not a known verb. | Fix the client. Do not retry. |
| `UNAUTHORIZED` | 401 | The bearer token is missing or wrong. | Fix `AGENT_MOBILE_TOKEN`. Do not retry unchanged. |
| `DRIVER_ERROR` | 500 | An error the driver did not expect. | Retry once. Escalate if it happens again. |

## MCP server

Spec revisions: `2026-07-28` (current) and `2025-11-25` (legacy), at modelcontextprotocol.io/specification.

### Transport

- `agent-mobile mcp` serves stdio only. It has no HTTP listener.
- It uses newline-delimited JSON-RPC on `serde_json`, with no async runtime.
- Stdout carries only flushed JSON-RPC lines. Logs and panics go to stderr. The server exits on stdin EOF.
- `notifications/cancelled` gets no reply. The server drops the pending call and its response. A reader thread and a channel let a cancel arrive during blocking driver work.

### Protocol versions

- `2026-07-28` is stateless. It uses `server/discover`, the protocol version and client capabilities per request in `_meta`, `resultType: "complete"`, and the server identity in result `_meta`.
- `2025-11-25` uses the `initialize`, `notifications/initialized`, and `ping` handshake.
- The server advertises both revisions and only `tools: {}`. The tool list is static and deterministic. Discovery and list results carry `ttlMs` and `cacheScope: "public"`.

### Tools

- The registry in `src/surface/verbs.rs` owns the tool list, descriptions, schemas, annotations, and reply kinds. Each tool has the CLI command's name and arguments.
- `mcp` and `serve` are not tools. `mcp` is the transport. `serve` blocks for the life of the driver.
- `release` stops the active driver session. The device keeps running.
- Each device tool takes an optional `device` per call. It overrides the startup choice for that call only. It does not change the remembered default.
- The server runs one serve at a time. A call for another device while one is active names the serving device and tells the caller to `release`.
- The first tool call starts the driver lazily, as the CLI does.
- Input schemas are plain JSON Schema 2020-12 objects with `additionalProperties:false` and no root `anyOf` or `oneOf`.
- Annotations come from each verb's hints in the registry. The hint classes are read, action, stop, and release. `launch` force-stops and restarts the app, so it is destructive and not idempotent. Action tools can act on any target, a Delete button too, so they are also destructive.

### Replies

- Data replies declare an output schema and return matching `structuredContent`. Devices, status, snapshot or action, and stop (`terminate`) each use their reply-kind schema.
- Snapshot JSON has the metadata but not the tree. The framed compact listing in the text already has every ref. A JSON copy of the tree would about double the tokens per action.
- `screenshot` returns base64 image content with `mimeType: "image/png"`. `skills` and `release` have no output schema.

### Error replies

- Typed driver errors (`STALE_REF` and the others) return `isError:true` with the code and the next action. See Errors.
- Argument faults in `tools/call` return `isError:true` tool results that name the field, the problem, and the fix.
- Unknown tool names and malformed requests return JSON-RPC `-32602`. Unknown methods return `-32601`.
- Missing modern request metadata returns `-32602`. An unsupported version returns `-32022` with the supported versions.

### Safety

- App UI text is untrusted data. The server frames output as untrusted, strips control characters, blocks forged content boundaries, and limits node and reply sizes. Server instructions stay static.
- The server checks each argument (types, ranges, refs, and ids) and rejects unknown properties.
- The driver URL comes only from local discovery or configuration. The token comes only from the environment or configuration. Neither comes from tool arguments. Neither shows in replies or logs.
- One worker runs calls one at a time. There is no separate rate limiter.
- Each verb, startup too, stays under the client's 60 s tool timeout. The server returns a typed error before that time ends.

## Sessions and state

Everything lives under `~/.agent-mobile/`.

| File | Holds |
|---|---|
| `state.json` | Session rows keyed by `platform:id` (old iOS name keys still work): driver URL, serve pid, iOS runner pid or Android serial, forward, bridge, and APK data, the token file name, and the default device. It never holds a token. |
| `tokens/` | One `0600` file per session. The file holds the token itself. |
| `driver-<device>.log` | The driver-side log, written `0600` because it can show env values. |
| `serve.lock`, `boot.lock` | One serve at a time. One lazy boot at a time. |

- A second `serve` on a live device reports the URL, the pid, and the remedy.
- A dead `serve` leaves an orphaned runner (iOS) or forward row (Android). The next serve reaps exactly the recorded resource: the recorded iOS runner PID, or the Android serial and forward port. It never does a broad port or process sweep.
- On iOS, a foreign process that holds port 8770 fails fast and names the `lsof` remedy. Android uses ephemeral loopback ports, so there is nothing to conflict.

## Security

- Each `serve` makes a new bearer token. It goes into a `0600` file and prints once. It never goes into `state.json`, logs, or argv. No script has a default token.
- Fixture and curl helpers pass bearer headers through private files, not argv.
- Physical iPhone: the Mac-to-phone hop is plain HTTP on the LAN, and the phone binds all interfaces. A tunnel, such as `drivers/ios/tunnel-cloudflared.sh`, adds encryption on the way out. The driver itself has no TLS.
- Android: the agent endpoint is loopback only. The bridge and the owned `adb forward` keep device traffic on the host, behind an authorized adb transport.
- Whoever holds the URL and token has full UI control. There is no rate limit, expiry, allowlist, or audit log. Use a fresh token per session. Use trusted Wi-Fi for a physical iPhone, and a host you already trust for Android's authorized-adb transport.
