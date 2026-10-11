//! Vanilla data access.
//!
//! * [`Identifier`] — `namespace:path` resource locations.
//! * [`DataPacks`] — layered resource lookup (vanilla jar first, then world data packs).
//! * [`blocks::BlockRegistry`] — block/state table per Minecraft version (embedded).
//! * [`entities::EntityTables`] — default entity attributes and item enchantability (embedded).
//! * [`vanilla`] — locating or downloading the official server jar for a version.

pub mod blocks;
pub mod entities;
mod identifier;
mod packs;
pub mod vanilla;
mod version;

pub use identifier::Identifier;
pub use packs::{DataPacks, PackSource, TagEntry};
pub use version::McVersion;
