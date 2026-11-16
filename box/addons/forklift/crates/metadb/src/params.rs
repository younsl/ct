//! Statement parameter lists, accepted in the same shapes rusqlite takes.

use crate::value::{ToSql, Value};

/// A list of values to bind, in placeholder order.
pub trait Params {
    fn into_values(self) -> Vec<Value>;
}

impl Params for [&dyn ToSql; 0] {
    fn into_values(self) -> Vec<Value> {
        Vec::new()
    }
}

impl Params for &[&dyn ToSql] {
    fn into_values(self) -> Vec<Value> {
        self.iter().map(|p| (*p).to_value()).collect()
    }
}

impl Params for Vec<Value> {
    fn into_values(self) -> Vec<Value> {
        self
    }
}

macro_rules! array_params {
    ($($n:literal),*) => {$(
        impl<T: ToSql> Params for [T; $n] {
            fn into_values(self) -> Vec<Value> {
                self.iter().map(ToSql::to_value).collect()
            }
        }
    )*};
}
array_params!(1, 2, 3, 4, 5, 6, 7, 8);

/// Parameters drawn from an iterator, for statements built with a variable
/// number of placeholders.
pub struct ParamsFromIter<I>(I);

/// Binds every item of `iter` in order.
pub const fn params_from_iter<I>(iter: I) -> ParamsFromIter<I>
where
    I: IntoIterator,
    I::Item: ToSql,
{
    ParamsFromIter(iter)
}

impl<I> Params for ParamsFromIter<I>
where
    I: IntoIterator,
    I::Item: ToSql,
{
    fn into_values(self) -> Vec<Value> {
        self.0.into_iter().map(|p| p.to_value()).collect()
    }
}

/// Builds a parameter slice from heterogeneous values.
#[macro_export]
macro_rules! params {
    () => {
        &[] as &[&dyn $crate::ToSql]
    };
    ($($param:expr),+ $(,)?) => {
        &[$(&$param as &dyn $crate::ToSql),+] as &[&dyn $crate::ToSql]
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shape_yields_values_in_order() {
        assert_eq!([].into_values(), Vec::<Value>::new());
        assert_eq!(
            params![1_i64, "a", None::<String>].into_values(),
            vec![Value::Integer(1), Value::Text("a".into()), Value::Null]
        );
        assert_eq!(["a", "b"].into_values().len(), 2);
        let names = ["x".to_string(), "y".to_string()];
        assert_eq!(
            params_from_iter(names.iter()).into_values(),
            vec![Value::Text("x".into()), Value::Text("y".into())]
        );
    }
}
