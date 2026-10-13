//! A Flockfile re-run and a sheep `shep disable` parked: the file never
//! brings it back, and the rest of the file still loads.

use super::*;

use shep_core::status::ProcStatus;

fn a_disabled_zam() -> ProcessInfo {
    ProcessInfo::builder(0, "zam", ProcStatus::Stopped)
        .disabled(Some(true))
        .build()
}

/// A fake shepherd holding `flock`, answering a `Start` with one online row
/// per app sent
fn a_shepherd_holding(flock: Vec<ProcessInfo>) -> impl Fn(&Request) -> Response + Send + 'static {
    move |request| match request {
        Request::ListFlock => Response::Flock(flock.clone()),
        Request::ApplyConfig { apps, .. } => Response::Applied(
            apps.iter()
                .map(|app| SheepApplied::new(app.config.name.clone(), Vec::new(), Vec::new(), None))
                .collect(),
        ),
        Request::Start { apps } => Response::Started(
            apps.iter()
                .zip(10..)
                .map(|(app, id)| {
                    ProcessInfo::builder(id, app.name.clone(), ProcStatus::Online).build()
                })
                .collect(),
        ),
        _ => Response::Pong,
    }
}

fn write_two_app_flockfile(dir: &Path) -> String {
    let flockfile = dir.join("Flockfile.toml");
    std::fs::write(
        &flockfile,
        "[[app]]\nname = \"zam\"\nscript = \"./srv\"\n\n[[app]]\nname = \"fresh\"\nscript = \"./new\"\n",
    )
    .unwrap();
    flockfile.to_str().unwrap().to_string()
}

/// fails if a re-run respawns a disabled sheep, stops before the file's new
/// app, or exits non-zero because it passed a sheep by
#[tokio::test]
async fn a_rerun_leaves_a_disabled_sheep_down_and_starts_the_new_app() {
    use shep_client::testing::fake_client_answering;

    let dir = tempfile::tempdir().unwrap();
    let flockfile = write_two_app_flockfile(dir.path());
    let path = shep_client::testing::control_address(dir.path());
    let (client, mut envelopes) =
        fake_client_answering(&path, a_shepherd_holding(vec![a_disabled_zam()])).await;

    let (code, _printed, said) = start_against(&client, &flockfile).await;

    assert_eq!(code, ExitCode::Success, "{said}");
    let mut respawned = Vec::new();
    let mut started = Vec::new();
    while let Ok(envelope) = envelopes.try_recv() {
        match envelope.body {
            Request::Restart { selector } => respawned.push(selector),
            Request::Start { apps } => {
                started.extend(apps.into_iter().map(|app| app.name));
            }
            _ => {}
        }
    }
    assert_eq!(respawned, Vec::new(), "the disabled sheep stays down");
    assert_eq!(started, vec!["fresh".to_string()], "the new app starts");
}

/// fails if the aside stops naming the verb that brings the sheep back
#[tokio::test]
async fn the_aside_names_shep_enable_for_each_sheep_passed_by() {
    use shep_client::testing::fake_client_answering;

    let dir = tempfile::tempdir().unwrap();
    let flockfile = write_two_app_flockfile(dir.path());
    let path = shep_client::testing::control_address(dir.path());
    let (client, _envelopes) =
        fake_client_answering(&path, a_shepherd_holding(vec![a_disabled_zam()])).await;

    let (_code, _printed, said) = start_against(&client, &flockfile).await;

    assert!(
        said.contains("zam is disabled") && said.contains("`shep enable zam`"),
        "the aside names the sheep and its remedy: {said}"
    );
}

/// fails if the pass-by leaks into a named start: `shep start zam` is an
/// operator request, which the shepherd refuses with the remedy
#[tokio::test]
async fn a_start_naming_a_disabled_sheep_still_reaches_the_shepherd() {
    use shep_client::testing::fake_client_answering;

    let dir = tempfile::tempdir().unwrap();
    let path = shep_client::testing::control_address(dir.path());
    let (client, mut envelopes) =
        fake_client_answering(&path, a_shepherd_holding(vec![a_disabled_zam()])).await;

    let (_code, _printed, _said) = start_against(&client, "zam").await;

    assert_eq!(respawns(&mut envelopes), vec![SelectorSpec::Id(0)]);
}

/// fails if a disabled row survives the filter, a live row is dropped, or
/// the aside stops naming `shep enable` for each sheep
#[test]
fn leaving_disabled_rows_out_keeps_the_rest_and_names_each_remedy() {
    use crate::commands::lifecycle::respawn::leave_disabled;

    let rows = vec![
        a_disabled_zam(),
        ProcessInfo::builder(1, "api", ProcStatus::Stopped).build(),
        ProcessInfo::builder(2, "web", ProcStatus::Stopped)
            .disabled(Some(true))
            .build(),
    ];
    let mut out = Vec::new();
    let mut err = Vec::new();
    let kept = {
        let mut streams = Streams {
            out: &mut out,
            err: &mut err,
            style: crate::style::Presentation::BARE,
            fmt: Format::Table,
        };
        leave_disabled(&mut streams, rows)
    };

    let kept_ids: Vec<u32> = kept.iter().map(|info| info.id).collect();
    assert_eq!(kept_ids, vec![1]);
    let said = String::from_utf8(err).unwrap();
    assert!(
        said.contains("`shep enable zam`") && said.contains("`shep enable web`"),
        "{said}"
    );
    assert!(
        !said.contains("api"),
        "a sheep left running is not mentioned: {said}"
    );
}
