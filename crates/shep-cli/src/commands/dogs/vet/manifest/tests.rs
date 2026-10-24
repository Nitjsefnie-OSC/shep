use std::time::Duration;

use shep_core::dogs::manifest::Manifest;

use super::super::{DogSchema, VettedBinary, vet_binary_within};

/// Generous: the question is the answer, not the clock.
const BUDGET: Duration = Duration::from_secs(10);

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
