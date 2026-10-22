//! What [`install`](super::step_execution::install) runs once the unit is
//! written, and whether that includes starting it now.
//!
//! A shepherd already holding the target home makes the unit's own daemon
//! exit [`ExitCode::DaemonAlreadyRunning`](crate::exit::ExitCode::DaemonAlreadyRunning)
//! the moment it starts, so `install` enables the unit and leaves the start to
//! the next boot.

use super::step_execution::shell_quote;
use super::target_resolution::StartupPlan;
use super::unit_layout::unit_file_name;
use crate::cli::Init;
use shep_core::paths::ShepPaths;
use shep_daemon::boot::{self, BootError, Shepherd};
use std::path::Path;

/// One init-system command: the program and its arguments.
pub(super) type InitCommand = (&'static str, Vec<String>);

/// The commands that enable the unit, then the one that starts it when
/// `start` holds.
///
/// launchd has no enable step of its own: a plist under
/// `/Library/LaunchDaemons` is loaded at every boot, and `bootstrap` is
/// what loads it now.
pub(super) fn install_commands(plan: &StartupPlan, start: bool) -> Vec<InitCommand> {
    let unit = unit_file_name(plan);
    let mut commands = match plan.init {
        Init::Systemd => {
            let enable = if start {
                words(&["enable", "--now", &unit])
            } else {
                words(&["enable", &unit])
            };
            return vec![
                ("systemctl", words(&["daemon-reload"])),
                ("systemctl", enable),
            ];
        }
        Init::Launchd => Vec::new(),
        Init::Openrc => vec![("rc-update", words(&["add", &unit, "default"]))],
        Init::FreebsdRc => vec![("sysrc", vec![format!("{unit}_enable=YES")])],
        Init::OpenbsdRc => vec![("rcctl", words(&["enable", &unit]))],
    };
    if start {
        commands.push(start_command(plan));
    }
    commands
}

/// The command that starts the installed unit now.
fn start_command(plan: &StartupPlan) -> InitCommand {
    let unit = unit_file_name(plan);
    match plan.init {
        Init::Systemd => ("systemctl", words(&["start", &unit])),
        Init::Launchd => (
            "launchctl",
            words(&["bootstrap", "system", &plan.unit_path.display().to_string()]),
        ),
        Init::Openrc => ("rc-service", words(&[&unit, "start"])),
        Init::FreebsdRc => ("service", words(&[&unit, "start"])),
        Init::OpenbsdRc => ("rcctl", words(&["start", &unit])),
    }
}

/// What holds `home`'s pidfile lock, asked without creating the pidfile.
///
/// [`boot::daemon_liveness`] creates a missing pidfile, and under `sudo` it
/// would be root's at 0600, which the unit's daemon, running as the target
/// user, could never open. No pidfile means no shepherd has run there.
///
/// # Errors
/// - [`BootError::Io`] if the pidfile exists but could not be opened or read.
pub(super) fn running_shepherd(home: &Path) -> Result<Shepherd, BootError> {
    let paths = ShepPaths::at(home.to_path_buf());
    if !boot::pidfile(&paths).exists() {
        return Ok(Shepherd::Absent);
    }
    boot::daemon_liveness(&paths)
}

/// The notice for a unit enabled but not started, naming how to hand the
/// flock over to it without waiting for a reboot.
pub(super) fn not_started_notice(plan: &StartupPlan, shepherd: Shepherd) -> String {
    let holder = match shepherd {
        Shepherd::Running(pid) => format!("a shepherd (pid {pid})"),
        Shepherd::Absent | Shepherd::Booting => "a shepherd that is still starting".to_string(),
    };
    let home = shell_quote(&plan.spec.home.display().to_string());
    let (program, args) = start_command(plan);
    format!(
        "{holder} already runs on {home}, so the unit is enabled but not started; it takes over \
         at the next boot, or now: {exec} --home {home} kill && {program} {args}",
        exec = shell_quote(&plan.spec.exec.display().to_string()),
        args = args.join(" "),
    )
}

/// Owned copies of `args`, the shape an [`InitCommand`] carries.
fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(ToString::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::super::testing::plan_for_test;
    use super::*;
    use std::io::Write as _;
    use std::path::PathBuf;

    fn plan(init: Init) -> StartupPlan {
        StartupPlan {
            init,
            ..plan_for_test(Path::new("/home/deploy/.shep"))
        }
    }

    fn lines(commands: &[InitCommand]) -> Vec<String> {
        commands
            .iter()
            .map(|(program, args)| format!("{program} {}", args.join(" ")))
            .collect()
    }

    /// With nothing holding the home, every init system enables the unit
    /// and starts it.
    #[test]
    fn an_empty_home_enables_and_starts_the_unit() {
        let cases = [
            (
                Init::Systemd,
                vec![
                    "systemctl daemon-reload",
                    "systemctl enable --now shep-deploy.service",
                ],
            ),
            (
                Init::Launchd,
                vec!["launchctl bootstrap system /home/deploy/shep-deploy.service"],
            ),
            (
                Init::Openrc,
                vec![
                    "rc-update add shep-deploy.service default",
                    "rc-service shep-deploy.service start",
                ],
            ),
            (
                Init::FreebsdRc,
                vec![
                    "sysrc shep-deploy.service_enable=YES",
                    "service shep-deploy.service start",
                ],
            ),
            (
                Init::OpenbsdRc,
                vec![
                    "rcctl enable shep-deploy.service",
                    "rcctl start shep-deploy.service",
                ],
            ),
        ];
        for (init, expected) in cases {
            assert_eq!(
                lines(&install_commands(&plan(init), true)),
                expected,
                "{init:?}"
            );
        }
    }

    /// A unit started over a running shepherd exits 10 and, on systemd,
    /// restarts into the same lock every five seconds forever.
    #[test]
    fn a_held_home_enables_the_unit_without_starting_it() {
        let cases = [
            (
                Init::Systemd,
                vec![
                    "systemctl daemon-reload",
                    "systemctl enable shep-deploy.service",
                ],
            ),
            (Init::Launchd, vec![]),
            (
                Init::Openrc,
                vec!["rc-update add shep-deploy.service default"],
            ),
            (
                Init::FreebsdRc,
                vec!["sysrc shep-deploy.service_enable=YES"],
            ),
            (Init::OpenbsdRc, vec!["rcctl enable shep-deploy.service"]),
        ];
        for (init, expected) in cases {
            assert_eq!(
                lines(&install_commands(&plan(init), false)),
                expected,
                "{init:?}"
            );
        }
    }

    /// The notice is the only place the operator learns the unit is idle,
    /// so it has to carry a command that hands the flock over.
    #[test]
    fn the_notice_names_the_pid_and_a_pasteable_handover() {
        let notice = not_started_notice(&plan(Init::Systemd), Shepherd::Running(205_270));
        assert_eq!(
            notice,
            "a shepherd (pid 205270) already runs on /home/deploy/.shep, so the unit is enabled \
             but not started; it takes over at the next boot, or now: /usr/local/bin/shep \
             --home /home/deploy/.shep kill && systemctl start shep-deploy.service"
        );
    }

    /// Root creating the pidfile would leave it root-owned at 0600 in the
    /// target user's home, and the unit's daemon could then never boot.
    #[test]
    fn a_home_with_no_pidfile_is_absent_and_stays_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ShepPaths::at(dir.path().to_path_buf());
        std::fs::create_dir_all(&paths.pids).unwrap();
        assert_eq!(running_shepherd(&paths.home).unwrap(), Shepherd::Absent);
        assert!(
            !boot::pidfile(&paths).exists(),
            "the probe created a pidfile"
        );
    }

    /// The lock, not the file, is proof of life: the same check the unit's
    /// daemon fails on.
    #[test]
    fn a_held_pidfile_lock_is_a_running_shepherd() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ShepPaths::at(dir.path().to_path_buf());
        std::fs::create_dir_all(&paths.pids).unwrap();
        let path: PathBuf = boot::pidfile(&paths);
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"4242").unwrap();
        let _held =
            nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock).unwrap();
        assert_eq!(
            running_shepherd(&paths.home).unwrap(),
            Shepherd::Running(4242)
        );
    }
}
