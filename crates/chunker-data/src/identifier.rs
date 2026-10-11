use std::fmt;
use std::sync::Arc;

/// A resource location (`namespace:path`). Cheap to clone.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Identifier(Arc<str>);

impl Identifier {
    pub const DEFAULT_NAMESPACE: &'static str = "minecraft";

    /// Parses `ns:path` or `path` (defaulting to `minecraft`). Returns `None` for invalid characters,
    /// mirroring vanilla `Identifier.tryParse`.
    pub fn parse(s: &str) -> Option<Self> {
        let (ns, path) = match s.split_once(':') {
            Some((ns, path)) => (
                if ns.is_empty() {
                    Self::DEFAULT_NAMESPACE
                } else {
                    ns
                },
                path,
            ),
            None => (Self::DEFAULT_NAMESPACE, s),
        };
        let valid_ns = ns
            .bytes()
            .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.'));
        let valid_path = path
            .bytes()
            .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'));
        if !valid_ns || !valid_path {
            return None;
        }
        Some(Self(format!("{ns}:{path}").into()))
    }

    /// Like [`Identifier::parse`] but panics on invalid input. For compile-time known names.
    pub fn new(s: &str) -> Self {
        Self::parse(s).unwrap_or_else(|| panic!("invalid identifier {s:?}"))
    }

    pub fn vanilla(path: &str) -> Self {
        Self::new(&format!("minecraft:{path}"))
    }

    pub fn namespace(&self) -> &str {
        self.0.split_once(':').unwrap().0
    }

    pub fn path(&self) -> &str {
        self.0.split_once(':').unwrap().1
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_defaults() {
        assert_eq!(Identifier::parse("stone").unwrap().as_str(), "minecraft:stone");
        assert_eq!(Identifier::parse("a:b/c").unwrap().path(), "b/c");
        assert!(Identifier::parse("Bad:Name").is_none());
    }
}
