---
name: agent-mobile
description: Drive iOS simulators, Android emulators, and real phones by reading the screen's accessibility tree and acting on its refs. Use when a task asks to open, tap, type in, scroll, test, or check an app on a mobile device or simulator.
---

# agent-mobile

Use the agent-mobile MCP tools when you have them. Otherwise run the same verbs in a shell as `agent-mobile <verb>`; the names and arguments match.

## Start

1. Call `devices`, filtered by `platform` when the task names one. Copy the row's `key`.
2. Call `launch` with the `app_id` and `device: <key>`. A shut-down device boots and the driver starts on this call, which can take a minute.
3. Read the tree in the reply and act on its refs.
4. Read the tree that each action returns to confirm the action worked.

Never boot devices, open apps, or install anything with simctl, xcrun, adb, emulator, or `open`. agent-mobile does it.

## Rules

- Every action returns the next tree. Use it; do not call `snapshot` after each action.
- Refs look like `@<snapshot>:e<N>` and stop working after the next action. Use refs from the newest tree only.
- Prefer a ref to an x,y point. Use a point only when no node covers the target.
- To reach an item off screen, call `scroll_until_visible` with its exact name instead of a loop of swipes.
- To submit a field, such as a search box, end the typed text with a newline.
- Permission dialogs and alerts are in the tree. Tap their buttons by ref.
- Use `screenshot` only to check how the screen looks. Text and refs come from the tree.
- One driver session runs at a time. To switch devices, call `release`, then pass the new `device`.

Common app ids: Safari `com.apple.mobilesafari`, iOS Settings `com.apple.Preferences`, Android Settings `com.android.settings`.

## Errors

Each error names the cause and a `next:` step. Follow the step.

| Code | Do this |
|---|---|
| `STALE_REF` | Use a ref from the newest tree, or take a snapshot, then retry. |
| `AMBIGUOUS_TARGET` | Pick a ref with a firmer `native_id`, or tap the node's center as x,y. |
| `BAD_REQUEST` | Fix the field the message names. Do not retry unchanged. |
| `DRIVER_ERROR` | Take a snapshot to see the real state, then retry once. Report it if it repeats. |
| `UNAUTHORIZED` | Do not retry. Tell the user the driver token is wrong. |

- `settled=false`: the screen was still moving. The tree is usable.
- `complete=false`: the tree is cut short, so ref actions fail. Take a snapshot without `max_depth`.
- A prompt on the phone (USB debugging, Android "Restricted settings", the "Agent Mobile Driver" accessibility switch) needs a person. Stop and tell the user.
- A boot that times out writes its log to `~/.agent-mobile/driver-*.log`. Read it and report the cause.

## Show the device window

Devices boot without a window. To watch, the user sets `AGENT_MOBILE_HEADED=1`. Set it in the MCP server's environment, or as a prefix of a CLI command, before the device boots. An Android emulator that already runs without a window keeps running that way.

## Commands

The CLI prints each verb's full rules with `agent-mobile <verb> --help`. `--json` prints the raw reply. Exit codes: 0 ok, 1 driver failure, 2 usage error.

<!-- commands:start -->
- `mcp`: Serve the tools over MCP on stdio
- `devices [--platform <platform>] [--kind <kind>] [--state <state>]`: List the simulators, emulators, and phones you can drive
- `serve <device>`: Run a driver for one device in the foreground
- `status`: Show the foreground app and the session's device
- `snapshot`: Read the screen as an accessibility tree with refs
- `tap <ref> | <x> <y>`: Tap a ref, or an x,y point
- `type [<ref>] <text...>`: Type text into a ref or the focused field
- `swipe <direction> [<ref>]`: Swipe up, down, left, or right
- `scroll_until_visible <text> [<direction>]`: Scroll until a node with this exact name is on screen
- `home`: Go to the home screen; returns its tree
- `doubletap <ref> | <x> <y>`: Double-tap a ref, or an x,y point
- `pinch <ref> <scale> [--velocity <velocity>]`: Pinch on a ref to zoom
- `hold <ref> | <x> <y> [--duration <duration>]`: Press and hold a ref or an x,y point
- `back`: Go back one screen
- `twofinger <ref>`: Tap a ref with two fingers
- `center notification`: Open the notification center
- `launch <app_id>`: Open an app by its id, or restart it
- `screenshot [<output>]`: Capture the screen as a PNG
- `stop`: Close the foreground app
- `release`: Stop the driver session; the device keeps running
- `skills`: Print the agent guide (SKILL.md)

Global flags: `--device`, `--app`, `--max-depth`, `--json`.
<!-- commands:end -->
