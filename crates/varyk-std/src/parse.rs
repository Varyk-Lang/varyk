use crate::Error;

mod private {
    pub trait Sealed {}
}

/// A type `parse` can read from text: every Varyk number type and `bool`.
pub trait Parse: private::Sealed + Sized {
    fn parse_str(text: &str) -> Result<Self, Error>;
}

/// Reads a `T` from `text`; the error names the text.
pub fn parse<T: Parse>(text: &str) -> Result<T, Error> {
    T::parse_str(text)
}

macro_rules! int_parse {
    ($($t:ty),*) => {$(
        impl private::Sealed for $t {}
        impl Parse for $t {
            fn parse_str(text: &str) -> Result<$t, Error> {
                match text.parse::<$t>() {
                    Ok(v) => Ok(v),
                    Err(_) => Err(int_error(text)),
                }
            }
        }
    )*};
}

macro_rules! float_parse {
    ($($t:ty),*) => {$(
        impl private::Sealed for $t {}
        impl Parse for $t {
            fn parse_str(text: &str) -> Result<$t, Error> {
                match text.parse::<$t>() {
                    Ok(v) if float_fits(text, v.is_finite(), v == 0.0) => Ok(v),
                    _ if float_out_of_range(text) => Err(Error::new(format!(
                        "`{text}` is out of range for this number type"
                    ))),
                    _ => Err(Error::new(format!("`{text}` is not a number"))),
                }
            }
        }
    )*};
}

int_parse!(i8, i16, i32, i64, u8, u16, u32, u64, usize);
float_parse!(f32, f64);

/// Whether a float read from `text` is the number written: finite (Rust
/// also reads `NaN`, `inf`, and a too-large number as a float), and not
/// zero unless `text` is zero (a tiny number rounds to zero).
pub(crate) fn float_fits(text: &str, finite: bool, zero: bool) -> bool {
    finite && !(zero && nonzero_digits(text))
}

/// True when `text` is a number written with digits that a float type
/// could not hold; `NaN` and `inf` are not numbers at all. Called only
/// after `float_fits` failed or the parse did.
pub(crate) fn float_out_of_range(text: &str) -> bool {
    text.parse::<f64>().is_ok() && text.chars().any(|c| c.is_ascii_digit())
}

fn nonzero_digits(text: &str) -> bool {
    text.split(['e', 'E'])
        .next()
        .is_some_and(|digits| digits.chars().any(|c| ('1'..='9').contains(&c)))
}

/// True when `text` is a whole number that some integer type could not
/// hold: too big, or negative for an unsigned type. Called only after the
/// type's own parse failed.
pub(crate) fn is_integer_out_of_range(text: &str) -> bool {
    text.parse::<i128>().is_ok()
}

fn int_error(text: &str) -> Error {
    if is_integer_out_of_range(text) {
        Error::new(format!("`{text}` is out of range for this number type"))
    } else {
        Error::new(format!("`{text}` is not a number"))
    }
}

impl private::Sealed for bool {}
impl Parse for bool {
    fn parse_str(text: &str) -> Result<bool, Error> {
        match text {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(Error::new(format!("`{text}` is not `true` or `false`"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg<T: Parse + std::fmt::Debug>(text: &str) -> String {
        match parse::<T>(text) {
            Ok(v) => format!("ok {v:?}"),
            Err(e) => e.to_string(),
        }
    }

    #[test]
    fn integers() {
        assert_eq!(parse::<i32>("42"), Ok(42));
        assert_eq!(msg::<i32>("abc"), "`abc` is not a number");
        assert_eq!(msg::<i32>(""), "`` is not a number");
    }

    #[test]
    fn out_of_range_is_an_error() {
        assert_eq!(
            msg::<u8>("300"),
            "`300` is out of range for this number type"
        );
        assert_eq!(msg::<u8>("-1"), "`-1` is out of range for this number type");
    }

    #[test]
    fn bools() {
        assert_eq!(parse::<bool>("true"), Ok(true));
        assert_eq!(parse::<bool>("false"), Ok(false));
        assert_eq!(msg::<bool>("yes"), "`yes` is not `true` or `false`");
    }

    #[test]
    fn trailing_text_and_floats() {
        assert_eq!(msg::<u16>("12abc"), "`12abc` is not a number");
        assert_eq!(parse::<f64>("2.5"), Ok(2.5));
        assert_eq!(msg::<f32>("x"), "`x` is not a number");
        assert_eq!(parse::<usize>("7"), Ok(7));
    }

    #[test]
    fn a_float_that_changes_or_is_not_a_number_is_an_error() {
        assert_eq!(parse::<f64>("0"), Ok(0.0));
        assert_eq!(parse::<f32>("0.0e5"), Ok(0.0));
        assert_eq!(
            msg::<f32>("1e40"),
            "`1e40` is out of range for this number type"
        );
        assert_eq!(
            msg::<f32>("1e-50"),
            "`1e-50` is out of range for this number type"
        );
        assert_eq!(
            msg::<f64>("1e-400"),
            "`1e-400` is out of range for this number type"
        );
        assert_eq!(msg::<f64>("NaN"), "`NaN` is not a number");
        assert_eq!(msg::<f32>("inf"), "`inf` is not a number");
    }
}
