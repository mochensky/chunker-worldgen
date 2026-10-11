use std::fmt;

/// Minecraft Java Edition versions supported by chunker-worldgen.
///
/// Adding a version means: embedding its block table (`data/<version>/blocks.txt`), and branching
/// worldgen code where vanilla behaviour differs (see "Adding a Minecraft version" in README/en/README.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum McVersion {
    V26_3,
}

impl McVersion {
    pub const LATEST: McVersion = McVersion::V26_3;
    pub const ALL: &'static [McVersion] = &[McVersion::V26_3];

    pub fn id(self) -> &'static str {
        match self {
            McVersion::V26_3 => "26.3",
        }
    }

    /// `SharedConstants.WORLD_VERSION` / chunk `DataVersion`.
    pub fn data_version(self) -> i32 {
        match self {
            McVersion::V26_3 => 5023,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.id() == s)
    }

    pub fn from_data_version(data_version: i32) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|v| v.data_version() == data_version)
    }
}

impl fmt::Display for McVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}
