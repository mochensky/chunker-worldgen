//! Bit-exact reimplementation of Minecraft: Java Edition world generation.
//!
//! Given a seed, the data packs of a world and a dimension, this crate produces the chunks the
//! vanilla server produces, block for block: biomes ([`biome`]), terrain with aquifers, surface
//! rules and carvers ([`terrain`]), structure starts and pieces ([`structure`]) and every feature
//! of the `features` step ([`feature`]). Chunks serialize to vanilla chunk NBT
//! ([`chunk::serialize`]).
//!
//! Vanilla data is read at runtime from the official server jar through [`chunker_data`]; start
//! with [`registry::WorldgenRegistries`] and [`terrain::TerrainGenerator`]. The `terrain` example
//! of the repository walks through generating and saving chunks.
//!
//! The modules follow vanilla's classes, and doc comments name the vanilla original. Floating
//! point code keeps Java's operand types and evaluation order, because the goal is identical
//! output, not similar output.

pub mod biome;
pub mod block_set;
pub mod chunk;
pub mod codec;
pub mod density;
pub mod entity;
pub mod feature;
pub mod java;
pub mod math;
pub mod noise;
pub mod providers;
pub mod random;
pub mod random_state;
pub mod registry;
pub mod settings;
pub mod structure;
pub mod terrain;
