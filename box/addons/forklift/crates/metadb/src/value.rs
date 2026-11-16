//! The dynamically typed value both backends bind and return, and the
//! conversions between it and Rust types.

use crate::error::{Error, Result};

/// A column or parameter value. The variants follow SQLite's storage classes,
/// which is the model the store's SQL was written against.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    const fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Integer(_) => "integer",
            Self::Real(_) => "real",
            Self::Text(_) => "text",
            Self::Blob(_) => "blob",
        }
    }
}

/// A Rust value that can be bound as a statement parameter.
pub trait ToSql {
    fn to_value(&self) -> Value;
}

/// A Rust value that can be read from a result column.
pub trait FromSql: Sized {
    fn from_value(value: Value) -> Result<Self>;
}

fn mismatch<T>(want: &'static str, got: &Value) -> Result<T> {
    Err(Error::Conversion(format!(
        "expected {want}, got {}",
        got.kind()
    )))
}

impl ToSql for Value {
    fn to_value(&self) -> Value {
        self.clone()
    }
}

impl<T: ToSql + ?Sized> ToSql for &T {
    fn to_value(&self) -> Value {
        (**self).to_value()
    }
}

impl<T: ToSql> ToSql for Option<T> {
    fn to_value(&self) -> Value {
        self.as_ref().map_or(Value::Null, ToSql::to_value)
    }
}

impl ToSql for str {
    fn to_value(&self) -> Value {
        Value::Text(self.to_owned())
    }
}

impl ToSql for String {
    fn to_value(&self) -> Value {
        Value::Text(self.clone())
    }
}

impl ToSql for [u8] {
    fn to_value(&self) -> Value {
        Value::Blob(self.to_vec())
    }
}

impl ToSql for Vec<u8> {
    fn to_value(&self) -> Value {
        Value::Blob(self.clone())
    }
}

impl ToSql for bool {
    fn to_value(&self) -> Value {
        Value::Integer(i64::from(*self))
    }
}

impl ToSql for f64 {
    fn to_value(&self) -> Value {
        Value::Real(*self)
    }
}

macro_rules! integer_to_sql {
    ($($t:ty),*) => {$(
        impl ToSql for $t {
            fn to_value(&self) -> Value {
                Value::Integer(i64::from(*self))
            }
        }
    )*};
}
integer_to_sql!(i8, i16, i32, i64, u8, u16, u32);

impl ToSql for usize {
    fn to_value(&self) -> Value {
        Value::Integer(i64::try_from(*self).unwrap_or(i64::MAX))
    }
}

impl ToSql for u64 {
    fn to_value(&self) -> Value {
        Value::Integer(i64::try_from(*self).unwrap_or(i64::MAX))
    }
}

impl FromSql for Value {
    fn from_value(value: Value) -> Result<Self> {
        Ok(value)
    }
}

impl<T: FromSql> FromSql for Option<T> {
    fn from_value(value: Value) -> Result<Self> {
        match value {
            Value::Null => Ok(None),
            other => T::from_value(other).map(Some),
        }
    }
}

impl FromSql for String {
    fn from_value(value: Value) -> Result<Self> {
        match value {
            Value::Text(s) => Ok(s),
            other => mismatch("text", &other),
        }
    }
}

impl FromSql for Vec<u8> {
    fn from_value(value: Value) -> Result<Self> {
        match value {
            Value::Blob(b) => Ok(b),
            Value::Text(s) => Ok(s.into_bytes()),
            other => mismatch("blob", &other),
        }
    }
}

impl FromSql for bool {
    fn from_value(value: Value) -> Result<Self> {
        match value {
            Value::Integer(i) => Ok(i != 0),
            other => mismatch("integer", &other),
        }
    }
}

impl FromSql for f64 {
    fn from_value(value: Value) -> Result<Self> {
        match value {
            Value::Real(f) => Ok(f),
            Value::Integer(i) => Ok(i as Self),
            other => mismatch("real", &other),
        }
    }
}

impl FromSql for i64 {
    fn from_value(value: Value) -> Result<Self> {
        match value {
            Value::Integer(i) => Ok(i),
            other => mismatch("integer", &other),
        }
    }
}

macro_rules! integer_from_sql {
    ($($t:ty),*) => {$(
        impl FromSql for $t {
            fn from_value(value: Value) -> Result<Self> {
                let i = i64::from_value(value)?;
                Self::try_from(i)
                    .map_err(|_| Error::Conversion(format!("integer {i} out of range")))
            }
        }
    )*};
}
integer_from_sql!(i8, i16, i32, u8, u16, u32, u64, usize);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_round_trip_and_reject_overflow() {
        assert_eq!(7_u32.to_value(), Value::Integer(7));
        assert_eq!(u32::from_value(Value::Integer(7)).unwrap(), 7);
        assert!(u8::from_value(Value::Integer(300)).is_err());
        assert!(i64::from_value(Value::Text("7".into())).is_err());
    }

    #[test]
    fn options_map_null() {
        assert_eq!(None::<i64>.to_value(), Value::Null);
        assert_eq!(Option::<String>::from_value(Value::Null).unwrap(), None);
        assert_eq!(
            Option::<String>::from_value(Value::Text("a".into())).unwrap(),
            Some("a".to_string())
        );
        assert!(String::from_value(Value::Null).is_err());
    }

    #[test]
    fn bool_and_real_follow_sqlite_affinity() {
        assert_eq!(true.to_value(), Value::Integer(1));
        assert!(bool::from_value(Value::Integer(2)).unwrap());
        assert!((f64::from_value(Value::Integer(2)).unwrap() - 2.0).abs() < f64::EPSILON);
        assert_eq!(
            Vec::<u8>::from_value(Value::Text("ab".into())).unwrap(),
            b"ab"
        );
    }
}
