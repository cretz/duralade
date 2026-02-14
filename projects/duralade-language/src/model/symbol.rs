use std::fmt;
use std::sync::{Arc, LazyLock};

use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(Arc<str>);

macro_rules! well_known {
    ($name:ident, $lit:expr) => {
        pub fn $name() -> Self {
            static S: LazyLock<Symbol> = LazyLock::new(|| Symbol(Arc::from($lit)));
            S.clone()
        }
    };
}

impl Symbol {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    well_known!(empty, "");
    well_known!(unknown, "<unknown>");
    well_known!(id, "id");
    well_known!(default_patch, "@default");

    // Primitive type names
    well_known!(int, "int");
    well_known!(float, "float");
    well_known!(str, "str");
    well_known!(bool, "bool");
    well_known!(error, "error");

    // Collection type names
    well_known!(array, "array");
    well_known!(map, "map");

    // Primitive module paths
    well_known!(mod_int, "duralade.int");
    well_known!(mod_float, "duralade.float");
    well_known!(mod_str, "duralade.str");
    well_known!(mod_bool, "duralade.bool");
    well_known!(mod_error, "duralade.error");

    // Collection module paths
    well_known!(mod_array, "duralade.array");
    well_known!(mod_map, "duralade.map");

    // Runtime sentinel names
    well_known!(anon_func, "<anon_func>");
    well_known!(message, "message");
    well_known!(value, "value");
    well_known!(run, "run");
    well_known!(init, "init");
    well_known!(iter, "iter");

    // Stdlib field names
    well_known!(key, "key");
    well_known!(old_value, "old_value");
    well_known!(result, "result");

    // Type parameter names
    well_known!(k, "k");
    well_known!(t, "t");
    well_known!(v, "v");

    // Internal native field names
    well_known!(underscore_items, "_items");
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Symbol({})", self.0)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Symbol {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Symbol {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(|s| s.into())
    }
}

impl std::borrow::Borrow<str> for Symbol {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for Symbol {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl Default for Symbol {
    fn default() -> Self {
        Self::empty()
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Symbol(Arc::from(s))
    }
}

impl From<String> for Symbol {
    fn from(s: String) -> Self {
        Symbol(Arc::from(s))
    }
}
