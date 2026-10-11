# chunker-worldgen

A bit-exact Rust reimplementation of Minecraft: Java Edition world generation.

## Documentation:
 - ### [English](https://github.com/mochensky/chunker-worldgen/blob/main/README/en/README.md)
 - ### [Русский](https://github.com/mochensky/chunker-worldgen/blob/main/README/ru/README.md)

## Overview

chunker-worldgen generates the same chunks as the vanilla server for a given seed, data packs and dimension, block for block: biomes, terrain, caves, ores, trees, every feature and every structure of the Overworld, the Nether and the End, with the mobs that structures place. It opens a server or world folder, pre-generates areas of it in parallel and writes vanilla region files, with progress, pause, resume after a restart and map previews.

Supported version: Minecraft 26.3.

## Quick start

```toml
[dependencies]
chunker-worldgen = "0.1"
```

```rust
use std::path::PathBuf;
use chunker_worldgen::job::{JobSpec, Shape};
use chunker_worldgen::{World, WorldOptions};

let world = World::open(WorldOptions {
    path: PathBuf::from("my-server"),
    server_jar: None,
    cache_dir: PathBuf::from("cache"),
    allow_download: true,
})?;
let job = world.start(JobSpec {
    dimension: "minecraft:overworld".into(),
    center_x: 0,
    center_z: 0,
    radius: 1024,
    shape: Shape::Square,
    skip_existing: true,
    threads: 0,
    decorate: true,
})?;
world.wait();
```

Stop the server first: the job locks the world like the server does. The server jar with the vanilla data is found in the server folder or downloaded from Mojang.

## License

MIT or Apache-2.0, at your option.

Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft.
