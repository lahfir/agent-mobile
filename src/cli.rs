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
    /// Target bundle id; sets `app` on snapshot.
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
    /// Type text; a leading ref-shaped arg taps that element first.
    Type {
        /// Optional `@<snapshot>:e<N>` ref, then the text to type. Use `--`
        /// before text that starts with a hyphen.
        #[arg(num_args = 1.., allow_hyphen_values = true, value_name = "[REF] TEXT")]
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
