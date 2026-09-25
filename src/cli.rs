//! Command-line surface (KTD16): derive parser, global flags, and the
//! argument shapes each verb validates in code.

use clap::{Parser, Subcommand};

/// Drive iOS apps through a snapshot -> act loop.
#[derive(Debug, Parser)]
#[command(name = "agent-mobile", version, about)]
pub struct Cli {
    /// Emit the raw JSON envelope instead of text.
    #[arg(long, global = true)]
    pub json: bool,
    /// Target bundle id; `serve` launches it once the driver binds.
    #[arg(long, global = true, value_name = "BUNDLE")]
    pub app: Option<String>,
    /// Drop tree nodes deeper than N levels; marks the snapshot incomplete.
    #[arg(long, global = true, value_name = "N")]
    pub max_depth: Option<u32>,
    /// Device to target; remembered for later invocations.
    #[arg(long, global = true, value_name = "NAME")]
    pub device: Option<String>,
    /// The verb to run.
    #[command(subcommand)]
    pub command: Command,
}

/// One CLI verb per variant; `devices` is CLI-side only.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List reachable simulators and paired devices.
    Devices,
    /// Start a driver for one device in the foreground; prints the token once.
    Serve {
        /// Device name or UDID from `agent-mobile devices`.
        device: String,
    },
    /// Show the active app and session identity.
    Status,
    /// Capture the accessibility tree and mint refs.
    Snapshot,
    /// Tap a ref, or an x y point from the app frame's top-left.
    Tap {
        /// One `@<snapshot>:e<N>` ref, or two float coordinates.
        #[arg(num_args = 1..=2, value_name = "REF | X Y")]
        args: Vec<String>,
    },
    /// Append text; a leading ref-shaped arg writes that field directly, else the focused field gets keystrokes.
    Type {
        /// Optional `@<snapshot>:e<N>` ref, then the text to type. Text that
        /// begins with a hyphen needs `--` first: `type -- -flag`.
        #[arg(num_args = 1.., value_name = "[REF] TEXT")]
        args: Vec<String>,
    },
    /// Swipe up, down, left, or right — on a ref, or the whole app.
    Swipe {
        /// Direction to swipe.
        #[arg(value_parser = ["up", "down", "left", "right"])]
        direction: String,
        /// Optional `@<snapshot>:e<N>` ref to swipe on.
        target: Option<String>,
    },
    /// Press the Home button; returns the springboard tree.
    Home,
    /// Double-tap a ref, or an x y point from the app frame's top-left.
    Doubletap {
        /// One `@<snapshot>:e<N>` ref, or two float coordinates.
        #[arg(num_args = 1..=2, value_name = "REF | X Y")]
        args: Vec<String>,
    },
    /// Pinch-zoom on a ref; scale above 1 zooms out, below 1 zooms in.
    Pinch {
        /// `@<snapshot>:e<N>` ref to pinch on.
        target: String,
        /// Zoom scale; near-1 or non-positive values are rejected
        /// driver-side as `BAD_REQUEST`. Non-finite values are rejected
        /// here: JSON cannot carry them.
        scale: f64,
        /// Pinch velocity; omitted leaves the driver default in place.
        #[arg(long, value_name = "V")]
        velocity: Option<f64>,
    },
    /// Press a ref or point for a duration; surfaces context menus.
    Hold {
        /// One `@<snapshot>:e<N>` ref, or two float coordinates.
        #[arg(num_args = 1..=2, value_name = "REF | X Y")]
        args: Vec<String>,
        /// Hold duration in seconds; the driver rejects values outside
        /// 0 < d <= 10. Negative numbers must reach the driver, so the
        /// flag accepts them instead of parsing them as flags.
        #[arg(
            long,
            default_value_t = 1.0,
            value_name = "SECS",
            allow_negative_numbers = true
        )]
        duration: f64,
    },
    /// System edge swipe back; no target.
    Back,
    /// Two-finger tap on a ref.
    Twofinger {
        /// `@<snapshot>:e<N>` ref to tap with two fingers.
        target: String,
    },
    /// Open Notification Center from the `SpringBoard` session.
    Center {
        /// Which sheet to open; only `notification` is supported.
        #[arg(value_parser = ["notification"])]
        which: String,
    },
    /// Cold-start a bundle id; kills any saved app state.
    Launch {
        /// Bundle id to launch, e.g. `com.apple.mobilecal`.
        bundle_id: String,
    },
    /// Write a PNG screenshot to a path, or base64 to stdout.
    Screenshot {
        /// File to write; omitted prints base64.
        output: Option<String>,
    },
    /// Terminate the active app; the driver stays up.
    Stop,
    /// Print the one-page agent guide.
    Skills,
}

impl Command {
    /// The verb name — for failure envelopes and ignore-notes.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Devices => "devices",
            Self::Serve { .. } => "serve",
            Self::Status => "status",
            Self::Snapshot => "snapshot",
            Self::Tap { .. } => "tap",
            Self::Type { .. } => "type",
            Self::Swipe { .. } => "swipe",
            Self::Home => "home",
            Self::Doubletap { .. } => "doubletap",
            Self::Pinch { .. } => "pinch",
            Self::Hold { .. } => "hold",
            Self::Back => "back",
            Self::Twofinger { .. } => "twofinger",
            Self::Center { .. } => "center",
            Self::Launch { .. } => "launch",
            Self::Screenshot { .. } => "screenshot",
            Self::Stop => "stop",
            Self::Skills => "skills",
        }
    }
}
