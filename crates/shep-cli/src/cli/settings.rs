//! Arguments for the settings verbs: `shep get <name>[.<path>]`, and the
//! `set` and `unset` that write the same addresses.
//!
//! One address, spelled [`CopyArgs`](crate::cli::CopyArgs)'s way: a sheep's
//! or a dog's name, then a path inside the settings that name holds. Nothing
//! here parses that address or its value: the verb module that owns the verb
//! does, through the one address parser `shep copy` already has.

/// Arguments to `shep get`.
///
/// Not [`SelectorArgs`](crate::cli::SelectorArgs): a read names one settings
/// holder, and `all`, a glob, `fold:<name>` or an id have no path to read a
/// path out of. The positional is required: bare `shep get` would have to
/// guess whether it means the whole flock or the key/value store, and the
/// store has its own verb.
#[derive(Debug, clap::Args)]
pub struct GetArgs {
    /// A sheep or a dog, and the path inside its settings, such as
    /// `api.max_memory`, `api.dogs.kelpie.git`, or `kelpie.worker`
    pub address: String,
}

#[cfg(test)]
mod tests {
    use crate::cli::{Cli, Commands};

    /// The address `shep get <argv>` parsed, or a panic naming the argv.
    fn address(argv: &[&str]) -> String {
        use clap::Parser as _;

        let full: Vec<&str> = ["shep", "get"]
            .into_iter()
            .chain(argv.iter().copied())
            .collect();
        let Commands::Get(args) = Cli::try_parse_from(&full)
            .unwrap_or_else(|err| panic!("{full:?} must parse: {err}"))
            .command
        else {
            panic!("{full:?} is not `shep get`")
        };
        args.address
    }

    /// fails if `get` stops taking one address, or stops accepting the four
    /// shapes an address has: a name alone, a sheep's field, a dog table key
    /// inside a sheep, and a table inside a dog's own section.
    #[test]
    fn get_takes_one_address_in_any_of_its_shapes() {
        assert_eq!(address(&["api"]), "api");
        assert_eq!(address(&["api.max_memory"]), "api.max_memory");
        assert_eq!(address(&["api.dogs.kelpie.git"]), "api.dogs.kelpie.git");
        assert_eq!(address(&["kelpie.worker"]), "kelpie.worker");
    }

    /// fails if bare `shep get` becomes accepted. It used to list the
    /// key/value store, and an operator who means that types `shep kv get`.
    #[test]
    fn bare_get_is_a_usage_error() {
        use clap::Parser;
        assert!(Cli::try_parse_from(["shep", "get"]).is_err());
    }

    /// fails if `get` grows a second positional. One address per read is what
    /// keeps `shep get api.max_memory` saying one thing.
    #[test]
    fn get_takes_exactly_one_address() {
        use clap::Parser;
        assert!(Cli::try_parse_from(["shep", "get", "api.max_memory", "web.max_memory"]).is_err());
    }
}
