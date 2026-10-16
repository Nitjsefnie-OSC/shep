//! The one refusal shared by every Flockfile key that has to be a table.
//!
//! `dog`, `env` and `dogs` each read a table, and each refuses anything else
//! in the operator's words rather than serde's: the key named, the shape it
//! needs named, and nothing about what the document actually wrote.
//!
//! Two things make one visitor worth three local ones.
//!
//! - `deserialize_any`, not `deserialize_map`. At least one YAML backend
//!   folds a null scalar asked for as a map into an empty map before
//!   `deserialize_map` ever reaches a visitor, so `env:` with nothing after
//!   it parses as "no variables". `deserialize_any` reaches that null as
//!   `visit_unit`, which is the arm a refusal can be made to say something
//!   from.
//! - serde's default type errors print the offending value: `visit_str`
//!   reports `invalid type: string "…"` with the string in it. An `env`
//!   value is a secret and an error message reaches a log, so every arm that
//!   could echo one is overridden to name only the key. TOML and YAML echo
//!   the offending source line whatever serde says, which is the backend's
//!   doing and not this visitor's; that is why the test that claims a
//!   refusal carries no value runs through JSON and JSON5.
//!
//! A bare TOML date, time or datetime is the one value that arrives wearing
//! a table's clothes, as a one-key table under the name
//! [`TOML_PRIVATE_DATETIME_KEY`]. Without the branch in `visit_map` it lands
//! in the parsed config as though an operator had written that key.

use std::collections::BTreeMap;
use std::marker::PhantomData;

use serde::de::{Deserialize, Deserializer, EnumAccess, MapAccess, SeqAccess, Visitor};

/// toml_datetime's private field name for a bare TOML date, time, or
/// datetime: the shape `toml` 0.8 hands a generic `Deserialize` before the
/// value ever reaches JSON, measured against toml 0.8.23.
pub(super) const TOML_PRIVATE_DATETIME_KEY: &str = "$__toml_private_datetime";

/// Reads the table `key` must be, refusing anything else with
/// [`refusal`].
///
/// `V` is what sits *inside* the table and keeps its own rules: an `env`
/// value still refuses a float, a [`DogTable`](crate::config::DogTable)
/// still refuses a bare value one level down. This function only decides
/// whether the key itself holds a table.
pub(super) fn deserialize_table<'de, D, V>(
    deserializer: D,
    key: &'static str,
) -> Result<BTreeMap<String, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct TableVisitor<'key, V> {
        key: &'key str,
        value: PhantomData<V>,
    }

    impl<'de, V: Deserialize<'de>> Visitor<'de> for TableVisitor<'_, V> {
        type Value = BTreeMap<String, V>;

        fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            write!(formatter, "a table for `{}`", self.key)
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some(name) = map.next_key::<String>()? {
                // The marker as a table's first key is a bare TOML date,
                // time or datetime wearing a table's clothes, not a key an
                // operator wrote. Refused before its value is read: the
                // value is a plain string, and a `V` that itself wants a
                // table (a `DogTable` under `dogs`) would refuse the string
                // first and put the wrong words in front of the operator.
                if out.is_empty() && name == TOML_PRIVATE_DATETIME_KEY {
                    return Err(refusal(self.key));
                }
                out.insert(name, map.next_value::<V>()?);
            }
            Ok(out)
        }

        // Every arm below that serde would otherwise answer with
        // `invalid_type`, which prints what it got. See the module comment.
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_some<D: Deserializer<'de>>(
            self,
            _deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            Err(refusal(self.key))
        }

        fn visit_bool<E: serde::de::Error>(self, _v: bool) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_i8<E: serde::de::Error>(self, _v: i8) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_i16<E: serde::de::Error>(self, _v: i16) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_i32<E: serde::de::Error>(self, _v: i32) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_i64<E: serde::de::Error>(self, _v: i64) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_i128<E: serde::de::Error>(self, _v: i128) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_u8<E: serde::de::Error>(self, _v: u8) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_u16<E: serde::de::Error>(self, _v: u16) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_u32<E: serde::de::Error>(self, _v: u32) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_u64<E: serde::de::Error>(self, _v: u64) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_u128<E: serde::de::Error>(self, _v: u128) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_f32<E: serde::de::Error>(self, _v: f32) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_f64<E: serde::de::Error>(self, _v: f64) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_char<E: serde::de::Error>(self, _v: char) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_str<E: serde::de::Error>(self, _v: &str) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_borrowed_str<E: serde::de::Error>(self, _v: &'de str) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_string<E: serde::de::Error>(self, _v: String) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_bytes<E: serde::de::Error>(self, _v: &[u8]) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_borrowed_bytes<E: serde::de::Error>(
            self,
            _v: &'de [u8],
        ) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_byte_buf<E: serde::de::Error>(self, _v: Vec<u8>) -> Result<Self::Value, E> {
            Err(refusal(self.key))
        }

        fn visit_seq<A: SeqAccess<'de>>(self, _seq: A) -> Result<Self::Value, A::Error> {
            Err(refusal(self.key))
        }

        fn visit_enum<A: EnumAccess<'de>>(self, _access: A) -> Result<Self::Value, A::Error> {
            Err(refusal(self.key))
        }
    }

    deserializer.deserialize_any(TableVisitor {
        key,
        value: PhantomData,
    })
}

/// The refusal: the key, the shape it needs, and nothing about the value it
/// was given.
fn refusal<E: serde::de::Error>(key: &str) -> E {
    E::custom(format!("`{key}` must be a table"))
}
