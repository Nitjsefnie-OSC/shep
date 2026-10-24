//! Static shell completion scripts: `shep completions <shell>`.
//!
//! Static only: sheep and fold names are never completed, since that would
//! need a connected daemon at completion time and `clap_complete`'s dynamic
//! engine is still an unstable feature upstream.
//!
//! Adopted dogs are completed, with the subcommands their manifests declare,
//! off `shep.toml` and the manifest cache as they stand at generation time.
//! That is still static: adopting a dog later means generating the script
//! again.

use clap::CommandFactory;

use crate::adopted_help::{self, AdoptedVerb};
use crate::cli::{Cli, CompletionArgs};
use crate::exit::ExitCode;

/// Writes the completion script for `args.shell` to `out`
///
/// `dogs` are the registry's adopted dogs, added to a command tree built for
/// this script alone. Nothing parses it: `shep <dog>` still reaches the dog
/// through clap's unknown-subcommand path, so a manifest stays a description
/// of a verb shep already gave the dog rather than a way to claim one.
pub fn completions(
    out: &mut dyn std::io::Write,
    args: &CompletionArgs,
    dogs: &[AdoptedVerb],
) -> ExitCode {
    let mut command = adopted_help::with_completion_entries(Cli::command(), dogs);
    clap_complete::aot::generate(args.shell, &mut command, "shep", out);
    ExitCode::Success
}

#[cfg(test)]
mod tests {
    use clap_complete::aot::Shell;

    use super::*;

    #[test]
    fn completions_generate_a_named_script_for_every_supported_shell() {
        let mut scripts = Vec::new();
        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
            let mut buf = Vec::new();
            let code = completions(&mut buf, &CompletionArgs { shell }, &[]);
            assert_eq!(code, ExitCode::Success, "{shell}");
            let script = String::from_utf8(buf).unwrap();
            assert!(!script.is_empty(), "{shell} produced nothing");
            assert!(
                script.contains("shep"),
                "{shell} script must name the binary"
            );
            scripts.push((shell, script));
        }

        // Two real scripts never collide, so a pairwise compare is what
        // catches a `completions` that ignores `args.shell`.
        for i in 0..scripts.len() {
            for j in (i + 1)..scripts.len() {
                let (shell_i, script_i) = &scripts[i];
                let (shell_j, script_j) = &scripts[j];
                assert_ne!(
                    script_i, script_j,
                    "{shell_i} and {shell_j} produced identical scripts — args.shell is being ignored"
                );
            }
        }
    }

    #[test]
    fn completions_cover_the_visible_aliases() {
        let mut buf = Vec::new();
        completions(&mut buf, &CompletionArgs { shell: Shell::Bash }, &[]);
        let script = String::from_utf8(buf).unwrap();
        for verb in ["flock", "list", "ls", "bleats", "logs"] {
            assert!(script.contains(verb), "{verb} missing from the bash script");
        }
    }
}
