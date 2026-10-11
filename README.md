# chunker-worldgen

A bit-exact Rust reimplementation of Minecraft: Java Edition world generation.

## Documentation:
 - ### [English](README/en/README.md)
 - ### [Русский](README/ru/README.md)

## Overview

chunker-worldgen generates the same chunks as the vanilla server for a given seed, data packs and dimension, block for block: biomes, terrain, caves, ores, trees, every feature and every structure of the Overworld, the Nether and the End, with the mobs that structures place. Generated chunks are written in the vanilla format and can be opened by a regular server.

The code is written from scratch against the decompiled vanilla server and is checked against the real server classes: the tests compare the results bit for bit. Supported version: Minecraft 26.3.

## Quick start

Generate a few chunks and write them to a region file:

```bash
git clone https://github.com/mochensky/chunker-worldgen
cd chunker-worldgen
cargo run --release --example terrain
```

Or add the library to your project:

```toml
[dependencies]
chunker-worldgen = { git = "https://github.com/mochensky/chunker-worldgen" }
chunker-data = { git = "https://github.com/mochensky/chunker-worldgen" }
```

The server jar with the vanilla data is downloaded from Mojang automatically. See the full guide in the documentation linked above.

## License

MIT or Apache-2.0, at your option.

Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft.
