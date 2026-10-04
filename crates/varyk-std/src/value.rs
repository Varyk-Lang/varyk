/// One value passed beside a call's other arguments, as a facade's last
/// `Vec<varyk_std::Value>` parameter takes them: a SQL parameter, for
/// one. Generated code builds it with `Value::from`, which never fails.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

impl From<bool> for Value {
    fn from(value: bool) -> Value {
        Value::Bool(value)
    }
}

/// The string is copied: the callee owns its values.
impl From<&str> for Value {
    fn from(value: &str) -> Value {
        Value::Text(value.to_string())
    }
}

impl From<f32> for Value {
    fn from(value: f32) -> Value {
        Value::Float(f64::from(value))
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Value {
        Value::Float(value)
    }
}

/// `From` for each integer type that fits `i64`.
macro_rules! from_int {
    ($($int:ty),*) => {$(
        impl From<$int> for Value {
            fn from(value: $int) -> Value {
                Value::Int(i64::from(value))
            }
        }
    )*};
}

from_int!(i8, i16, i32, i64, u8, u16, u32);

/// `From` for `Option` of each type above: `None` is `Null`.
macro_rules! from_option {
    ($($ty:ty),*) => {$(
        impl From<Option<$ty>> for Value {
            fn from(value: Option<$ty>) -> Value {
                value.map_or(Value::Null, Value::from)
            }
        }
    )*};
}

from_option!(bool, &str, f32, f64, i8, i16, i32, i64, u8, u16, u32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_type_gives_its_variant() {
        assert_eq!(Value::from(true), Value::Bool(true));
        assert_eq!(Value::from("Ada"), Value::Text("Ada".to_string()));
        assert_eq!(Value::from(1.5f32), Value::Float(1.5));
        assert_eq!(Value::from(2.5f64), Value::Float(2.5));
        assert_eq!(Value::from(-8i8), Value::Int(-8));
        assert_eq!(Value::from(-16i16), Value::Int(-16));
        assert_eq!(Value::from(-32i32), Value::Int(-32));
        assert_eq!(Value::from(i64::MIN), Value::Int(i64::MIN));
        assert_eq!(Value::from(u8::MAX), Value::Int(255));
        assert_eq!(Value::from(u16::MAX), Value::Int(65535));
        assert_eq!(Value::from(u32::MAX), Value::Int(4294967295));
    }

    #[test]
    fn none_is_null_and_some_is_its_value() {
        assert_eq!(Value::from(None::<bool>), Value::Null);
        assert_eq!(Value::from(None::<&str>), Value::Null);
        assert_eq!(Value::from(None::<f32>), Value::Null);
        assert_eq!(Value::from(None::<f64>), Value::Null);
        assert_eq!(Value::from(None::<i8>), Value::Null);
        assert_eq!(Value::from(None::<i16>), Value::Null);
        assert_eq!(Value::from(None::<i32>), Value::Null);
        assert_eq!(Value::from(None::<i64>), Value::Null);
        assert_eq!(Value::from(None::<u8>), Value::Null);
        assert_eq!(Value::from(None::<u16>), Value::Null);
        assert_eq!(Value::from(None::<u32>), Value::Null);
        assert_eq!(Value::from(Some("x")), Value::Text("x".to_string()));
        assert_eq!(Value::from(Some(true)), Value::Bool(true));
        assert_eq!(Value::from(Some(7u32)), Value::Int(7));
        assert_eq!(Value::from(Some(0.5f64)), Value::Float(0.5));
    }

    #[test]
    fn a_value_clones_and_prints() {
        let value = Value::from("a");
        assert_eq!(value.clone(), value);
        assert_eq!(format!("{value:?}"), "Text(\"a\")");
    }
}
