//! ID newtypes.
//!
//! Domain entities use zero-cost `i64` newtypes (`TaskId`, `EpicId`,
//! `LearningId`) instead of bare integers. The [`define_id_newtype!`] macro
//! generates each wrapper with `Display`, `From`/`Into<i64>`, `FromStr`,
//! `Serialize`/`Deserialize`, and a set of unit tests.
//!
//! The macro is `#[macro_export]`ed, so it lives at the crate root. Each
//! consuming module (`tasks`, `epics`, `learnings`) brings it into scope with
//! `use crate::define_id_newtype;`.

/// Generate a zero-cost i64 newtype with Display, From/Into<i64>, FromStr,
/// Serialize/Deserialize, and basic unit tests.
#[macro_export]
macro_rules! define_id_newtype {
    ($(#[$attr:meta])* $name:ident, $test_mod:ident) => {
        $(#[$attr])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub i64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl From<i64> for $name {
            fn from(v: i64) -> Self {
                $name(v)
            }
        }

        impl From<$name> for i64 {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl std::str::FromStr for $name {
            type Err = std::num::ParseIntError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.parse::<i64>().map($name)
            }
        }

        #[cfg(test)]
        mod $test_mod {
            use super::$name;

            #[test]
            fn display() {
                assert_eq!($name(42).to_string(), "42");
            }

            #[test]
            fn copy_eq_hash() {
                let a = $name(1);
                let b = a;
                assert_eq!(a, b);
                let mut set = std::collections::HashSet::new();
                set.insert(a);
                assert!(set.contains(&b));
            }

            #[test]
            fn debug_contains_value() {
                assert!(format!("{:?}", $name(7)).contains("7"));
            }

            #[test]
            fn from_into_i64() {
                let id = $name::from(5i64);
                let raw: i64 = id.into();
                assert_eq!(raw, 5);
            }
        }
    };
}
