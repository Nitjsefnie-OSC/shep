//! The clap command tree: [`Cli`], [`Commands`], and every argument struct.
//!
//! The whole parse surface of the `shep` binary, pure tier (spec §11): it
//! compiles and its tests run on every target, Windows included, so
//! `Cli::command().debug_assert()` and the alias tests in `verbs` cover a
//! platform that cannot build the rest of this crate.
//!
//! One submodule per group of verbs, each owning the argument structs
//! behind those verbs and the tests that pin their grammar. Every type is
//! re-exported here, so the rest of the crate names them
//! `crate::cli::<Name>` and never has to know which group a verb landed in.
//!
//! What stays in this module is what the whole tree shares: the top-level
//! `Cli`, the flags valid on every subcommand, and the two `ValueEnum`s a
//! `#[cfg(unix)]` module could not own.

mod dogs;
mod flock;
mod help;
mod hosts;
mod import;
mod kv;
mod logs;
mod settings;
mod sheep;
mod store;
mod system;
mod verbs;

use std::path::PathBuf;

use shep_core::destination::Destination;

use help::version_text;

// `crate::adopted_help` renders the registry's dogs into a copy of this
// template; the template itself is what ships and stays hand-written.
pub(crate) use help::HELP_TEMPLATE;

pub use dogs::{AdoptArgs, DisableArgs, DogArgs, DogsArgs, EnableArgs};
pub use flock::{FlockArgs, FoldArgs, LookoutArgs};
pub use hosts::{HostArgs, HostCommand};
pub use import::{ImportArgs, ImportCommand, ImportEnvArgs, ImportPm2Args};
pub use kv::{KvArgs, KvCommand, KvGetArgs, KvSetArgs, KvUnsetArgs};
pub use logs::{BarksArgs, BleatsArgs, FlushArgs, ReopenArgs};
// Two defaults whose only caller outside their own module is a test
// elsewhere in the crate: lib.rs builds a `FlockArgs` by hand, and
// commands/bleats.rs builds a `BleatsArgs`. A release build reaches neither
// through this path, which is what the allow is for.
#[allow(unused_imports)]
pub(crate) use flock::FOLLOW_INTERVAL_FLOOR_SECONDS;
#[allow(unused_imports)]
pub use logs::DEFAULT_BLEAT_LINES;
pub use settings::GetArgs;
pub use sheep::{
    CopyArgs, ResetMode, SelectorArgs, ServeArgs, SignalArgs, StartArgs, StockArgs, TriggerArgs,
    WhisperArgs,
};
pub use store::{SecretArgs, SecretCommand};
pub use system::{
    CompletionArgs, DaemonArgs, DaemonCmd, DevArgs, DialStdioArgs, InitArgs, RuntimeArgs,
    StartupArgs, StyleArgs,
};
pub use verbs::Commands;

/// The `shep` command line.
// `bin_name = "shep"` below is load-bearing, not decoration. Without it, clap
// renders every `Usage:` line from `argv[0]` rather than from `name` — so
// `shep-runtime --help` prints `Usage: shep-runtime runtime ...` and
// `shep-dev --help` prints `Usage: shep-dev dev ...` when both alias
// binaries are built and run with no override.
// Pinned so every rendering of a verb's own usage line reads `shep <verb>`
// regardless of which of the three `[[bin]]` targets produced it — the alias
// binaries are convenience entrypoints for exactly that invocation, not
// commands in their own right, and their own `--help` should say so.
//
// A `//` comment, not `///`, deliberately: clap renders a doc comment as
// `long_about`, so as a doc comment this paragraph WAS the opening of
// `shep --help` for three phases. See the test
// `the_top_level_help_carries_no_implementation_notes`.
#[derive(Debug, clap::Parser)]
#[command(
    name = "shep",
    bin_name = "shep",
    version,
    long_version = version_text(),
    about = "A process manager for your flock",
    propagate_version = true,
    help_template = HELP_TEMPLATE,
    after_help = "Run `shep help <command>` for one command, or `shep welcome` for the tour."
)]
pub struct Cli {
    /// Flags valid on every subcommand.
    #[command(flatten)]
    pub global: GlobalArgs,
    /// The verb being invoked.
    #[command(subcommand)]
    pub command: Commands,
}

/// Flags valid on every subcommand, folded into [`Cli`] via `#[command(flatten)]`.
#[derive(Debug, clap::Args)]
pub struct GlobalArgs {
    /// Output format
    #[arg(long, global = true, value_enum, default_value_t = Format::Table)]
    pub format: Format,
    /// Suppress non-essential output
    ///
    /// Currently narrows `bleats`' own notices (a dropped-events count, a
    /// daemon-shutdown notice, ...): diagnostics distinct from a sheep's
    /// own line or a real error, both of which still print regardless.
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// How much this invocation dresses up its output: `full`, `plain`, or
    /// `bare`
    ///
    /// Wins over `$SHEP_STYLE` and `shep.toml`'s `[style] level`. Omit to
    /// let those decide; `shep style` reports which one answered.
    // The precedence order above is `style::resolve`'s; this field is only
    // the flag's own place in it.
    #[arg(long, global = true, value_enum)]
    pub style: Option<crate::style::StyleLevel>,
    /// Reach another host's shepherd over ssh
    ///
    /// DEST is an ssh destination: `amp2`, `user@amp2`, or an alias from
    /// ~/.ssh/config. `flock`, `describe`, `fold`, `bleats`,
    /// `stop`, `restart`, `reload`, `delete`, `stock`, `enable`, `disable`,
    /// `signal`, `whisper`, `trigger` and `lookout` take it; any other verb
    /// refuses it rather than act on this machine. `describe` shows no
    /// secrets over it, and `enable` takes no hidden `--exec` form over it.
    /// `--home` still names this machine's home, never the host's.
    #[arg(long, global = true, value_name = "DEST", value_parser = parse_destination)]
    pub host: Option<String>,
    /// Talk to a different shepherd
    ///
    /// Mostly plumbing: `shep dev` sessions, a system-wide flock, tests. You
    /// almost certainly want the default, ~/.shep. Must be an absolute path:
    /// a relative one would name a different flock from every directory.
    // Declared last on purpose. It was the first global option anyone read,
    // which announced it as a choice when it is really the daemon's
    // data-root. `{options}` renders in declaration order and ignores
    // `help_heading`, so position is the only lever available here: a `Less
    // common` section would need `{all-args}`, which re-emits the
    // alphabetical command wall this template exists to replace.
    //
    // `//`, not `///`, for the same reason the note above `Cli` is one. This
    // paragraph shipped in `shep --help` for exactly one build before the
    // render was read.
    #[arg(long, global = true, env = "SHEP_HOME")]
    pub home: Option<PathBuf>,
}

/// `--host`'s value parser: the destination as written, once
/// [`Destination::parse`] accepts it.
///
/// Held as the `String` rather than the [`Destination`], so [`GlobalArgs`]
/// stays plain data that the adopted-dog path can build by hand.
fn parse_destination(raw: &str) -> Result<String, String> {
    Destination::parse(raw)
        .map(|dest| dest.as_str().to_owned())
        .map_err(|err| err.to_string())
}

/// `--format`'s two shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    /// Human-readable columns (the default).
    Table,
    /// A versioned JSON envelope, one object per invocation.
    Json,
}

/// Which init system a unit is written for.
///
/// Five variants, all constructible on every target: `--init` lets an
/// operator name one directly, which is also what lets a macOS machine
/// exercise the systemd, openrc and rc.d renderers at all. Selection without
/// the flag is `commands::startup::current_init` — a runtime probe on Linux,
/// where systemd and openrc share one target triple, and a compile-time fact
/// everywhere else, where nothing else the target could be exists.
///
/// It lives in `cli.rs` rather than beside the renderers because `cli.rs`
/// compiles on **every** target while `mod commands` is `#[cfg(unix)]`. A
/// field on `StartupArgs` naming a type from a unix-only module breaks
/// `cargo check --workspace --all-targets --all-features --target
/// x86_64-pc-windows-gnu`, which is a phase-gate command. `Format` above is
/// the precedent: a `clap::ValueEnum` the parse surface owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum Init {
    /// Linux + systemd: a unit file, `Type=notify`.
    Systemd,
    /// Linux + openrc: an `openrc-run` script. No readiness protocol — see
    /// the renderer's own doc.
    Openrc,
    /// macOS: a `LaunchDaemon` plist.
    Launchd,
    /// FreeBSD: an `/etc/rc.subr` script under `/usr/local/etc/rc.d`.
    FreebsdRc,
    /// OpenBSD: an `/etc/rc.d/rc.subr` script under `/etc/rc.d`.
    OpenbsdRc,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_defaults_to_table_and_accepts_json() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["shep", "flock"]).unwrap();
        assert_eq!(cli.global.format, Format::Table);
        let cli = Cli::try_parse_from(["shep", "--format", "json", "flock"]).unwrap();
        assert_eq!(cli.global.format, Format::Json);
    }

    /// fails if `--style` stops being optional (a run must still work with
    /// nothing said, falling through to `$SHEP_STYLE`/`shep.toml`/default —
    /// see `style::resolve`), or if a level clap now rejects or mis-parses.
    #[test]
    fn style_flag_defaults_to_unset_and_accepts_the_three_levels() {
        use crate::style::StyleLevel;
        use clap::Parser;

        let cli = Cli::try_parse_from(["shep", "flock"]).unwrap();
        assert_eq!(cli.global.style, None);

        for (raw, expected) in [
            ("full", StyleLevel::Full),
            ("plain", StyleLevel::Plain),
            ("bare", StyleLevel::Bare),
        ] {
            let cli = Cli::try_parse_from(["shep", "--style", raw, "flock"]).unwrap();
            assert_eq!(cli.global.style, Some(expected), "--style {raw}");
        }

        assert!(Cli::try_parse_from(["shep", "--style", "loud", "flock"]).is_err());
    }

    /// fails if `--host` stops being optional, stops being global (valid on
    /// either side of the verb), or stops accepting either spelling of its
    /// value.
    #[test]
    fn host_flag_defaults_to_unset_and_takes_a_destination() {
        use clap::Parser;

        let cli = Cli::try_parse_from(["shep", "flock"]).unwrap();
        assert_eq!(cli.global.host, None);

        for argv in [
            vec!["shep", "--host", "amp2", "flock"],
            vec!["shep", "--host=amp2", "flock"],
            vec!["shep", "flock", "--host", "amp2"],
            vec!["shep", "flock", "--host=amp2"],
        ] {
            let cli = Cli::try_parse_from(&argv).unwrap();
            assert_eq!(cli.global.host.as_deref(), Some("amp2"), "{argv:?}");
        }

        let cli = Cli::try_parse_from(["shep", "--host", "deploy@amp2", "flock"]).unwrap();
        assert_eq!(cli.global.host.as_deref(), Some("deploy@amp2"));
    }

    /// `--host` reaches ssh as an argument, so a value ssh would read as an
    /// option never gets as far as a `GlobalArgs`.
    #[test]
    fn host_flag_refuses_a_destination_ssh_would_read_as_an_option() {
        use clap::Parser;

        for argv in [
            vec!["shep", "--host=-oProxyCommand=x", "flock"],
            vec!["shep", "--host", "", "flock"],
            vec!["shep", "--host", "a b", "flock"],
        ] {
            let err = Cli::try_parse_from(&argv).expect_err(&format!("{argv:?}"));
            assert_eq!(
                err.kind(),
                clap::error::ErrorKind::ValueValidation,
                "{argv:?}"
            );
        }
    }

    /// `std::env::set_var` is `unsafe` in edition 2024 and this crate is
    /// `#![forbid(unsafe_code)]`, so nothing here can establish an ambient
    /// `$SHEP_HOME` and observe clap actually reading it. The next best
    /// thing, and the thing that actually matters for `$SHEP_HOME` to keep
    /// working, is pinning that clap was *told* to read it: if `env =
    /// "SHEP_HOME"` (`cli.rs:30`) is ever deleted, this fails.
    #[test]
    fn home_flag_is_wired_to_the_shep_home_env_var() {
        use clap::CommandFactory;
        let cmd = Cli::command();
        let home_arg = cmd
            .get_arguments()
            .find(|a| a.get_id().as_str() == "home")
            .expect("GlobalArgs::home must still be a flattened argument named `home`");
        assert_eq!(home_arg.get_env(), Some(std::ffi::OsStr::new("SHEP_HOME")));
    }
}
