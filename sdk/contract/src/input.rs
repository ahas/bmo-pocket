//! Input shapes more than one tool asks for.

use std::borrow::Cow;
use std::fmt;
use std::ops::Deref;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Deserialize;

/// A string with something in it. An empty one is refused as malformed before the tool runs, and
/// the schema says so.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct NonEmpty(String);

impl TryFrom<String> for NonEmpty {
  type Error = &'static str;

  fn try_from(value: String) -> Result<Self, Self::Error> {
    if value.is_empty() { Err("must not be empty") } else { Ok(Self(value)) }
  }
}

impl Deref for NonEmpty {
  type Target = str;

  fn deref(&self) -> &str {
    &self.0
  }
}

impl fmt::Display for NonEmpty {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(&self.0)
  }
}

impl JsonSchema for NonEmpty {
  fn inline_schema() -> bool {
    true
  }

  fn schema_name() -> Cow<'static, str> {
    "NonEmpty".into()
  }

  fn json_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "string", "minLength": 1 })
  }
}

/// A whole number above nought.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "serde_json::Number")]
pub struct Positive(pub i64);

impl TryFrom<serde_json::Number> for Positive {
  type Error = &'static str;

  fn try_from(value: serde_json::Number) -> Result<Self, Self::Error> {
    value
      .as_i64()
      .filter(|whole| *whole > 0)
      .map(Self)
      .ok_or("must be a whole number above nought")
  }
}

impl JsonSchema for Positive {
  fn inline_schema() -> bool {
    true
  }

  fn schema_name() -> Cow<'static, str> {
    "Positive".into()
  }

  fn json_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "integer", "exclusiveMinimum": 0 })
  }
}

/// A calendar day written as `YYYY-MM-DD`, as a date argument's pattern asks for one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct IsoDay(String);

impl TryFrom<String> for IsoDay {
  type Error = &'static str;

  fn try_from(value: String) -> Result<Self, Self::Error> {
    let shaped = value.len() == 10
      && value
        .bytes()
        .enumerate()
        .all(|(at, byte)| if at == 4 || at == 7 { byte == b'-' } else { byte.is_ascii_digit() });

    if shaped { Ok(Self(value)) } else { Err("a date as YYYY-MM-DD") }
  }
}

impl Deref for IsoDay {
  type Target = str;

  fn deref(&self) -> &str {
    &self.0
  }
}

impl JsonSchema for IsoDay {
  fn inline_schema() -> bool {
    true
  }

  fn schema_name() -> Cow<'static, str> {
    "IsoDay".into()
  }

  fn json_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "string", "pattern": "^\\d{4}-\\d{2}-\\d{2}$" })
  }
}

/// An id written in decimal digits alone, as a numeric id passed as a string asks for one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Digits(String);

impl TryFrom<String> for Digits {
  type Error = &'static str;

  fn try_from(value: String) -> Result<Self, Self::Error> {
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
      Ok(Self(value))
    } else {
      Err("decimal digits alone")
    }
  }
}

impl Deref for Digits {
  type Target = str;

  fn deref(&self) -> &str {
    &self.0
  }
}

impl JsonSchema for Digits {
  fn inline_schema() -> bool {
    true
  }

  fn schema_name() -> Cow<'static, str> {
    "Digits".into()
  }

  fn json_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "string", "pattern": "^\\d+$" })
  }
}

/// A web address.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct WebAddress(pub url::Url);

impl TryFrom<String> for WebAddress {
  type Error = String;

  fn try_from(value: String) -> Result<Self, Self::Error> {
    url::Url::parse(&value)
      .map(Self)
      .map_err(|why| format!("{value} is not an address: {why}"))
  }
}

impl JsonSchema for WebAddress {
  fn inline_schema() -> bool {
    true
  }

  fn schema_name() -> Cow<'static, str> {
    "WebAddress".into()
  }

  fn json_schema(_: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "string", "format": "uri" })
  }
}

/// A list with at least one thing in it, as `.min(1)` asks of an array.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(try_from = "Vec<T>", bound(deserialize = "T: Deserialize<'de>"))]
pub struct AtLeastOne<T>(pub Vec<T>);

impl<T> TryFrom<Vec<T>> for AtLeastOne<T> {
  type Error = &'static str;

  fn try_from(value: Vec<T>) -> Result<Self, Self::Error> {
    if value.is_empty() { Err("must hold at least one") } else { Ok(Self(value)) }
  }
}

impl<T: JsonSchema> JsonSchema for AtLeastOne<T> {
  fn inline_schema() -> bool {
    true
  }

  fn schema_name() -> Cow<'static, str> {
    format!("AtLeastOne_{}", T::schema_name()).into()
  }

  fn json_schema(generator: &mut SchemaGenerator) -> Schema {
    json_schema!({ "type": "array", "items": generator.subschema_for::<T>(), "minItems": 1 })
  }
}
