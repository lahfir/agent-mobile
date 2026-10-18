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
        cli_note: "",
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
            "Serve the tools over MCP on stdio",
            Exec::Local(cmd::mcp_exec),
        )
    },
    read(
        Verb {
            guide: "Pass a row's `key` as `device` in later calls. A shut-down device boots on the first call that targets it, so never boot devices with simctl, adb, or emulator yourself. Filters combine with AND.",
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
                "List the simulators, emulators, and phones you can drive",
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
        guide: "Other verbs start a driver on demand, so you rarely need this. `--app` launches the app once the driver is up. The token prints only in a terminal; otherwise it is stored under `~/.agent-mobile/tokens/<device>`.",
        globals: &[Global::App],
        reply: ReplyKind::Text,
        ..verb(
            "serve",
            "Run a driver for one device in the foreground",
            Exec::Local(cmd::serve::exec),
        )
    },
    read(
        verb(
            "status",
            "Show the foreground app and the session's device",
            wire("status"),
        ),
        ReplyKind::Status,
        SESSION,
    ),
    read(
        Verb {
            guide: "Use it to start, or after a STALE_REF. Every action already returns the next tree, so do not call it after each action. On iOS `app` picks the app to read; on Android it only checks the foreground app, so use `launch` to change apps.",
            ..verb(
                "snapshot",
                "Read the screen as an accessibility tree with refs",
                wire("snapshot"),
            )
        },
        ReplyKind::Snapshot,
        SELECTING,
    ),
    Verb {
        args: &[TARGET],
        guide: "Prefer a ref. A point is measured from the app frame's top-left; to hit a node by point, tap its center (x + w/2, y + h/2), because `at=x,y` in the tree is its top-left corner.",
        ..verb("tap", "Tap a ref, or an x,y point", wire("tap"))
    },
    Verb {
        args: &[arg(
            "text",
            "Text to append; a leading ref-shaped word on the CLI selects the field",
            Kind::RefThenText,
        )],
        guide: "With a ref the text goes into that field; without one it goes to the focused field. End the text with a newline to press Return, for example to submit a search.",
        cli_note: "Text that starts with `-` needs `--` before it.",
        ..verb(
            "type",
            "Type text into a ref or the focused field",
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
        guide: "With a ref it swipes inside that node; without one it swipes the whole app. Swipe up to see content further down. To find a known item, use scroll_until_visible.",
        ..verb("swipe", "Swipe up, down, left, or right", wire("swipe"))
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
        guide: "Use it instead of a loop of swipes. Returns the tree that shows the node. Fails after 15 swipes, 40 s, or when the list stops moving.",
        ..verb(
            "scroll_until_visible",
            "Scroll until a node with this exact name is on screen",
            Exec::Local(cmd::scroll::exec),
        )
    },
    verb(
        "home",
        "Go to the home screen; returns its tree",
        wire("home"),
    ),
    Verb {
        args: &[TARGET],
        ..verb(
            "doubletap",
            "Double-tap a ref, or an x,y point",
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
                    except: Some(1.0),
                },
            ),
            flag(arg(
                "velocity",
                "Pinch velocity; omitted keeps the driver default",
                Kind::Number {
                    gt: None,
                    max: None,
                    default: None,
                    except: None,
                },
            )),
        ],
        ..verb("pinch", "Pinch on a ref to zoom", wire("pinch"))
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
                    except: None,
                },
            )),
        ],
        guide: "Use it to open a context menu; the menu is in the returned tree.",
        ..verb(
            "hold",
            "Press and hold a ref or an x,y point",
            Exec::Wire {
                route: "hold",
                timeout: Timeout::Hold,
            },
        )
    },
    Verb {
        guide: "Check the returned tree; some screens ignore back.",
        ..verb("back", "Go back one screen", wire("back"))
    },
    Verb {
        args: &[REF],
        ..verb("twofinger", "Tap a ref with two fingers", wire("twofinger"))
    },
    Verb {
        args: &[arg(
            "which",
            "Which sheet to open",
            Kind::Enum(&["notification"]),
        )],
        guide: "Earlier refs stop working. To return to your app, call `back` on Android, or `snapshot` with `app` on iOS.",
        ..verb("center", "Open the notification center", wire("center"))
    },
    Verb {
        args: &[arg(
            "app_id",
            "iOS bundle id or Android package, e.g. com.apple.mobilesafari or com.android.settings",
            Kind::AppId,
        )],
        guide: "The usual first call: it boots the device and starts the driver when needed, then returns the app's tree. A running app restarts; it keeps its data but loses unsaved state.",
        ..verb(
            "launch",
            "Open an app by its id, or restart it",
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
            guide: "Use it only to check how the screen looks; text and refs come from the tree.",
            cli_note: "Writes the PNG to the given path, or prints base64. A path cannot be combined with `--json`.",
            ..verb(
                "screenshot",
                "Capture the screen as a PNG",
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
        guide: "The driver keeps running and no tree comes back; take a snapshot to see the screen.",
        ..verb("stop", "Close the foreground app", wire("terminate"))
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
        guide: "Call it before you switch to another device. It succeeds when no session runs.",
        ..verb(
            "release",
            "Stop the driver session; the device keeps running",
            Exec::Local(cmd::release::exec),
        )
    },
    read(
        verb(
            "skills",
            "Print the agent guide (SKILL.md)",
            Exec::Local(cmd::skills::exec),
        ),
        ReplyKind::Text,
        &[],
    ),
];
