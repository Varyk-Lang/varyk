//! JSON on serde_json (spec 2.4).

use crate::Error;
use serde::Serialize;
use serde::de::DeserializeOwned;

/// Reads a `T` from JSON text. Unknown keys are ignored; a missing key is
/// an error naming it unless serde has a default for it (`Option` is `None`).
pub fn parse<T: DeserializeOwned>(text: &str) -> Result<T, Error> {
    serde_json::from_str(text).map_err(|e| Error::new(e.to_string()))
}

/// Writes `value` as compact JSON.
pub fn stringify<T: Serialize + ?Sized>(value: &T) -> String {
    match serde_json::to_string(value) {
        Ok(text) => text,
        // Unreachable: the checker admits only convertible types, and none
        // of them can fail to serialize. Return the text of the error
        // rather than panic.
        Err(e) => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct User {
        name: String,
        age: u8,
        email: Option<String>,
    }

    #[test]
    fn a_struct_round_trips() {
        let u = User {
            name: "Ada".into(),
            age: 36,
            email: Some("a@x.io".into()),
        };
        let text = stringify(&u);
        assert_eq!(text, r#"{"name":"Ada","age":36,"email":"a@x.io"}"#);
        assert_eq!(parse::<User>(&text), Ok(u));
    }

    #[test]
    fn none_is_null_and_a_missing_option_key_is_none() {
        let u = User {
            name: "Bo".into(),
            age: 1,
            email: None,
        };
        assert_eq!(stringify(&u), r#"{"name":"Bo","age":1,"email":null}"#);
        assert_eq!(parse::<User>(r#"{"name":"Bo","age":1}"#), Ok(u));
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let got = parse::<User>(r#"{"name":"Bo","age":1,"extra":[1,2]}"#);
        assert_eq!(got.map(|u| u.age), Ok(1));
    }

    #[test]
    fn a_missing_required_key_names_it() {
        match parse::<User>(r#"{"name":"Bo"}"#) {
            Err(e) => assert!(e.message().contains("missing field `age`"), "{e}"),
            Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn a_number_out_of_range_is_an_error() {
        match parse::<User>(r#"{"name":"Bo","age":300}"#) {
            Err(e) => assert!(e.message().contains("300"), "{e}"),
            Ok(_) => panic!("expected an error"),
        }
    }

    #[test]
    fn bad_json_is_an_error() {
        assert!(parse::<User>("{nope").is_err());
    }

    #[test]
    fn a_map_round_trips_as_values() {
        let mut m: HashMap<String, i32> = HashMap::new();
        m.insert("a".into(), 1);
        m.insert("b".into(), 2);
        m.insert("c".into(), 3);
        let text = stringify(&m);
        assert_eq!(parse::<HashMap<String, i32>>(&text), Ok(m));
    }

    #[test]
    fn stringify_takes_unsized_values() {
        assert_eq!(stringify(&[1, 2][..]), "[1,2]");
        assert_eq!(stringify("hi"), "\"hi\"");
    }
}
