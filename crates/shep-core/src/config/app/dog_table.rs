//! [`DogTable`]: one dog's opaque per-sheep settings table.

use core::fmt;
use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::config::table::{TOML_PRIVATE_DATETIME_KEY, deserialize_table};

/// One dog's `[app.dogs.<name>]` table on a sheep, held opaque.
///
/// shep stores this and hands it to the dog it names; nothing here reads a
/// key inside it. `Debug` prints only how many keys the table holds, never
/// their values: a table can carry a credential the way a dog's own
/// `dogs.toml` section can.
// wire format: changing this is a breaking change
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DogTable(Map<String, Value>);

impl DogTable {
    /// Borrows the underlying table.
    #[must_use]
    pub fn as_map(&self) -> &Map<String, Value> {
        &self.0
    }

    /// Unwraps the underlying table.
    #[must_use]
    pub fn into_map(self) -> Map<String, Value> {
        self.0
    }
}

impl From<Map<String, Value>> for DogTable {
    fn from(map: Map<String, Value>) -> Self {
        Self(map)
    }
}

impl fmt::Debug for DogTable {
    /// Prints only the key count, never a value: a table can carry a
    /// credential the way a dog's own `dogs.toml` section can.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.0.len();
        write!(f, "DogTable(<{n} key{}>)", if n == 1 { "" } else { "s" })
    }
}

/// Reads the `dogs` table itself, one [`DogTable`] per dog. The
/// `deserialize_with` on [`AppConfig::dogs`](crate::config::AppConfig::dogs).
///
/// The shared reader is what refuses `dogs: null` in YAML, where a null
/// scalar asked for as a map would otherwise become an empty one, and a bare
/// TOML datetime under `[[app]]`, which arrives as a one-key table. Each
/// table inside `dogs` keeps its own rules, below.
pub(super) fn deserialize_dogs<'de, D: Deserializer<'de>>(
    de: D,
) -> Result<BTreeMap<String, DogTable>, D::Error> {
    deserialize_table(de, "dogs")
}

impl<'de> Deserialize<'de> for DogTable {
    /// Reads a table, refusing anything else, and rewrites a TOML datetime,
    /// date or time to its plain string at any depth.
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(de)?;
        let Value::Object(map) = value else {
            return Err(serde::de::Error::custom(
                "a dog's table must be a table, not a bare value",
            ));
        };
        Ok(Self(convert_datetimes(map)))
    }
}

/// Walks `map`, turning a toml_datetime private one-key object into its
/// plain string, at any depth and inside an array.
fn convert_datetimes(map: Map<String, Value>) -> Map<String, Value> {
    map.into_iter().map(|(k, v)| (k, convert(v))).collect()
}

/// One value in [`convert_datetimes`]'s walk.
fn convert(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            if map.len() == 1
                && let Some(Value::String(s)) = map.get(TOML_PRIVATE_DATETIME_KEY)
            {
                Value::String(s.clone())
            } else {
                Value::Object(convert_datetimes(map))
            }
        }
        Value::Array(items) => Value::Array(items.into_iter().map(convert).collect()),
        other => other,
    }
}

/// A table's keys and their shapes are the dog's own business, never
/// shep's: the schema can say no more than "a table".
#[cfg(feature = "schema")]
impl schemars::JsonSchema for DogTable {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DogTable".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "additionalProperties": true,
            "description": "A dog's own settings for this sheep. shep stores it and never reads a key inside.",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, Deserialize)]
    struct Holder {
        table: DogTable,
    }

    fn table_of(pairs: impl IntoIterator<Item = (&'static str, Value)>) -> DogTable {
        DogTable::from(
            pairs
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect::<Map<String, Value>>(),
        )
    }

    #[test]
    fn a_bare_toml_time_arrives_as_its_plain_string() {
        let holder: Holder = toml::from_str("[table]\nstart = 09:00:00\n").unwrap();
        assert_eq!(
            holder.table.as_map().get("start"),
            Some(&Value::String("09:00:00".to_string()))
        );
    }

    #[test]
    fn a_bare_toml_date_arrives_as_its_plain_string() {
        let holder: Holder = toml::from_str("[table]\nday = 2026-09-27\n").unwrap();
        assert_eq!(
            holder.table.as_map().get("day"),
            Some(&Value::String("2026-09-27".to_string()))
        );
    }

    #[test]
    fn a_bare_toml_datetime_arrives_as_its_plain_string() {
        let holder: Holder = toml::from_str("[table]\nat = 2026-09-27T09:00:00Z\n").unwrap();
        assert_eq!(
            holder.table.as_map().get("at"),
            Some(&Value::String("2026-09-27T09:00:00Z".to_string()))
        );
    }

    #[test]
    fn a_toml_datetime_nested_in_a_table_still_converts() {
        let toml_src = "[table.hours]\nstart = 09:00:00\nend = 17:00:00\n";
        let holder: Holder = toml::from_str(toml_src).unwrap();
        let hours = holder
            .table
            .as_map()
            .get("hours")
            .and_then(Value::as_object)
            .expect("hours must still be a table");
        assert_eq!(
            hours.get("start"),
            Some(&Value::String("09:00:00".to_string()))
        );
        assert_eq!(
            hours.get("end"),
            Some(&Value::String("17:00:00".to_string()))
        );
    }

    #[test]
    fn a_toml_datetime_inside_an_array_still_converts() {
        let toml_src = "table = { slots = [09:00:00, 17:00:00] }\n";
        let holder: Holder = toml::from_str(toml_src).unwrap();
        let slots = holder
            .table
            .as_map()
            .get("slots")
            .and_then(Value::as_array)
            .expect("slots must still be an array");
        assert_eq!(
            slots.as_slice(),
            [
                Value::String("09:00:00".to_string()),
                Value::String("17:00:00".to_string())
            ]
        );
    }

    /// Only the exact one-key shape converts: a real key of the same name
    /// beside a second key is left alone.
    #[test]
    fn a_real_key_of_the_same_name_beside_another_is_left_alone() {
        let raw = json!({"at": {"$__toml_private_datetime": "09:00:00", "extra": true}});
        let table: DogTable = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(&table).unwrap(), raw);
    }

    #[test]
    fn a_number_is_refused() {
        let err = toml::from_str::<Holder>("table = 5\n").unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn a_string_is_refused() {
        assert!(toml::from_str::<Holder>("table = \"x\"\n").is_err());
    }

    #[test]
    fn an_array_is_refused() {
        assert!(toml::from_str::<Holder>("table = [1]\n").is_err());
    }

    /// fails if a wire `dogs` that is not a table stops being refused.
    ///
    /// `AppConfig` decodes from a peer as well as from a Flockfile, and both
    /// paths read the key through the shared table reader. The wire refused a
    /// null map before, in serde's words: what changed is the words, not what
    /// is refused. A real table still decodes.
    #[test]
    fn a_wire_dogs_that_is_not_a_table_is_still_refused() {
        let src = r#"{"name":"web","script":"./srv","dogs":null}"#;
        let err = serde_json::from_str::<crate::config::AppConfig>(src)
            .expect_err("a wire dogs that is not a table must be refused, as it always was");
        let msg = err.to_string();
        assert!(msg.contains("`dogs` must be a table"), "{msg}");

        let ok: crate::config::AppConfig = serde_json::from_str(
            r#"{"name":"web","script":"./srv","dogs":{"jobs":{"concurrency":2}}}"#,
        )
        .expect("a real dogs table on the wire still decodes");
        assert_eq!(ok.dogs.len(), 1);
    }

    #[test]
    fn debug_prints_the_key_count_and_never_a_value() {
        // Exact string pinned so a lazy derive(Debug) refactor fails here:
        // a derived Debug would print the credential this table might hold.
        let table = table_of([("token", json!("hunter2"))]);
        assert_eq!(format!("{table:?}"), "DogTable(<1 key>)");

        let table = table_of([
            ("concurrency", json!(2)),
            ("merge", json!("ask")),
            ("token", json!("hunter2")),
        ]);
        assert_eq!(format!("{table:?}"), "DogTable(<3 keys>)");
        assert!(!format!("{table:?}").contains("hunter2"));
    }

    #[test]
    fn a_json_round_trip_is_byte_identical() {
        let table = table_of([("concurrency", json!(2)), ("merge", json!("ask"))]);
        let json = serde_json::to_string(&table).unwrap();
        let holder: Holder = serde_json::from_str(&format!("{{\"table\":{json}}}")).unwrap();
        assert_eq!(serde_json::to_string(&holder.table).unwrap(), json);
    }
}
