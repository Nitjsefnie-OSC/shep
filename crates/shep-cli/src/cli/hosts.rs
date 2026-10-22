//! Arguments for `shep host`: the list of machines whose shepherds this CLI
//! reaches, kept in `shep.toml`.

/// Arguments to `shep host`.
#[derive(Debug, clap::Args)]
pub struct HostArgs {
    /// What to do to the list.
    #[command(subcommand)]
    pub command: HostCommand,
}

/// `shep host`'s subcommands.
#[derive(Debug, clap::Subcommand)]
pub enum HostCommand {
    /// Add a machine to the list
    ///
    /// A destination is what you would hand to `ssh`: a host name or an
    /// `~/.ssh/config` alias, optionally with a `user@` in front. Letters,
    /// digits, `.`, `_` and `-` only, and neither part may start with `-`.
    /// Ports, jump hosts and IPv6 literals go in `~/.ssh/config`, where
    /// `Port` and `ProxyJump` already apply.
    ///
    /// The remote `shep` must be on the PATH that a non-interactive ssh
    /// session sees. Adding a host that is already listed changes nothing.
    Add {
        /// The ssh destination, such as `amp2` or `deploy@build.example.com`
        destination: String,
    },
    /// Remove a machine from the list
    ///
    /// The destination is compared as written. Exits 4 when it is not listed.
    Remove {
        /// The ssh destination, exactly as listed
        destination: String,
    },
    /// List the machines, in the order `shep.toml` holds them
    ///
    /// Reads `shep.toml` only and never connects to a machine.
    List,
}
