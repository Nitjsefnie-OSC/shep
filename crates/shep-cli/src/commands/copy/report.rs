//! What a copy prints: the change list, and the run's verdict.
//!
//! The table form prints the change list on stdout, and asks about it after.
//! A form whose stdout is a JSON envelope prints one object per invocation,
//! which is what `--format json` keeps.

use crate::cli::Format;
use crate::exit::ExitCode;
use crate::output::{CopyChange, CopyReport, Streams, emit, write_outcome};

use super::address::Address;
use super::plan::Plan;

/// The report a plan renders, before or after the write.
pub(crate) fn report_for(
    from: &Address,
    to: &Address,
    plan: &Plan,
    written: bool,
    dry_run: bool,
) -> CopyReport {
    CopyReport {
        from: from.sheep.clone(),
        to: to.sheep.clone(),
        path: from.path.dotted(),
        written,
        dry_run,
        pending: Vec::new(),
        changes: plan
            .changes
            .iter()
            .map(|change| CopyChange {
                key: change.key.clone(),
                change: change.kind.as_str(),
                old: change.old.clone(),
                new: change.new.clone(),
            })
            .collect(),
    }
}

/// Prints the change list in the table form.
///
/// # Errors
/// [`ExitCode::Failure`] if the write itself failed.
pub(crate) fn print_changes(
    streams: &mut Streams<'_>,
    report: &CopyReport,
) -> Result<(), ExitCode> {
    emit(
        &mut *streams.out,
        Format::Table,
        "copy",
        report.clone(),
        streams.style,
    )
    .map_err(|_| ExitCode::Failure)
}

/// Prints the run's verdict.
///
/// The table form already printed the change list, so this adds nothing to it.
/// The JSON form prints the one envelope this invocation owes, with `written`
/// and `pending` filled in by what actually happened.
pub(crate) fn finish(streams: &mut Streams<'_>, report: &CopyReport) -> ExitCode {
    if streams.fmt != Format::Json {
        return ExitCode::Success;
    }
    write_outcome(emit(
        &mut *streams.out,
        Format::Json,
        "copy",
        report.clone(),
        streams.style,
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::commands::copy::address::ConfigPath;
    use crate::commands::copy::plan::{Change, ChangeKind};
    use crate::output::Render;

    /// The two sides of every report test here.
    fn sides(dog: &str) -> (Address, Address) {
        let path = ConfigPath::Dog {
            dog: dog.to_string(),
            keys: Vec::new(),
        };
        (
            Address {
                sheep: "web".to_string(),
                path: path.clone(),
            },
            Address {
                sheep: "api".to_string(),
                path,
            },
        )
    }

    /// A report prints its changes and its verdict.
    #[test]
    fn a_report_prints_its_changes_and_its_verdict() {
        let (from, to) = sides("kelpie");
        let plan = Plan {
            changes: vec![Change {
                key: "rulings.decide".to_string(),
                kind: ChangeKind::Changed,
                old: Some(json!("issue")),
                new: Some(json!("panel")),
            }],
            write: None,
        };
        let report = report_for(&from, &to, &plan, true, false);
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["from"], "web");
        assert_eq!(json["to"], "api");
        assert_eq!(json["path"], "dogs.kelpie");
        assert_eq!(json["written"], true);
        assert_eq!(json["dry_run"], false);
        assert_eq!(json["changes"][0]["key"], "rulings.decide");
        assert_eq!(json["changes"][0]["change"], "changed");
        assert_eq!(json["changes"][0]["old"], "issue");
        assert_eq!(json["changes"][0]["new"], "panel");
        assert_eq!(
            report.rows(),
            vec![vec![
                "rulings.decide".to_string(),
                "changed".to_string(),
                "issue".to_string(),
                "panel".to_string(),
            ]]
        );
    }
}
