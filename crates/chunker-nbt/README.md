# chunker-nbt

A small, fast NBT (Named Binary Tag) reader and writer for Minecraft: Java Edition. Part of [chunker-worldgen](https://github.com/mochensky/chunker-worldgen).

## Overview

- **Format** — big endian, Modified UTF-8 strings, every tag type.
- **Compounds** — keep insertion order, so the written output is deterministic.
- **Compression** — `read_compressed` / `write_compressed` for gzip files like `level.dat`.

```rust
let level = chunker_nbt::read_compressed(std::fs::File::open("level.dat")?)?;
println!("{:?}", level.get_compound("Data").and_then(|d| d.get_str("LevelName")));
```

## License

MIT or Apache-2.0, at your option.
