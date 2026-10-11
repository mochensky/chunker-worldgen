# chunker-data

Vanilla data for [chunker-worldgen](https://github.com/mochensky/chunker-worldgen): identifiers, the data pack stack (vanilla jar and world data packs), tags, the block registry and its embedded tables, and finding or downloading the official server jar.

## Overview

- **Identifiers** — `namespace:path` resource locations.
- **Data packs** — layered lookup of JSON, NBT and tags, in the order of `level.dat`.
- **Blocks** — every block, property and state of a Minecraft version, with properties worldgen needs (`BlockFlags`, map colours, rotations).
- **Server jar** — `vanilla::locate` finds the jar in a server folder or a cache and downloads it from Mojang with a SHA-1 check.

Supported version: Minecraft 26.3. Mojang data is not included; it is read from the server jar at runtime.

## License

MIT or Apache-2.0, at your option.
