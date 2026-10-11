# chunker-anvil

Anvil region files (`.mca`, `.mcc`) of Minecraft: Java Edition: reading, writing and chunk compression (zlib, gzip, none). Part of [chunker-worldgen](https://github.com/mochensky/chunker-worldgen).

## Overview

- **Read** — `Region::load` loads a region file; `RawChunk::decode_nbt` returns a chunk's NBT.
- **Write** — `Region::set_nbt` and `Region::save` (the whole file at once, through a temporary file).
- **Large chunks** — chunks of 256 sectors or more go to `c.<x>.<z>.mcc`, like the server does.
- **Other compressions** — existing chunks are copied as they are, so formats like LZ4 survive a rewrite.

```rust
use chunker_anvil::{Region, RegionPos};

let region = Region::load(region_dir, RegionPos::new(0, 0))?;
if let Some(raw) = region.get(0) {
    println!("{:?}", raw.decode_nbt()?.get_str("Status"));
}
```

## License

MIT or Apache-2.0, at your option.
