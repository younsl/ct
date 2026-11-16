//! Binds and reads [`Value`] over the PostgreSQL binary protocol.
//!
//! The store's SQL was written for SQLite, where a value adapts to the column
//! it meets. PostgreSQL fixes every parameter's type when it parses the
//! statement, so a value is encoded as whatever type the server inferred.

use std::error::Error as StdError;

use bytes::{BufMut, BytesMut};
use postgres_types::{FromSql, IsNull, ToSql, Type, to_sql_checked};

use crate::value::Value;

type BoxError = Box<dyn StdError + Sync + Send>;

const fn is_text(ty: &Type) -> bool {
    matches!(
        *ty,
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN | Type::JSON
    )
}

fn put_text(ty: &Type, text: &str, out: &mut BytesMut) -> Result<IsNull, BoxError> {
    if *ty == Type::JSONB {
        // jsonb's binary form is a version byte followed by the JSON text.
        out.put_u8(1);
    } else if !is_text(ty) {
        return Err(format!("cannot bind text to a {ty} parameter").into());
    }
    out.put_slice(text.as_bytes());
    Ok(IsNull::No)
}

fn put_integer(ty: &Type, i: i64, out: &mut BytesMut) -> Result<IsNull, BoxError> {
    match *ty {
        Type::INT8 => out.put_i64(i),
        Type::INT4 => out.put_i32(i32::try_from(i)?),
        Type::INT2 => out.put_i16(i16::try_from(i)?),
        Type::OID => out.put_u32(u32::try_from(i)?),
        Type::BOOL => out.put_u8(u8::from(i != 0)),
        Type::FLOAT8 => out.put_f64(i as f64),
        Type::FLOAT4 => out.put_f32(i as f32),
        _ => return put_text(ty, &i.to_string(), out),
    }
    Ok(IsNull::No)
}

fn put_real(ty: &Type, f: f64, out: &mut BytesMut) -> Result<IsNull, BoxError> {
    match *ty {
        Type::FLOAT8 => out.put_f64(f),
        Type::FLOAT4 => out.put_f32(f as f32),
        Type::INT8 | Type::INT4 | Type::INT2 if f.fract() == 0.0 => {
            return put_integer(ty, f as i64, out);
        }
        _ => return put_text(ty, &f.to_string(), out),
    }
    Ok(IsNull::No)
}

fn put_string(ty: &Type, s: &str, out: &mut BytesMut) -> Result<IsNull, BoxError> {
    match *ty {
        Type::INT8 | Type::INT4 | Type::INT2 | Type::OID | Type::BOOL => {
            put_integer(ty, s.trim().parse::<i64>()?, out)
        }
        Type::FLOAT8 | Type::FLOAT4 => put_real(ty, s.trim().parse::<f64>()?, out),
        Type::BYTEA => {
            out.put_slice(s.as_bytes());
            Ok(IsNull::No)
        }
        _ => put_text(ty, s, out),
    }
}

impl ToSql for Value {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BoxError> {
        match self {
            Self::Null => Ok(IsNull::Yes),
            Self::Integer(i) => put_integer(ty, *i, out),
            Self::Real(f) => put_real(ty, *f, out),
            Self::Text(s) => put_string(ty, s, out),
            Self::Blob(b) => {
                if *ty == Type::BYTEA {
                    out.put_slice(b);
                    Ok(IsNull::No)
                } else {
                    put_text(ty, std::str::from_utf8(b)?, out)
                }
            }
        }
    }

    fn accepts(_: &Type) -> bool {
        true
    }

    to_sql_checked!();
}

/// A column value decoded into the store's value model.
pub struct PgValue(pub Value);

fn fixed<const N: usize>(raw: &[u8]) -> Result<[u8; N], BoxError> {
    raw.try_into()
        .map_err(|_| format!("expected {N} bytes, got {}", raw.len()).into())
}

/// Decodes the binary `numeric` form: a header of four 16-bit fields followed
/// by base-10000 digits, most significant first.
fn numeric(raw: &[u8]) -> Result<Value, BoxError> {
    if raw.len() < 8 {
        return Err("short numeric".into());
    }
    let field = |i: usize| i16::from_be_bytes([raw[i * 2], raw[i * 2 + 1]]);
    let ndigits = usize::try_from(field(0))?;
    let weight = i32::from(field(1));
    let negative = match u16::from_be_bytes([raw[4], raw[5]]) {
        0x0000 => false,
        0x4000 => true,
        _ => return Err("numeric is NaN or infinite".into()),
    };
    let dscale = field(3);
    if raw.len() < 8 + ndigits * 2 {
        return Err("truncated numeric".into());
    }
    let digits: Vec<i64> = (0..ndigits).map(|i| i64::from(field(4 + i))).collect();

    // An integral value has no digit below the 10000^0 position.
    let integral = dscale == 0 && i32::try_from(ndigits)? <= weight + 1;
    if integral {
        let mut acc: i128 = 0;
        for i in 0..=weight {
            let d = usize::try_from(i).ok().and_then(|i| digits.get(i)).copied();
            acc = acc
                .checked_mul(10_000)
                .and_then(|a| a.checked_add(i128::from(d.unwrap_or(0))))
                .ok_or("numeric overflow")?;
        }
        if negative {
            acc = -acc;
        }
        if let Ok(v) = i64::try_from(acc) {
            return Ok(Value::Integer(v));
        }
        return Ok(Value::Real(acc as f64));
    }

    let mut acc = 0.0_f64;
    for (i, d) in digits.iter().enumerate() {
        let exp = weight - i32::try_from(i)?;
        acc += (*d as f64) * 10_000_f64.powi(exp);
    }
    Ok(Value::Real(if negative { -acc } else { acc }))
}

impl<'a> FromSql<'a> for PgValue {
    fn from_sql(ty: &Type, raw: &'a [u8]) -> Result<Self, BoxError> {
        let value = match *ty {
            Type::INT8 => Value::Integer(i64::from_be_bytes(fixed(raw)?)),
            Type::INT4 => Value::Integer(i64::from(i32::from_be_bytes(fixed(raw)?))),
            Type::INT2 => Value::Integer(i64::from(i16::from_be_bytes(fixed(raw)?))),
            Type::OID => Value::Integer(i64::from(u32::from_be_bytes(fixed(raw)?))),
            Type::BOOL => Value::Integer(i64::from(fixed::<1>(raw)?[0] != 0)),
            Type::FLOAT8 => Value::Real(f64::from_be_bytes(fixed(raw)?)),
            Type::FLOAT4 => Value::Real(f64::from(f32::from_be_bytes(fixed(raw)?))),
            Type::NUMERIC => numeric(raw)?,
            Type::BYTEA => Value::Blob(raw.to_vec()),
            Type::JSONB => {
                Value::Text(std::str::from_utf8(raw.get(1..).unwrap_or_default())?.to_owned())
            }
            Type::VOID => Value::Null,
            _ => match std::str::from_utf8(raw) {
                Ok(s) => Value::Text(s.to_owned()),
                Err(_) => Value::Blob(raw.to_vec()),
            },
        };
        Ok(Self(value))
    }

    fn from_sql_null(_: &Type) -> Result<Self, BoxError> {
        Ok(Self(Value::Null))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(v: &Value, ty: &Type) -> Vec<u8> {
        let mut out = BytesMut::new();
        v.to_sql(ty, &mut out).expect("encode");
        out.to_vec()
    }

    fn decode(ty: &Type, raw: &[u8]) -> Value {
        PgValue::from_sql(ty, raw).expect("decode").0
    }

    #[test]
    fn integers_adapt_to_the_inferred_parameter_type() {
        assert_eq!(encode(&Value::Integer(5), &Type::INT8), 5_i64.to_be_bytes());
        assert_eq!(encode(&Value::Integer(5), &Type::INT4), 5_i32.to_be_bytes());
        assert_eq!(encode(&Value::Integer(1), &Type::BOOL), [1]);
        assert_eq!(encode(&Value::Integer(5), &Type::TEXT), b"5");
    }

    #[test]
    fn text_adapts_to_numeric_and_json_parameters() {
        assert_eq!(
            encode(&Value::Text("42".into()), &Type::INT8),
            42_i64.to_be_bytes()
        );
        assert_eq!(encode(&Value::Text("{}".into()), &Type::JSONB), b"\x01{}");
        let mut out = BytesMut::new();
        assert!(
            Value::Text("abc".into())
                .to_sql(&Type::INT8, &mut out)
                .is_err()
        );
    }

    #[test]
    fn null_is_null_for_every_type() {
        let mut out = BytesMut::new();
        assert!(matches!(
            Value::Null.to_sql(&Type::INT8, &mut out),
            Ok(IsNull::Yes)
        ));
        assert_eq!(
            PgValue::from_sql_null(&Type::TEXT).expect("null").0,
            Value::Null
        );
    }

    #[test]
    fn columns_decode_into_sqlite_storage_classes() {
        assert_eq!(decode(&Type::INT4, &7_i32.to_be_bytes()), Value::Integer(7));
        assert_eq!(decode(&Type::BOOL, &[1]), Value::Integer(1));
        assert_eq!(decode(&Type::TEXT, b"hi"), Value::Text("hi".into()));
        assert_eq!(decode(&Type::BYTEA, &[0xff]), Value::Blob(vec![0xff]));
        assert_eq!(decode(&Type::JSONB, b"\x01{}"), Value::Text("{}".into()));
    }

    /// `SUM` over a bigint column comes back as numeric.
    #[test]
    fn numeric_decodes_integers_and_fractions() {
        // 12345678 = digits [1234, 5678], weight 1.
        let raw = [0, 2, 0, 1, 0, 0, 0, 0, 0x04, 0xd2, 0x16, 0x2e];
        assert_eq!(decode(&Type::NUMERIC, &raw), Value::Integer(12_345_678));
        // -20000 = digits [2], weight 1, trailing zero digit dropped.
        let raw = [0, 1, 0, 1, 0x40, 0, 0, 0, 0, 2];
        assert_eq!(decode(&Type::NUMERIC, &raw), Value::Integer(-20_000));
        // 0 has no digits at all.
        let raw = [0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(decode(&Type::NUMERIC, &raw), Value::Integer(0));
        // 1.5 = digits [1, 5000], weight 0, dscale 1.
        let raw = [0, 2, 0, 0, 0, 0, 0, 1, 0, 1, 0x13, 0x88];
        assert_eq!(decode(&Type::NUMERIC, &raw), Value::Real(1.5));
    }
}
