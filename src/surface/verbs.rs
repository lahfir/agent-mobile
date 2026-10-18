//! The vocabulary: one entry per verb. Adding a verb, argument, or global
//! here updates the CLI, the MCP tool list, validation, and dispatch.

use super::{
    ACTION, Arg, Exec, Global, Hints, Kind, READ, ReplyKind, STOP, Surface, Timeout, Verb,
};
use crate::cmd;

const DIRECTIONS: &[&str] = &["up", "down", "left", "right"];
const DEVICE_PLATFORMS: &[&str] = &["ios", "android"];
const DEVICE_KINDS: &[&str] = &["simulator", "device", "emulator", "usb", "wireless"];
const SESSION: &[Global] = &[Global::Device];
const ACTING: &[Global] = &[Global::Device, Global::MaxDepth];
const SELECTING: &[Global] = &[Global::Device, Global::App, Global::MaxDepth];

const fn arg(name: &'static str, help: &'static str, kind: Kind) -> Arg {
    Arg {
        name,
        help,
        kind,
        required: true,
        flag: false,
    }
}

const fn optional(arg: Arg) -> Arg {
    Arg {
        required: false,
        ..arg
    }
}

const fn flag(arg: Arg) -> Arg {
    Arg {
        flag: true,
        ..optional(arg)
    }
}

const TARGET: Arg = arg(
    "target",
    "A ref, or an x y point in the app frame from its top-left",
    Kind::Point,
);
const REF: Arg = arg(
    "ref",
    "Ref from the most recent snapshot or action reply",
    Kind::Ref,
);

const fn wire(route: &'static str) -> Exec {
    Exec::Wire {
        route,
        timeout: Timeout::Default,
    }
}

const fn verb(name: &'static str, summary: &'static str, exec: Exec) -> Verb {
    Verb {
        name,
        summary,
        guide: "",
        args: &[],
        globals: ACTING,
        hints: ACTION,
        reply: ReplyKind::Snapshot,
        exec,
        surface: Surface::Both,
    }
}

const fn read(verb: Verb, reply: ReplyKind, globals: &'static [Global]) -> Verb {
    Verb {
        hints: READ,
        reply,
        globals,
        ..verb
    }
}

/// Every verb, in CLI help and MCP tool-list order.
pub const VERBS: &[Verb] = &[
    Verb {
        surface: Surface::Cli,
        hints: Hints {
            open_world: false,
            ..ACTION
        },
        reply: ReplyKind::Text,
        globals: SELECTING,
        ..verb(
            "mcp",
            "Serve tools over newline-delimited JSON-RPC on stdio",
            Exec::Local(cmd::mcp_exec),
        )
    },
    read(
        Verb {
            guide: "Filters combine with AND. Rows begin with a stable `key`, followed by name, platform, kind, OS, state, and serving status.",
            args: &[
                flag(arg(
                    "platform",
                    "Filter by platform",
                    Kind::Enum(DEVICE_PLATFORMS),
                )),
                flag(arg(
                    "kind",
                    "Filter by device kind",
                    Kind::Enum(DEVICE_KINDS),
                )),
                flag(arg(
                    "state",
                    "Filter by lowercase device state",
                    Kind::Text(1024),
                )),
            ],
            ..verb(
                "devices",
                "List reachable simulators, devices, and AVDs",
                Exec::Local(cmd::devices::exec),
            )
        },
        ReplyKind::Devices,
        &[],
    ),
    Verb {
        surface: Surface::Cli,
        args: &[arg(
            "device",
            "Device name, id, or stable key from `agent-mobile devices`",
            Kind::Text(1024),
        )],
        guide: "Accepts a name, raw id, or collision-free `platform:id` key (`ios:<udid>`, `android:avd:<name>`, `android:<serial>`). `--app` launches the app after the driver binds. Prints the URL and device; the token is shown only in a terminal and otherwise stored under `~/.agent-mobile/tokens/<device>`.",
        globals: &[Global::App],
        reply: ReplyKind::Text,
        ..verb(
            "serve",
            "Start a driver for one device in the foreground; prints the token once",
            Exec::Local(cmd::serve::exec),
        )
    },
    read(
        verb(
            "status",
            "Show the active app and session identity",
            wire("status"),
        ),
        ReplyKind::Status,
        SESSION,
    ),
    read(
        Verb {
            guide: "`--app` selects the target: iOS targets the bundle; Android only validates the foreground package, so use launch to change apps.",
            ..verb(
                "snapshot",
                "Capture the accessibility tree and mint refs",
                wire("snapshot"),
            )
        },
        ReplyKind::Snapshot,
        SELECTING,
    ),
    Verb {
        args: &[TARGET],
        guide: "Tap a node's center (x + w/2, y + h/2); `at=x,y` in the listing is its top-left origin.",
        ..verb(
            "tap",
            "Tap a ref, or an x y point from the app frame's top-left",
            wire("tap"),
        )
    },
    Verb {
        args: &[arg(
            "text",
            "Text to append; a leading ref-shaped word on the CLI selects the field",
            Kind::RefThenText,
        )],
        guide: "With a ref the driver writes that field's value directly; otherwise keystrokes go to the focused field (the field may not keep focus). A newline requests IME Return when the field exposes it (Android), otherwise it is appended literally. On the CLI, text starting with `-` needs `--` first.",
        ..verb(
            "type",
            "Append text to a ref or the focused field",
            Exec::Wire {
                route: "type",
                timeout: Timeout::Long,
            },
        )
    },
    Verb {
        args: &[
            arg("direction", "Direction to swipe", Kind::Enum(DIRECTIONS)),
            optional(REF),
        ],
        ..verb(
            "swipe",
            "Swipe up, down, left, or right, on a ref or the whole app",
            wire("swipe"),
        )
    },
    Verb {
        args: &[
            arg(
                "text",
                "Exact accessibility node name to find",
                Kind::Text(1024),
            ),
            optional(arg(
                "direction",
                "Swipe direction; defaults to up",
                Kind::Enum(DIRECTIONS),
            )),
        ],
        globals: SELECTING,
        guide: "Matches the exact node name. Direction defaults to up. Stops after 15 swipes, 40 s, or when the list stops moving; returns the snapshot that shows the node.",
        ..verb(
            "scroll_until_visible",
            "Swipe until an exact node name appears in the snapshot",
            Exec::Local(cmd::scroll::exec),
        )
    },
    verb(
        "home",
        "Go to the launcher/home; returns that tree",
        wire("home"),
    ),
    Verb {
        args: &[TARGET],
        ..verb(
            "doubletap",
            "Double-tap a ref, or an x y point from the app frame's top-left",
            wire("doubletap"),
        )
    },
    Verb {
        args: &[
            REF,
            arg(
                "scale",
                "Zoom scale; above 1 zooms out, below 1 zooms in; near-1 values fail",
                Kind::Number {
                    gt: Some(0.0),
                    max: None,
                    default: None,
                },
            ),
            flag(arg(
                "velocity",
                "Pinch velocity; omitted keeps the driver default",
                Kind::Number {
                    gt: None,
                    max: None,
                    default: None,
                },
            )),
        ],
        guide: "Above 1 zooms out and below 1 zooms in. Near-1 or non-positive scales are rejected; non-finite scale or velocity is invalid.",
        ..verb("pinch", "Pinch-zoom on a ref", wire("pinch"))
    },
    Verb {
        args: &[
            TARGET,
            flag(arg(
                "duration",
                "Hold duration in seconds, 0 < d <= 10",
                Kind::Number {
                    gt: Some(0.0),
                    max: Some(10.0),
                    default: Some(1.0),
                },
            )),
        ],
        guide: "The duration is 0 < d <= 10 seconds, default 1.0. Context menus appear in the returned snapshot; web long-press is unproven.",
        ..verb(
            "hold",
            "Press a ref or point for a duration; surfaces context menus",
            Exec::Wire {
                route: "hold",
                timeout: Timeout::Hold,
            },
        )
    },
    Verb {
        guide: "Judge the result from the returned tree.",
        ..verb("back", "Navigate back; no target", wire("back"))
    },
    Verb {
        args: &[REF],
        ..verb("twofinger", "Two-finger tap on a ref", wire("twofinger"))
    },
    Verb {
        args: &[arg(
            "which",
            "Which sheet to open",
            Kind::Enum(&["notification"]),
        )],
        guide: "Earlier refs die. Take a snapshot to return to your app (iOS also accepts `--app`). On Android, back out of the shade first; `snapshot --app` fails while System UI holds the foreground.",
        ..verb(
            "center",
            "Open notifications from the system UI session",
            wire("center"),
        )
    },
    Verb {
        args: &[arg(
            "app_id",
            "iOS bundle id or Android package, e.g. com.apple.mobilecal",
            Kind::AppId,
        )],
        guide: "Keeps app data but loses transient state. On Android, terminate refuses the driver, System UI, and the current launcher.",
        ..verb(
            "launch",
            "Cold-start or restart an app",
            Exec::Wire {
                route: "launch",
                timeout: Timeout::Long,
            },
        )
    },
    read(
        Verb {
            args: &[optional(arg(
                "output",
                "PNG file to write; omitted prints base64",
                Kind::Path,
            ))],
            guide: "Writes a PNG to the supplied path or prints base64 to stdout. With `--json`, base64 stays in the envelope; a path cannot be combined with `--json`.",
            ..verb(
                "screenshot",
                "Capture a PNG screenshot",
                Exec::Local(cmd::screenshot::exec),
            )
        },
        ReplyKind::Image,
        SESSION,
    ),
    Verb {
        hints: STOP,
        reply: ReplyKind::Terminate,
        globals: SESSION,
        guide: "The driver stays up and no tree comes back; take a snapshot to verify.",
        ..verb("stop", "Terminate the active app", wire("terminate"))
    },
    Verb {
        globals: SESSION,
        hints: Hints {
            read_only: false,
            destructive: true,
            idempotent: true,
            open_world: false,
        },
        reply: ReplyKind::Text,
        guide: "If no session is running, reports that and succeeds.",
        ..verb(
            "release",
            "Stop the running driver session; the device stays up",
            Exec::Local(cmd::release::exec),
        )
    },
    read(
        verb(
            "skills",
            "Print the one-page agent guide",
            Exec::Local(cmd::skills::exec),
        ),
        ReplyKind::Text,
        &[],
    ),
];
