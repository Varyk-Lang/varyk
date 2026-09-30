//! `env::parse` (spec 2.5): fills a struct from variables named by its
//! fields, through a small serde `Deserializer`, so a `#[rename]` key reads
//! exactly the variable it names. Nothing is written to the process
//! environment.

use crate::{Error, dotenv};
use serde::de::{self, DeserializeOwned, Deserializer, IntoDeserializer, MapAccess, Visitor};
use std::fmt;

/// Reads a `T` from the process environment, then the `.env` in the
/// current directory.
pub fn parse<T: DeserializeOwned>() -> Result<T, Error> {
    let entries = match dotenv::cached() {
        Ok(entries) => entries.as_slice(),
        Err(e) => return Err(e.clone()),
    };
    parse_with(&process_variable, entries)
}

/// A variable of the process environment; one that is set but is not
/// valid text is an error, not a variable that is missing.
fn process_variable(key: &str) -> Result<Option<String>, String> {
    match std::env::var(key) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("`{key}` is not valid text")),
    }
}

/// A lookup in the process environment: the value, `None` when unset,
/// or the error to report.
type Lookup<'a> = &'a dyn Fn(&str) -> Result<Option<String>, String>;

/// `parse` with the process environment injected, for unit tests.
pub(crate) fn parse_with<T: DeserializeOwned>(
    lookup: Lookup<'_>,
    entries: &[(String, String)],
) -> Result<T, Error> {
    T::deserialize(StructReader { lookup, entries }).map_err(|e| Error::new(e.0))
}

#[derive(Debug)]
struct DeError(String);

impl fmt::Display for DeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DeError {}

impl de::Error for DeError {
    fn custom<T: fmt::Display>(msg: T) -> DeError {
        DeError(msg.to_string())
    }

    // A field with no variable and no serde default: name the variable.
    // (An `Option` field never gets here; serde makes it `None`.)
    fn missing_field(field: &'static str) -> DeError {
        DeError(format!("`{}` is not set", field.to_uppercase()))
    }
}

struct StructReader<'a> {
    lookup: Lookup<'a>,
    entries: &'a [(String, String)],
}

impl<'a> StructReader<'a> {
    fn find(&self, variable: &str) -> Result<Option<String>, DeError> {
        if let Some(value) = (self.lookup)(variable).map_err(DeError)? {
            return Ok(Some(value));
        }
        Ok(self
            .entries
            .iter()
            .rev()
            .find(|(key, _)| key == variable)
            .map(|(_, value)| value.clone()))
    }
}

impl<'de, 'a> Deserializer<'de> for StructReader<'a> {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, DeError> {
        Err(DeError("env::parse reads a struct".to_string()))
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        visitor.visit_map(Fields {
            reader: self,
            fields,
            index: 0,
            pending: None,
        })
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf option unit unit_struct newtype_struct seq tuple
        tuple_struct map enum identifier ignored_any
    }
}

struct Fields<'a> {
    reader: StructReader<'a>,
    fields: &'static [&'static str],
    index: usize,
    pending: Option<Value>,
}

impl<'de, 'a> MapAccess<'de> for Fields<'a> {
    type Error = DeError;

    // Only fields that have a value are offered; serde handles the rest
    // (`None` for an `Option`, its default, or `missing_field`).
    fn next_key_seed<K: de::DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        while let Some(field) = self.fields.get(self.index) {
            self.index += 1;
            let variable = field.to_uppercase();
            if let Some(text) = self.reader.find(&variable)? {
                self.pending = Some(Value { variable, text });
                return seed.deserialize((*field).into_deserializer()).map(Some);
            }
        }
        Ok(None)
    }

    fn next_value_seed<T: de::DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<T::Value, DeError> {
        match self.pending.take() {
            Some(value) => seed.deserialize(value),
            None => Err(DeError("env::parse has no value to read".to_string())),
        }
    }
}

/// One variable's text, read as the type the field asks for.
struct Value {
    variable: String,
    text: String,
}

macro_rules! number {
    ($($method:ident $visit:ident $t:ty),*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
            match self.text.parse::<$t>() {
                Ok(n) => visitor.$visit(n),
                Err(_) => Err(self.number_error()),
            }
        }
    )*};
}

impl Value {
    fn number_error(&self) -> DeError {
        let how = if crate::parse::is_integer_out_of_range(&self.text) {
            "is out of range"
        } else {
            "is not a number"
        };
        DeError(format!("`{}` {how}: `{}`", self.variable, self.text))
    }

    fn float_error(&self) -> DeError {
        let how = if crate::parse::float_out_of_range(&self.text) {
            "is out of range"
        } else {
            "is not a number"
        };
        DeError(format!("`{}` {how}: `{}`", self.variable, self.text))
    }
}

impl<'de> Deserializer<'de> for Value {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_string(self.text)
    }

    number! {
        deserialize_i8 visit_i8 i8, deserialize_i16 visit_i16 i16,
        deserialize_i32 visit_i32 i32, deserialize_i64 visit_i64 i64,
        deserialize_u8 visit_u8 u8, deserialize_u16 visit_u16 u16,
        deserialize_u32 visit_u32 u32, deserialize_u64 visit_u64 u64
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.text.parse::<f32>() {
            Ok(n) if crate::parse::float_fits(&self.text, n.is_finite(), n == 0.0) => {
                visitor.visit_f32(n)
            }
            _ => Err(self.float_error()),
        }
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.text.parse::<f64>() {
            Ok(n) if crate::parse::float_fits(&self.text, n.is_finite(), n == 0.0) => {
                visitor.visit_f64(n)
            }
            _ => Err(self.float_error()),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.text.as_str() {
            "true" => visitor.visit_bool(true),
            "false" => visitor.visit_bool(false),
            _ => Err(DeError(format!(
                "`{}` is not `true` or `false`: `{}`",
                self.variable, self.text
            ))),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_some(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        let text = self.text.clone();
        let reader: de::value::StringDeserializer<DeError> = text.into_deserializer();
        reader
            .deserialize_enum(name, variants, visitor)
            .map_err(|_| {
                DeError(format!(
                    "`{}` is not a known value: `{}`",
                    self.variable, self.text
                ))
            })
    }

    serde::forward_to_deserialize_any! {
        char str string bytes byte_buf unit unit_struct newtype_struct seq
        tuple tuple_struct map struct identifier ignored_any i128 u128
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::HashMap;

    #[derive(Debug, PartialEq, Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Mode {
        Dev,
        Prod,
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct Config {
        port: u16,
        debug: bool,
        name: String,
        mode: Mode,
        ratio: f64,
        token: Option<String>,
        retries: Option<u8>,
    }

    fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Result<Option<String>, String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| Ok(map.get(key).cloned())
    }

    fn entries(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    const FULL: &[(&str, &str)] = &[
        ("PORT", "8080"),
        ("DEBUG", "true"),
        ("NAME", "svc"),
        ("MODE", "prod"),
        ("RATIO", "0.5"),
    ];

    fn read<T: serde::de::DeserializeOwned>(
        env: &[(&str, &str)],
        dotenv: &[(&str, &str)],
    ) -> Result<T, Error> {
        parse_with(&lookup_from(env), &entries(dotenv))
    }

    fn message<T: serde::de::DeserializeOwned + std::fmt::Debug>(
        env: &[(&str, &str)],
        dotenv: &[(&str, &str)],
    ) -> String {
        match read::<T>(env, dotenv) {
            Ok(v) => format!("ok {v:?}"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn reads_every_kind_of_field() {
        assert_eq!(
            read::<Config>(FULL, &[]),
            Ok(Config {
                port: 8080,
                debug: true,
                name: "svc".into(),
                mode: Mode::Prod,
                ratio: 0.5,
                token: None,
                retries: None,
            })
        );
    }

    #[test]
    fn present_options_are_some() {
        let mut env = FULL.to_vec();
        env.push(("TOKEN", "t"));
        env.push(("RETRIES", "3"));
        let got = read::<Config>(&env, &[]);
        assert_eq!(
            got.as_ref().map(|c| c.token.clone()),
            Ok(Some("t".to_string()))
        );
        assert_eq!(got.map(|c| c.retries), Ok(Some(3)));
    }

    #[test]
    fn the_process_environment_wins_over_dotenv() {
        let mut env = FULL.to_vec();
        env[0] = ("PORT", "1");
        let got = read::<Config>(&env, &[("PORT", "2")]);
        assert_eq!(got.map(|c| c.port), Ok(1));
    }

    #[test]
    fn dotenv_fills_what_the_environment_lacks() {
        let got = read::<Config>(&[], FULL);
        assert_eq!(got.map(|c| c.port), Ok(8080));
    }

    #[test]
    fn a_missing_variable_is_named() {
        let env: Vec<_> = FULL.iter().filter(|(k, _)| *k != "PORT").cloned().collect();
        assert_eq!(message::<Config>(&env, &[]), "`PORT` is not set");
    }

    #[test]
    fn a_bad_number_names_variable_and_value() {
        let mut env = FULL.to_vec();
        env[0] = ("PORT", "12abc");
        assert_eq!(
            message::<Config>(&env, &[]),
            "`PORT` is not a number: `12abc`"
        );
        env[0] = ("PORT", "70000");
        assert_eq!(
            message::<Config>(&env, &[]),
            "`PORT` is out of range: `70000`"
        );
        env[0] = ("PORT", "8080");
        env[4] = ("RATIO", "half");
        assert_eq!(
            message::<Config>(&env, &[]),
            "`RATIO` is not a number: `half`"
        );
    }

    #[test]
    fn a_bad_bool_names_variable_and_value() {
        let mut env = FULL.to_vec();
        env[1] = ("DEBUG", "yes");
        assert_eq!(
            message::<Config>(&env, &[]),
            "`DEBUG` is not `true` or `false`: `yes`"
        );
    }

    #[test]
    fn a_bad_enum_value_names_variable_and_value() {
        let mut env = FULL.to_vec();
        env[3] = ("MODE", "staging");
        assert_eq!(
            message::<Config>(&env, &[]),
            "`MODE` is not a known value: `staging`"
        );
    }

    #[test]
    fn a_bad_optional_value_is_still_an_error() {
        let mut env = FULL.to_vec();
        env.push(("RETRIES", "many"));
        assert_eq!(
            message::<Config>(&env, &[]),
            "`RETRIES` is not a number: `many`"
        );
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct Renamed {
        #[serde(rename = "listen_port")]
        port: u16,
    }

    #[test]
    fn a_renamed_field_reads_its_renamed_key() {
        assert_eq!(
            read::<Renamed>(&[("LISTEN_PORT", "9")], &[]),
            Ok(Renamed { port: 9 })
        );
        assert_eq!(
            message::<Renamed>(&[("PORT", "9")], &[]),
            "`LISTEN_PORT` is not set"
        );
    }

    fn seven() -> u16 {
        7
    }

    #[derive(Debug, PartialEq, Deserialize)]
    struct WithDefault {
        #[serde(default = "seven")]
        port: u16,
    }

    #[test]
    fn a_serde_default_applies_when_unset() {
        assert_eq!(read::<WithDefault>(&[], &[]), Ok(WithDefault { port: 7 }));
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Nested {
        inner: Renamed,
    }

    #[test]
    fn a_nested_struct_field_is_an_error_not_a_panic() {
        assert!(read::<Nested>(&[("INNER", "x")], &[]).is_err());
    }

    #[test]
    fn a_non_struct_target_is_an_error() {
        assert!(read::<u8>(&[], &[]).is_err());
    }

    #[test]
    fn strings_are_taken_as_written() {
        #[derive(Debug, PartialEq, Deserialize)]
        struct S {
            greeting: String,
        }
        assert_eq!(
            read::<S>(&[("GREETING", " hi there ")], &[]),
            Ok(S {
                greeting: " hi there ".into()
            })
        );
    }

    #[test]
    fn a_float_that_does_not_fit_is_an_error() {
        #[derive(Debug, PartialEq, Deserialize)]
        struct F {
            small: f32,
        }
        assert_eq!(
            message::<F>(&[("SMALL", "1e40")], &[]),
            "`SMALL` is out of range: `1e40`"
        );
        assert_eq!(
            message::<F>(&[("SMALL", "1e-50")], &[]),
            "`SMALL` is out of range: `1e-50`"
        );
        assert_eq!(
            message::<F>(&[("SMALL", "NaN")], &[]),
            "`SMALL` is not a number: `NaN`"
        );
        assert_eq!(message::<F>(&[("SMALL", "0")], &[]), "ok F { small: 0.0 }");
    }

    #[test]
    fn a_process_variable_that_is_not_text_is_an_error_not_missing() {
        #[derive(Debug, PartialEq, Deserialize)]
        struct S {
            name: Option<String>,
        }
        let lookup = |key: &str| Err(format!("`{key}` is not valid text"));
        let got = parse_with::<S>(&lookup, &entries(&[("NAME", "from-dotenv")]));
        assert_eq!(
            got.map_err(|e| e.to_string()),
            Err("`NAME` is not valid text".to_string())
        );
    }
}
