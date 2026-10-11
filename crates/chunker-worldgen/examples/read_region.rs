//! Lists the chunks of an Anvil region file with their generation status and section count.
//!
//! ```text
//! cargo run --release --example read_region -- <region dir> <region x> <region z>
//! cargo run --release --example read_region -- target/terrain-example -1 -2
//! ```

use std::path::Path;

use anyhow::{Context, Result};
use chunker_anvil::{Region, RegionPos};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let dir = args
        .next()
        .context("usage: read_region <region dir> <region x> <region z>")?;
    let x: i32 = args.next().context("region x")?.parse()?;
    let z: i32 = args.next().context("region z")?.parse()?;

    let pos = RegionPos::new(x, z);
    let region = Region::load(Path::new(&dir), pos)?;
    println!("{}: {} chunks", pos.file_name(), region.chunk_count());
    for index in 0..1024 {
        let Some(raw) = region.get(index) else { continue };
        let nbt = raw.decode_nbt()?;
        let status = nbt.get_str("Status").unwrap_or("?");
        let sections = nbt.get_list("sections").map_or(0, |l| l.len());
        let (cx, cz) = (x * 32 + (index % 32) as i32, z * 32 + (index / 32) as i32);
        println!("chunk {cx:>5} {cz:>5}: {status}, {sections} sections");
    }
    Ok(())
}
