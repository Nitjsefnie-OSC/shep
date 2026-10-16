//! Resolving the `{{secret:...}}` references in a dog's `dogs.toml` section.
//!
//! A dog that asks `Request::DogConfigResolved` gets its section with each
//! reference replaced by the value the shepherd holds for it. The text on
//! disk is never touched, and `Request::DogConfig` still serves it as
//! written, which is what an editor must read.

use core::fmt;
use std::collections::BTreeSet;

use shep_core::config::template::render_secrets;
use shep_core::protocol::DogSectionToml;
use shep_core::secrets::{SecretError, SecretView, references_in};
use toml_edit::{DocumentMut, Formatted, Item, Value};

use super::spec::DogError;

/// Why the secret store could not be read, without the store's own message.
///
/// A [`SecretError::Decode`] message can quote the JSON value that failed to
/// parse, which in `secrets.json` is a secret. This keeps the class of
/// failure and drops the text (IR-41).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreFault {
    /// The file could not be opened or read
    Io,
    /// The file is not the JSON the store writes
    Decode,
    /// The file is a version newer than this build understands
    FutureVersion,
    /// Any other [`SecretError`]
    Other,
}

impl From<&SecretError> for StoreFault {
    fn from(err: &SecretError) -> Self {
        match err {
            SecretError::Io(_) => Self::Io,
            SecretError::Decode(_) => Self::Decode,
            SecretError::FutureVersion(_) => Self::FutureVersion,
            _ => Self::Other,
        }
    }
}

impl fmt::Display for StoreFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Io => "I/O failed",
            Self::Decode => "its contents do not parse",
            Self::FutureVersion => "it is a newer version than this build understands",
            Self::Other => "unusable",
        })
    }
}

/// Every `{{secret:...}}` reference `raw`, a dog's section, names, as
/// `KEY` or `namespace/KEY`.
///
/// Walks the same strings [`resolve_section`] substitutes into: keys and
/// comments are not among them. A section that does not parse names
/// nothing.
#[must_use]
pub fn section_references(raw: &str) -> BTreeSet<String> {
    let Ok(mut doc) = raw.parse::<DocumentMut>() else {
        return BTreeSet::new();
    };
    let mut found = BTreeSet::new();
    let _ = walk_strings::<core::convert::Infallible>(
        doc.as_table_mut().iter_mut(),
        &mut String::new(),
        &mut |_, text| {
            found.extend(references_in(text));
            Ok(None)
        },
    );
    found
}

/// `raw`, a dog's section, with every `{{secret:...}}` string value replaced
/// by its resolution in `view`.
///
/// The value is replaced, never the text, so `toml_edit` escapes it: a
/// secret holding a quote, a backslash or a newline stays one string and
/// cannot open a table. Keys, comments, key order and every non-string
/// value are untouched, as is a string that holds no reference.
///
/// # Errors
/// - [`DogError::Config`]: `raw` is not TOML.
/// - [`DogError::Reference`]: a reference `view` cannot resolve, or a
///   `{{secret:` token outside the reference grammar. Carries the key path
///   of the first such string and never a value.
pub fn resolve_section(raw: &str, view: &SecretView) -> Result<DogSectionToml, DogError> {
    let mut doc: DocumentMut = raw
        .parse()
        .map_err(|err: toml_edit::TomlError| DogError::Config(err.to_string()))?;
    walk_strings(
        doc.as_table_mut().iter_mut(),
        &mut String::new(),
        &mut |key_path, text| {
            let rendered = render_secrets(text, view).map_err(|source| DogError::Reference {
                key_path: key_path.to_string(),
                source,
            })?;
            // Only a string that changed is replaced, so every other string
            // keeps the quoting the operator chose.
            Ok((rendered != text).then_some(rendered))
        },
    )?;
    Ok(doc.to_string().into())
}

/// The error for a section that names references when the store holding
/// them could not be read.
///
/// Names the key path of the first reference, so an operator knows which
/// line asked. The caller has already established that `raw` names one.
#[must_use]
pub fn store_unreadable(raw: &str, source: &SecretError) -> DogError {
    let key_path = raw
        .parse::<DocumentMut>()
        .ok()
        .and_then(|mut doc| {
            let mut first = None;
            let _ = walk_strings(
                doc.as_table_mut().iter_mut(),
                &mut String::new(),
                &mut |key_path, text| {
                    if first.is_none() && !references_in(text).is_empty() {
                        first = Some(key_path.to_string());
                    }
                    Ok::<_, core::convert::Infallible>(None)
                },
            );
            first
        })
        .unwrap_or_default();
    DogError::SecretStore {
        key_path,
        fault: source.into(),
    }
}

/// What a visitor does with one string value: the key path it sits at and
/// its text in, the replacement out, or `None` to leave it.
type Visit<'a, E> = dyn FnMut(&str, &str) -> Result<Option<String>, E> + 'a;

/// Calls `visit` on every string value under `entries`: tables, sub-tables,
/// inline tables, arrays and arrays of tables. Keys are never visited.
///
/// `path` is the key path of the entries' parent, restored on return.
fn walk_strings<'a, E>(
    entries: impl Iterator<Item = (toml_edit::KeyMut<'a>, &'a mut Item)>,
    path: &mut String,
    visit: &mut Visit<'_, E>,
) -> Result<(), E> {
    for (key, item) in entries {
        let mark = path.len();
        push_key(path, key.get());
        let walked = walk_item(item, path, visit);
        path.truncate(mark);
        walked?;
    }
    Ok(())
}

fn walk_item<E>(item: &mut Item, path: &mut String, visit: &mut Visit<'_, E>) -> Result<(), E> {
    match item {
        Item::Value(value) => walk_value(value, path, visit),
        Item::Table(table) => walk_strings(table.iter_mut(), path, visit),
        Item::ArrayOfTables(tables) => {
            for (index, table) in tables.iter_mut().enumerate() {
                let mark = path.len();
                push_index(path, index);
                let walked = walk_strings(table.iter_mut(), path, visit);
                path.truncate(mark);
                walked?;
            }
            Ok(())
        }
        Item::None => Ok(()),
    }
}

fn walk_value<E>(value: &mut Value, path: &mut String, visit: &mut Visit<'_, E>) -> Result<(), E> {
    match value {
        Value::String(formatted) => {
            if let Some(replacement) = visit(path, formatted.value())? {
                // A fresh `Formatted` has no representation, so `toml_edit`
                // picks one that escapes the text; the old decor (a comment
                // after the value) is carried across.
                let decor = formatted.decor().clone();
                let mut replaced = Formatted::new(replacement);
                *replaced.decor_mut() = decor;
                *formatted = replaced;
            }
            Ok(())
        }
        Value::Array(array) => {
            for (index, element) in array.iter_mut().enumerate() {
                let mark = path.len();
                push_index(path, index);
                let walked = walk_value(element, path, visit);
                path.truncate(mark);
                walked?;
            }
            Ok(())
        }
        Value::InlineTable(table) => {
            for (key, inner) in table.iter_mut() {
                let mark = path.len();
                push_key(path, key.get());
                let walked = walk_value(inner, path, visit);
                path.truncate(mark);
                walked?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Appends `.key`, or `key` at the root. A key that is not a bare TOML key
/// is quoted so the path stays unambiguous.
fn push_key(path: &mut String, key: &str) {
    if !path.is_empty() {
        path.push('.');
    }
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        path.push_str(key);
    } else {
        path.push_str(&format!("{key:?}"));
    }
}

fn push_index(path: &mut String, index: usize) {
    path.push_str(&format!("[{index}]"));
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use shep_core::secrets::ProviderCache;

    use super::*;

    /// A value no message may carry.
    const SENTINEL: &str = "sentinel-7f3a9c";

    fn view(environment: &str, pairs: &[(&str, &str)]) -> SecretView {
        let store = pairs
            .iter()
            .map(|(key, value)| {
                (
                    (*key).to_string(),
                    BTreeMap::from([(environment.to_string(), (*value).to_string())]),
                )
            })
            .collect();
        SecretView::new(environment.to_string(), store, ProviderCache::default())
    }

    fn parsed(section: &DogSectionToml) -> toml::Table {
        section.as_str().parse().unwrap()
    }

    #[test]
    fn a_reference_in_a_nested_table_an_array_and_an_array_of_tables_resolves() {
        let raw = "\
token = \"{{secret:T}}\"
port = 9615
kinds = [\"exit\", \"{{secret:T}}\"]

[sinks.local]
url = \"https://x.invalid/{{secret:T}}\"
inline = { deep = \"{{secret:T}}\", n = 1 }

[[rules]]
url = \"plain\"

[[rules]]
url = \"{{secret:T}}\"
";
        let section = resolve_section(raw, &view("production", &[("T", "s3cret")])).unwrap();
        let table = parsed(&section);
        assert_eq!(table["token"].as_str(), Some("s3cret"));
        assert_eq!(table["port"].as_integer(), Some(9615));
        assert_eq!(table["kinds"][1].as_str(), Some("s3cret"));
        assert_eq!(
            table["sinks"]["local"]["url"].as_str(),
            Some("https://x.invalid/s3cret")
        );
        assert_eq!(
            table["sinks"]["local"]["inline"]["deep"].as_str(),
            Some("s3cret")
        );
        assert_eq!(table["rules"][0]["url"].as_str(), Some("plain"));
        assert_eq!(table["rules"][1]["url"].as_str(), Some("s3cret"));
    }

    #[test]
    fn a_value_with_a_quote_a_backslash_a_newline_and_a_table_header_stays_one_string() {
        let hostile = "x\"\n[evil]\nk = \"y";
        let section = resolve_section(
            "token = \"{{secret:T}}\"\n",
            &view("production", &[("T", hostile)]),
        )
        .unwrap();
        let table = parsed(&section);
        assert_eq!(table["token"].as_str(), Some(hostile));
        assert_eq!(table.len(), 1, "no extra table: {}", section.as_str());

        let with_backslash = "C:\\dir\\\"q\"";
        let section = resolve_section(
            "token = \"{{secret:T}}\"\n",
            &view("production", &[("T", with_backslash)]),
        )
        .unwrap();
        assert_eq!(parsed(&section)["token"].as_str(), Some(with_backslash));
    }

    #[test]
    fn comments_and_key_order_survive_resolution() {
        let raw = "\
# above z
z = \"{{secret:T}}\" # beside z
a = 1

# above the table
[b]
y = 'literal'
c = \"{{secret:T}}\"
";
        let section = resolve_section(raw, &view("production", &[("T", "v")])).unwrap();
        assert_eq!(
            section.as_str(),
            "\
# above z
z = \"v\" # beside z
a = 1

# above the table
[b]
y = 'literal'
c = \"v\"
"
        );
    }

    #[test]
    fn a_reference_in_a_key_or_a_comment_is_left_alone() {
        let raw = "\
# {{secret:ABSENT}}
\"{{secret:ABSENT}}\" = 1
[\"{{secret:OTHER}}\"]
k = \"{{{{secret:ABSENT}}}}\"
";
        let empty = SecretView::empty("production".to_string());
        assert!(section_references(raw).is_empty());
        assert_eq!(resolve_section(raw, &empty).unwrap().as_str(), raw);
    }

    #[test]
    fn section_references_agrees_with_what_resolve_substitutes() {
        let raw = "\
a = \"{{secret:ONE}}\"
b = [\"{{secret:vercel/TWO}}\", \"{{name}}\"]
c = \"{{secret:ONE}} {{{{secret:NOPE}}}} {{secret:\"
[t]
d = { e = \"{{secret:THREE}}\" }
";
        let found = section_references(raw);
        assert_eq!(
            found,
            BTreeSet::from([
                "ONE".to_string(),
                "THREE".to_string(),
                "vercel/TWO".to_string()
            ])
        );

        // Resolving against a view holding exactly those references succeeds;
        // dropping any one of them is refused, so none is a reference the
        // resolver ignores.
        let providers = ProviderCache::new(
            BTreeMap::from([(
                "vercel".to_string(),
                BTreeMap::from([(
                    "TWO".to_string(),
                    BTreeMap::from([("production".to_string(), "2".to_string())]),
                )]),
            )]),
            BTreeMap::from([(
                "vercel".to_string(),
                BTreeSet::from(["production".to_string()]),
            )]),
        );
        let store = |keys: &[&str]| {
            keys.iter()
                .map(|key| {
                    (
                        (*key).to_string(),
                        BTreeMap::from([("production".to_string(), "1".to_string())]),
                    )
                })
                .collect()
        };
        let full = SecretView::new(
            "production".to_string(),
            store(&["ONE", "THREE"]),
            providers.clone(),
        );
        let section = resolve_section(raw, &full).unwrap();
        assert!(section_references(section.as_str()).is_empty());
        for missing in [&["THREE"][..], &["ONE"][..]] {
            let partial =
                SecretView::new("production".to_string(), store(missing), providers.clone());
            assert!(resolve_section(raw, &partial).is_err());
        }
    }

    #[test]
    fn an_unresolved_reference_is_refused_naming_its_key_path() {
        let raw = "[sinks.local]\nurl = \"{{secret:ABSENT}}\"\n\n[[rules]]\nurl = \"ok\"\n\n[[rules]]\nurl = \"{{secret:ABSENT}}\"\n";
        let err = resolve_section(raw, &view("production", &[("K", SENTINEL)])).unwrap_err();
        assert_eq!(
            err.to_string(),
            "`sinks.local.url`: `{{secret:ABSENT}}` has no value in the `production` environment"
        );
        let later = resolve_section(
            "[[rules]]\nurl = \"ok\"\n[[rules]]\nurl = \"{{secret:ABSENT}}\"\n",
            &view("production", &[]),
        )
        .unwrap_err();
        assert!(later.to_string().starts_with("`rules[1].url`: "), "{later}");
    }

    /// IR-41: exact strings, with a sentinel in the store that appears in
    /// neither rendering.
    #[test]
    fn the_secret_errors_print_exact_strings_and_never_a_value() {
        let store = view("production", &[("PRESENT", SENTINEL)]);

        let unresolved = resolve_section("k = \"{{secret:ABSENT}}\"\n", &store).unwrap_err();
        assert_eq!(
            unresolved.to_string(),
            "`k`: `{{secret:ABSENT}}` has no value in the `production` environment"
        );
        assert_eq!(
            format!("{unresolved:?}"),
            "Reference { key_path: \"k\", source: Unresolved { reference: \"{{secret:ABSENT}}\", \
             environment: \"production\" } }"
        );

        let unready = resolve_section("k = \"{{secret:paddock/owner}}\"\n", &store).unwrap_err();
        assert_eq!(
            unready.to_string(),
            "`k`: `{{secret:paddock/owner}}` reads the `paddock` namespace, which no provider \
             dog has pushed to for the `production` environment yet"
        );

        let malformed = resolve_section("k = \"{{secret:bad key}}\"\n", &store).unwrap_err();
        assert_eq!(
            malformed.to_string(),
            "`k`: `{{secret:bad key}}` is not a valid secret reference: write \
             `{{secret:KEY}}` or `{{secret:namespace/KEY}}`"
        );

        let decode = serde_json::from_str::<u32>(&format!("\"{SENTINEL}\"")).unwrap_err();
        assert!(decode.to_string().contains(SENTINEL), "the premise");
        let unreadable = store_unreadable(
            "[a]\nk = \"{{secret:PRESENT}}\"\n",
            &SecretError::Decode(decode),
        );
        assert_eq!(
            unreadable.to_string(),
            "`a.k`: the secret store could not be read (its contents do not parse)"
        );
        assert_eq!(
            format!("{unreadable:?}"),
            "SecretStore { key_path: \"a.k\", fault: Decode }"
        );

        for rendered in [
            unresolved.to_string(),
            format!("{unresolved:?}"),
            unready.to_string(),
            malformed.to_string(),
            unreadable.to_string(),
            format!("{unreadable:?}"),
        ] {
            assert!(!rendered.contains(SENTINEL), "{rendered}");
        }
    }

    #[test]
    fn a_section_that_is_not_toml_is_a_config_error() {
        let err =
            resolve_section("k = ", &SecretView::empty("production".to_string())).unwrap_err();
        assert!(matches!(err, DogError::Config(_)), "{err}");
        assert!(section_references("k = ").is_empty());
    }
}
