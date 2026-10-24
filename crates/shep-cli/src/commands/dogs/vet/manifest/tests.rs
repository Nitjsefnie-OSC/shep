use std::time::Duration;

use shep_core::dogs::manifest::Manifest;

use super::super::{DogSchema, VettedBinary, vet_binary_within};
use super::{ManifestRead, read_manifest};

/// Generous: the question is the answer, not the clock.
const BUDGET: Duration = Duration::from_secs(10);

/// Shorter than the fixture dog sleeps, for the case where the budget is the
/// answer: a probe still running when it runs out.
const BUDGET_SHORT: Duration = Duration::from_millis(300);

fn write_dog(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let binary = dir.join("shep-otel");
    std::fs::write(&binary, format!("#!/bin/sh\n{body}\nexit 0\n")).unwrap();
    let mut mode = std::fs::metadata(&binary).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    std::fs::set_permissions(&binary, mode).unwrap();
    binary
}

#[test]
fn a_manifests_settings_are_the_schema_and_the_manifest_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let body = r#"if [ "$1" = --shep-manifest ]; then
  echo '{"manifest":1,"name":"otel","version":"0.2.0","protocol":12,"settings":{"type":"object"}}'
fi"#;
    let binary = write_dog(dir.path(), body);

    let vetted = vet_binary_within(&binary, dir.path(), "otel", BUDGET).unwrap();

    assert_eq!(
        vetted.schema,
        DogSchema::Published(serde_json::json!({"type": "object"}))
    );
    assert_eq!(
        vetted.manifest.map(|manifest| manifest.name),
        Some("otel".to_string())
    );
}

/// Exact string, not a "contains no secret" probe: the manifest's `defaults`
/// are TOML text for a section that may hold a credential.
#[test]
fn vetted_binary_debug_never_prints_the_manifest() {
    let mut manifest = Manifest::new("otel", "0.2.0", 12);
    manifest.defaults = Some("token = \"hunter2\"\n".to_string());
    let vetted = VettedBinary {
        path: "/opt/shep-otel".into(),
        group_writable: Vec::new(),
        answer: None,
        schema: DogSchema::Silent,
        manifest: Some(manifest),
    };

    assert_eq!(
        format!("{vetted:?}"),
        "VettedBinary { path: \"/opt/shep-otel\", group_writable: [], answer: None, \
         schema: Silent, manifest: Some(\"..\") }"
    );
}

const MANIFEST_BODY: &str = r#"if [ "$1" = --shep-manifest ]; then
  echo '{"manifest":1,"name":"otel","version":"0.2.0","protocol":12,"defaults":"token = 1"}'
fi"#;

fn cached_files(home: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(home.join("manifests"))
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default()
}

/// The `defaults` of a manifest can hold a credential, so the cache is
/// readable by its owner alone.
#[test]
fn the_cache_is_owner_only() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let binary = write_dog(dir.path(), MANIFEST_BODY);

    read_manifest(&binary, dir.path(), "otel", BUDGET).unwrap();

    let files = cached_files(dir.path());
    assert_eq!(files.len(), 1, "{files:?}");
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&files[0]), 0o600);
    assert_eq!(mode(&dir.path().join("manifests")), 0o700);
}

/// A binary replaced while it answers may have answered as the new build, so
/// its answer is not filed under the digest of the old one.
#[test]
fn a_binary_that_changes_while_probed_is_not_cached() {
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        "{MANIFEST_BODY}\nif [ \"$1\" = --shep-manifest ]; then\n  \
         printf '#!/bin/sh\\n# rebuilt\\n' > \"$0.new\" && mv \"$0.new\" \"$0\"\nfi"
    );
    let binary = write_dog(dir.path(), &body);

    let read = read_manifest(&binary, dir.path(), "otel", BUDGET).unwrap();

    assert!(
        matches!(read, ManifestRead::Read(_)),
        "the answer is still used"
    );
    assert_eq!(cached_files(dir.path()), Vec::<std::path::PathBuf>::new());
}

/// A dog that printed nothing and exited 0 declared nothing. A dog whose probe
/// timed out, failed, or printed something that is not a manifest said nothing
/// readable, and a caller that has to know whether the dog asked to be consulted
/// cannot read those two alike.
#[test]
fn a_probe_that_answers_nothing_is_read_as_no_manifest_and_a_probe_that_fails_is_not() {
    let dir = tempfile::tempdir().unwrap();

    let silent = write_dog(dir.path(), "true");
    assert!(matches!(
        read_manifest(&silent, dir.path(), "otel", BUDGET).unwrap(),
        ManifestRead::Absent
    ));

    let broken = write_dog(
        dir.path(),
        r#"if [ "$1" = --shep-manifest ]; then
  echo 'not a manifest'
fi"#,
    );
    assert!(matches!(
        read_manifest(&broken, dir.path(), "otel", BUDGET).unwrap(),
        ManifestRead::Unreadable
    ));

    let failed = dir.path().join("shep-otel-failed");
    std::fs::write(
        &failed,
        "#!/bin/sh\nif [ \"$1\" = --shep-manifest ]; then\n  exit 1\nfi\n",
    )
    .unwrap();
    let mut mode = std::fs::metadata(&failed).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    std::fs::set_permissions(&failed, mode).unwrap();
    assert!(matches!(
        read_manifest(&failed, dir.path(), "otel", BUDGET).unwrap(),
        ManifestRead::Unreadable
    ));

    let slow = write_dog(
        dir.path(),
        r#"if [ "$1" = --shep-manifest ]; then
  sleep 5
fi"#,
    );
    assert!(
        matches!(
            read_manifest(&slow, dir.path(), "otel", BUDGET_SHORT).unwrap(),
            ManifestRead::Unreadable,
        ),
        "a probe that outlasted its budget answered nothing"
    );
}
