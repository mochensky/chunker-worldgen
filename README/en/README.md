# chunker-worldgen

A bit-exact Rust reimplementation of Minecraft: Java Edition world generation. For a given seed, data packs and dimension it generates the same chunks as the vanilla server, block for block, and writes them in the vanilla format.

Supported version: **Minecraft 26.3** (data version 5023). The code is designed to support more versions.

## Features

- **Biomes** — multi-noise sources with the Overworld and Nether presets, The End, fixed and checkerboard sources.
- **Terrain** — noise fill, aquifers, surface rules, ore veins, caves and canyons, heightmaps.
- **Structures** — starts and references of every structure type: jigsaw structures (villages, ancient cities, trial chambers, bastions, ...), strongholds with their rings, mineshafts, fortresses, monuments, mansions, End cities and the rest.
- **Terrain adaptation** — the beardifier: terrain under villages, ancient cities and other structures is shaped like in vanilla.
- **Features** — the whole `features` step: every placed feature (ores, trees, lakes, geodes, ...), structure pieces with processors, block entities with loot tables, scheduled ticks.
- **Structure mobs** — villagers, iron golems, cats and farm animals in villages, piglins in bastions, drowned in ocean ruins, illagers in mansions and the rest, finalized like vanilla (`Mob.finalizeSpawn`): variants by biome, villager types, equipment, enchantments, chicken and zombie nautilus jockeys.
- **Vanilla data at runtime** — worldgen JSON and structure templates are read from the official server jar, together with the world's own data packs.
- **Vanilla formats** — chunk NBT, Anvil region files (`.mca` / `.mcc`), NBT reading and writing.
- **Verified against vanilla** — the tests run the real server classes and compare the results bit for bit.
- **Pre-generation** — `World` opens a server or world folder and generates an area in parallel, with progress, pause, resume after a restart and map previews.
- **Speed** — about 5 ms per chunk of terrain on one thread, about 1500 chunks per second with structures and features on 24 threads.

## Requirements

- Rust 1.90+
- Internet access on the first run (to download the server jar from Mojang), or a server jar on disk
- Java 25 (only for running the verification tests)

## Installation

### Option 1: crates.io (recommended)

```toml
[dependencies]
chunker-worldgen = "0.2"
```

The other crates come along as its dependencies; add them yourself only if you use them directly:

```toml
chunker-data = "0.2"
chunker-anvil = "0.2"
chunker-nbt = "0.2"
```

What each crate does:

- `chunker-worldgen` — world generation
- `chunker-data` — identifiers, data packs, tags, block registry, server jar lookup and download
- `chunker-anvil` — region files and chunk compression
- `chunker-nbt` — NBT

Generation is CPU-heavy, so build in release mode. To keep debug builds usable, optimize these crates in your dev profile:

```toml
[profile.dev.package.chunker-worldgen]
opt-level = 3
```

### Option 2: Git or a local copy

The latest code from the repository:

```toml
[dependencies]
chunker-worldgen = { git = "https://github.com/mochensky/chunker-worldgen" }
```

Or a local copy:

```bash
git clone https://github.com/mochensky/chunker-worldgen
```

```toml
[dependencies]
chunker-worldgen = { path = "../chunker-worldgen/crates/chunker-worldgen" }
chunker-data = { path = "../chunker-worldgen/crates/chunker-data" }
```

> **Important:** `World` and the jobs are the stable entry point; the lower-level modules expose many internals and may change before the first release.

## Pre-generating a world

`World` opens a server or world folder and generates an area of it with everything vanilla generates: terrain, structures, features and structure mobs. Chunks are written to the world's region files at the `features` status; the server adds light and spawns animals when it loads them.

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
    radius_z: None,
    shape: Shape::Square,
    skip_existing: true,
    threads: 0,
    decorate: true,
})?;

while job.is_active() {
    let s = job.snapshot();
    println!("{}/{} chunks, {:.0} chunks/s", s.generated_chunks + s.skipped_chunks, s.total_chunks, s.chunks_per_second);
    std::thread::sleep(std::time::Duration::from_secs(1));
}
world.wait();
```

> **Important:** stop the server first. The job takes `session.lock` like the server does and fails if the world is in use.

What the job does:

- **Order** — regions nearest to the center first; terrain in parallel, then the `features` step in 9 phases, so that neighbouring chunks are never decorated at the same time.
- **Existing chunks** — never overwritten. With `skip_existing` they are skipped, and they stand in for reads of their neighbours.
- **Pause and resume** — `world.pause()`, `world.resume()`, `world.cancel()`. Progress is saved in `<world>/.chunker/job.json`: `World::open` finds an unfinished job, and `resume()` continues it after a restart.
- **Terrain only** — `decorate: false` saves chunks at `terrain`, about a third faster; the server then places structures and features when the chunks first load.
- **Difficulty and time** — read from the world (`level.dat` and `data/minecraft/world_clocks.dat`); structure mobs get their equipment from them.
- **Map previews** — `world.previews(&dimension)` keeps a 128x128 map image per region (vanilla map colours and biomes), updated while the job runs. In the Nether the map shows the floor below the bedrock roof.
- **Size estimate** — `world.estimate_chunk_bytes(&spec)` generates sample chunks of the area like the job does and returns the bytes per chunk on disk, so it follows the dimension, the data packs and the version.

`JobSpec` fields:

| Field | Meaning |
|---|---|
| `dimension` | dimension id, e.g. `minecraft:overworld`, `minecraft:the_nether` |
| `center_x`, `center_z` | center in block coordinates |
| `radius` | radius in blocks (half the width along x for a rectangle) |
| `radius_z` | half the length along z of a `Rectangle`; `None` for the other shapes |
| `shape` | `Square`, `Circle` or `Rectangle`; a chunk belongs to the area when its center does |
| `skip_existing` | leave chunks that already exist as they are |
| `threads` | worker threads, `0` for all CPUs |
| `decorate` | run the `features` step (structures, features, mobs) |

The full program is [`examples/pregenerate.rs`](../../crates/chunker-worldgen/examples/pregenerate.rs):

```bash
cargo run --release --example pregenerate -- <server or world folder> 512
```

## Lower-level API

The parts a job is built from can be used directly. The steps below generate the terrain of single chunks in memory; the full program is [`examples/terrain.rs`](../../crates/chunker-worldgen/examples/terrain.rs).

### Step 1: Load the vanilla data

`vanilla::locate` looks for the server jar: an explicit path, a server folder (`versions/<v>/server-<v>.jar`, Paper's `cache/mojang_<v>.jar`, any jar in the folder), a cache folder. If nothing is found and downloading is allowed, it downloads the jar from Mojang and checks its SHA-1.

```rust
use std::sync::Arc;
use chunker_data::{DataPacks, McVersion, PackSource, blocks::BlockRegistry, vanilla};
use chunker_worldgen::registry::WorldgenRegistries;

let version = McVersion::V26_3;
let jar = vanilla::locate(version, None, Some(server_dir), cache_dir, true)?;

let mut packs = DataPacks::new();
packs.push(jar.vanilla_pack()?);
// The world's own data packs go on top, in the order of level.dat
packs.push(PackSource::directory(world_dir.join("datapacks/my_pack")));

let registries = Arc::new(WorldgenRegistries::new(packs, BlockRegistry::for_version(version)));
```

### Step 2: Create a generator

A generator needs the dimension's noise settings and biome source, the same values as in the world's `world_gen_settings.dat`.

```rust
use chunker_data::Identifier;
use chunker_worldgen::biome::{BiomeRegistry, BiomeSource};
use chunker_worldgen::chunk::BlockEnv;
use chunker_worldgen::random_state::RandomState;
use chunker_worldgen::terrain::TerrainGenerator;

let biomes = BiomeRegistry::load(&registries.packs)?;
let env = BlockEnv::load(&registries)?;
let settings = registries.noise_settings(&Identifier::vanilla("overworld"))?;
let random_state = Arc::new(RandomState::new(registries.clone(), seed, false));
let source = Arc::new(BiomeSource::parse(
    &registries,
    &biomes,
    &serde_json::json!({ "type": "minecraft:multi_noise", "preset": "minecraft:overworld" }),
)?);
let generator = TerrainGenerator::new(&registries, biomes.clone(), settings, random_state, source, env.clone())?;
```

The generator can be shared between threads, so chunks can be generated in parallel.

### Step 3: Generate a chunk

Surface rules and carvers read the biomes of the neighbouring chunks, so the biomes of the 3x3 chunks around the target are resolved first.

```rust
use chunker_worldgen::chunk::{ChunkStatus, HeightmapKind, ProtoChunk};
use chunker_worldgen::terrain::NeighborBiomes;

let resolvers: Vec<_> = (0..9)
    .map(|k| generator.resolve_biomes(cx + k % 3 - 1, cz + k / 3 - 1, -64, 384))
    .collect();
let neighbors = NeighborBiomes {
    center_x: cx,
    center_z: cz,
    chunks: std::array::from_fn(|k| &resolvers[k]),
    beardifier: None,
};

let mut chunk = ProtoChunk::new(env.clone(), cx, cz, -64, 384);
chunk.fill_biomes(&resolvers[4]);
chunk.status = ChunkStatus::Biomes;
generator.build_terrain(&mut chunk, &neighbors);
chunk.status = ChunkStatus::Terrain;

let y = chunk.height_at(HeightmapKind::WorldSurfaceWg, 8, 8);
println!("surface: {} at y={y}", chunk.blocks.describe(chunk.get_block(8, y, 8)));
```

`beardifier` takes the structure starts around the chunk; without them the terrain is generated as if there were no structures.

### Step 4: Save the chunk

```rust
use chunker_anvil::{Compression, Region, RegionPos};
use chunker_worldgen::chunk::serialize::write_chunk;

let mut region = Region::empty(RegionPos::of_chunk(cx, cz));
let nbt = write_chunk(&chunk, &biomes, version.data_version(), 0);
let index = (cx & 31) as usize + (cz & 31) as usize * 32;
region.set_nbt(index, &nbt, Compression::Zlib, chunker_anvil::now_timestamp());
region.save(&world_dir.join("dimensions/minecraft/overworld/region"))?;
```

A server that opens the world continues generating saved chunks from their status: here from `terrain`, so it adds features, light and mobs itself. Structures are not added to such chunks: structure starts belong to an earlier step, see [Structures and features](#structures-and-features).

### Step 5: Run the example

```bash
cargo run --release --example terrain -- <seed> <chunk x> <chunk z> <output folder>
```

All arguments are optional. With the defaults you will see:

```
chunk  -12  -56: surface at y=70   minecraft:grass_block[snowy=false]
chunk  -11  -56: surface at y=62   minecraft:water[level=0]
...
wrote target/terrain-example/r.-1.-2.mca
```

### Difficulty and time of the world

Structure mobs depend on the world: their armor and enchantments get more likely with the difficulty and the age of the world. `World` reads them from the world; with the lower-level API, set them on the `FeatureEnv` of the dimension before decorating, from `level.dat` (`difficulty_settings`, `Time`) and the overworld clock in `data/minecraft/world_clocks.dat`:

```rust
use chunker_worldgen::entity::{Difficulty, SpawnSettings};

features.spawn = SpawnSettings {
    difficulty: Difficulty::Hard,
    clock_time: 1_920_000,
    game_time: 1_920_000,
    halloween: false,
};
```

The default is a new world on normal difficulty. A few values vanilla takes from an unseeded random of each entity (bonus attributes of zombies, the yaw of shulkers) can't match a particular vanilla run; they are drawn from a random seeded by the entity position.

### Other dimensions

The Nether and the End work the same way with their own settings and biome sources:

| Dimension | Noise settings | Biome source |
|---|---|---|
| Overworld | `minecraft:overworld` (also `amplified`, `large_biomes`) | `{"type": "minecraft:multi_noise", "preset": "minecraft:overworld"}` |
| Nether | `minecraft:nether` | `{"type": "minecraft:multi_noise", "preset": "minecraft:nether"}` |
| End | `minecraft:end` | `{"type": "minecraft:the_end"}` |

Use the build height of the dimension type: `-64` and `384` for the Overworld, `0` and `256` for the Nether and the End.

### Reading region files and NBT

[`examples/read_region.rs`](../../crates/chunker-worldgen/examples/read_region.rs) lists the chunks of a region file:

```rust
use chunker_anvil::{Region, RegionPos};

let region = Region::load(region_dir, RegionPos::new(x, z))?;
for index in 0..1024 {
    let Some(raw) = region.get(index) else { continue };
    let nbt = raw.decode_nbt()?;
    println!("{index}: {}", nbt.get_str("Status").unwrap_or("?"));
}
```

```bash
cargo run --release --example read_region -- target/terrain-example -1 -2
```

`chunker-nbt` reads and writes plain and compressed NBT: `read_root`, `read_compressed`, `write_root`, `write_compressed` (for example, `level.dat`).

### Structures and features

A decorated chunk needs its neighbours: it is decorated together with the 8 chunks around it, and structure starts come from chunks up to 8 away. Jobs take care of that. If you need the generators themselves, `world.dimension(&id)` returns the `Dimension` with all of them: `terrain` (`TerrainGenerator`), `structures` (`StructureGenerator`) and `features` (`FeatureEnv`, used by `feature::decorate`).

## What is not implemented

- `light` — the server relights chunks saved without light.
- Initial mob spawning (the `spawn` step) — the server runs it when the chunk first loads.
- Blending of old chunks, flat and debug world types.

## Performance

Measured on a Ryzen 9 9900X:

- terrain: about 5 ms per chunk on one thread, about 2100 chunks per second on 24 threads;
- terrain with structures and features: about 1500 chunks per second on 24 threads.

For comparison, the vanilla server and Chunky usually generate 100-300 chunks per second.

Single-thread benchmark with a breakdown by generation phase:

```bash
cargo run --release --example bench_terrain -- 500
```

## Development

### Setup

```bash
cargo xtask vanilla    # download the 26.3 server, extract its data, run the datagen reports, decompile it
cargo xtask oracle     # compile the Java oracle against the server jar
cargo test --release --workspace
```

`cargo xtask vanilla` puts everything into `vanilla/26.3/` (excluded from git): the server jar and libraries, the extracted `data/`, the datagen reports and the decompiled sources in `src/`.

> **Important:** Mojang code and data must never be committed.

Two more commands regenerate the tables embedded in `crates/chunker-data/data/<version>/`:

```bash
cargo xtask blocks     # blocks.txt from the datagen reports
cargo xtask extract    # the other tables, via the oracle
```

### Tests

[`tools/oracle`](../../tools/oracle) is a small Java program that boots the real server classes and answers queries: density samples, biomes, the terrain of a chunk, structure starts, the result of decorating a 3x3 region. The tests in `crates/chunker-worldgen/tests/` ask it and compare the results bit for bit: raw float bits, block state ids, heightmaps, block entities, ticks and entities.

| Test | What it checks |
|---|---|
| `oracle_density` | noise router functions, point and volume sampling, every noise settings preset |
| `oracle_biomes` | the Overworld parameter list against the datagen report, chunk biomes of four sources |
| `oracle_terrain` | noise fill, surface rules and carvers in the Overworld, the Nether and the End |
| `oracle_structures` | stronghold rings, every structure start with its pieces |
| `oracle_beardifier` | the beard kernel, terrain under structures that adapt it |
| `oracle_features` | feature order, decorated chunks in sample biomes, structure pieces, full decoration |

Entities are compared as complete NBT: both sides are loaded and saved again by the vanilla classes, so defaults don't matter, only what generation set. Values vanilla draws from an unseeded random of the entity are left out of the comparison.

Without `vanilla/` and the compiled oracle these tests are skipped, so `cargo test` always works.

Each oracle is a JVM that loads the whole server. At most `CHUNKER_ORACLES` (2 by default) run at the same time per test binary, each with `CHUNKER_ORACLE_XMX` (`2g` by default) of heap. If a JVM dies, the test fails with "oracle exited early"; its stderr is kept in `target/oracle-work/`.

### Additional checks

Slow and exploratory checks are marked `#[ignore]` and configured with environment variables:

```bash
# Structure pieces of the given structures
CHUNKER_STRUCTURES=minecraft:monument cargo test --release --test oracle_features structure_pieces_match_vanilla -- --ignored

# Everything applyBiomeDecoration does, structures included
CHUNKER_DIM=nether CHUNKER_STRUCTURES=1 cargo test --release --test oracle_features full_decoration_matches_vanilla -- --ignored

# One configured feature at a position
CHUNKER_PLACE=<feature>@x,y,z@seed CHUNKER_FEATURE_CHUNKS=cx,cz cargo test --release --test oracle_features single_feature -- --ignored
```

Other variables:

- `CHUNKER_DIM` — `overworld`, `nether` or `end`
- `CHUNKER_PIECE`, `CHUNKER_ALL_CHUNKS`, `CHUNKER_GRID_R` — narrow or widen `structure_pieces_match_vanilla`
- `CHUNKER_FEATURES` — features for `feature_hits`, which searches chunks where vanilla places them
- `CHUNKER_STRUCTURE_GRID`, `CHUNKER_STRUCTURE_SETS`, `CHUNKER_STRUCTURE_CHUNKS` — the search area of `oracle_structures`
- `CHUNKER_ORACLE_LOG=1` — a failing feature comparison prints the first differing `setBlock`
- `CHUNKER_SABOTAGE=1` — breaks our side on purpose in the beardifier and feature tests; they must fail then
- `CHUNKER_DIFFICULTY`, `CHUNKER_CLOCK`, `CHUNKER_GAME_TIME` — the world state for structure mobs (`hard` with a clock of `1920000` gives the most equipment and enchantments)
- `CHUNKER_ENTITY_LOG=<file>` — appends vanilla's entities of every compared chunk, to see what a run covered
- `CHUNKER_LOG_WINDOW=<n>` — prints `n` `setBlock` calls around the first difference

### Adding a generation step

1. Add an oracle command in `tools/oracle/src/chunker/oracle/` and a test first.
2. Port the vanilla code and change it until the test passes.
3. Break it on purpose (seed + 1, a skipped step) and make sure the test fails. A test that passes after that checks nothing.

### Bit-exactness rules

1. Density functions in 26.3 are `float`, noise coordinates are `double`. Keep every operand type and the evaluation order of the Java code: `int * float` converts the int, `float * double` widens the float. Do not use `mul_add`.
2. Do not use `f32::max/min/signum/round` from Rust. Use `math::{max_f32, min_f32, signum_f32, round_f32}`: they keep Java's behaviour for NaN, signed zero and rounding. The same goes for `floor_div` and `floor_mod`.
3. Integer arithmetic in Java wraps around. Use `wrapping_*` wherever an overflow is possible (hashes, seeds).
4. Point sampling (`sample_value`) and volume sampling (`sample_volume`) give different results in vanilla (`interpolated` accumulates steps in volume mode, `mul` skips the second argument on 0 only in point mode). Both are ported; always call the one vanilla calls.
5. Caches change results: a cache cell filled by a volume sample answers later point samples inside that volume. The order of sampling through one `SamplerContext` must be the same as in vanilla: aquifer construction, `doFill`, `buildSurface` (its rule compilation fills ore vein densities), carvers.
6. Gson keeps the decimal text of JSON numbers, so `Codec.FLOAT` is `Float.parseFloat(text)`, not `(float) Double.parseDouble(text)`. Use `codec::as_f32`.
7. The `Climate` R-tree search starts from the last result of the current thread (a `ThreadLocal` in vanilla), so ties depend on history. We use a thread local too.
8. Surface rule compilation order and the caching of lazy conditions follow `MaterialRuleContext` exactly.
9. `Math.sin` and `Math.cos` are HotSpot intrinsics. `math::trig` is correctly rounded, which matches Java except for about 0.15% of inputs. `Math.pow` and `Math.log` may differ from the platform libm by one ulp; they are used in only a few places.
10. In feature code, keep Java's comparisons for NaN (`!(d < 0.0)` is not `d >= 0.0`), iterate `HashSet`s through `java::JavaHashSet`, keep sorts stable, and check decompiled loops against the bytecode.
11. Vanilla changes shared structure pieces in `postProcess` (template positions, ground heights of scattered pieces, chest and spawner flags). The first chunk that places a piece decides, which is mirrored with interior mutability.
12. `postProcess` order: structures by generation step and registry order, starts of one structure in the order of the chunk's reference `LongOpenHashSet`. All starts of a chunk share one writable box.

The decompiled sources are not always exact. If a piece of code looks strange, check the bytecode:

```bash
javap -c -p -cp vanilla/26.3/server-26.3.jar net.minecraft.world.level.levelgen.SomeClass
```

### Adding a Minecraft version

`chunker_data::McVersion` lists the supported versions. Each version has its own tables in `crates/chunker-data/data/<version>/` and, where vanilla behaviour differs, its own branches in the generation code.

1. Run `cargo xtask vanilla --version <v>` and `cargo xtask extract --version <v>`.
2. Add the `McVersion` variant.
3. Compare the decompiled `levelgen` packages with the previous version and port the differences.
4. Extend the tests to the new version.

### Code style

- `cargo fmt` and `cargo clippy --workspace --all-targets` must pass without warnings (checked by CI).
- Doc comments name the vanilla original (``/// `Aquifer.computeSubstance` ``), so the source is easy to find.
- Comments explain why the code follows a vanilla quirk, not what it does.
- No `unwrap()` on data pack input: return `anyhow` errors with context.

For hot code: do not clone an `Arc` per block or sample, resolve registry lookups once per world, take buffers from the `SamplerContext` pool instead of allocating per chunk.

## Data sources

### Server jar and data packs

Worldgen JSON (noise settings, density functions, biomes, structures, features, tags) and structure templates are read at runtime from the server jar. Data packs are added on top in the order of `DataPacks.Enabled` from `level.dat`:

- `vanilla` — the root of the jar;
- `file/<name>` — `<world>/datapacks/<name>` (a folder or a zip);
- any other name — a built-in feature pack in the jar (`data/minecraft/datapacks/<name>/`).

Later packs override earlier ones; tags are merged unless they set `replace`.

### Embedded tables

Some data is defined in Java code, not in the data pack. It is extracted once per version and embedded from `crates/chunker-data/data/<version>/`:

- `blocks.txt` — blocks, properties and states in registry order;
- `block_flags.txt` — properties of each state: air, fluid, solid, replaceable, light, sturdy faces, collision, waterlogging and others;
- `block_colors.txt` — map colour of each state;
- `block_classes.txt` — the Java class chain of each block;
- `block_transforms.txt` — the result of `rotate` and `mirror` for each state;
- `block_shapes.txt` — wall connection tests;
- `block_entities.nbt` — a new block entity of each block, before and after loading;
- `entity_attributes.txt` — default attribute values of each entity type;
- `item_enchantable.txt` — the `enchantable` value of each item.

The Overworld and Nether parameter lists are built in code by vanilla too. They are ported as Rust (`biome/overworld_builder.rs`) and checked against the datagen report.

## World format notes

- Dimensions are stored in `<world>/dimensions/<namespace>/<path>/region`, the Overworld included.
- World generation settings are in `<world>/data/minecraft/world_gen_settings.dat`. Older worlds kept them in `level.dat`.
- Chunk statuses: `empty`, `structure_starts`, `structure_references`, `biomes`, `terrain`, `features`, `initialize_light`, `light`, `spawn`, `full`. The `terrain` step replaced `noise`, `surface` and `carvers`.
- A default block state in a palette is written as the bare block id, any other state as `{"id": ..., "properties": {...}}`. A palette that mixes both wraps the strings as `{"": "minecraft:stone"}`.
- Chunks of 256 sectors or more are written to `c.<x>.<z>.mcc` next to the region file.

## Project structure

```
├── crates/
│   ├── chunker-worldgen/      — world generation
│   │   ├── examples/          — pregenerate, terrain, read_region, bench_terrain
│   │   └── tests/             — verification tests against vanilla
│   ├── chunker-data/          — data packs, tags, block registry, server jar lookup
│   │   └── data/<version>/    — embedded tables
│   ├── chunker-anvil/         — region files
│   └── chunker-nbt/           — NBT
├── tools/oracle/              — Java oracle running the real server classes
├── xtask/                     — vanilla, blocks, oracle and extract commands
└── vanilla/<version>/         — server, extracted data and decompiled sources (not in git)
```

Modules of `chunker-worldgen` and the vanilla classes they port:

```
src/
├── world/             — World: opening a server or world folder, data packs, dimensions, job control
├── job/               — pre-generation jobs: region order, terrain and decoration phases, region writing, resume
├── preview.rs         — map previews: vanilla map colours, biome maps, PNG tiles
├── math/              — Mth, java.lang.Math semantics, Interval, correctly rounded sin/cos
├── java.rs            — java.util.HashSet iteration order
├── random/            — LegacyRandomSource, XoroshiroRandomSource, WorldgenRandom, RandomSupport
├── noise/             — Perlin, simplex, normal and blended noise
├── density/           — density functions: parser, DensityFunctionCompiler, samplers, CubicSpline
├── random_state.rs    — RandomState
├── settings.rs        — NoiseGeneratorSettings, NoiseRouter
├── biome/             — Climate, biome sources, OverworldBiomeBuilder, BiomeManager
├── chunk/             — ProtoChunk, LevelChunkSection, Heightmap, chunk serialization
├── terrain/           — NoiseChunk, Aquifer, surface rules, carvers, Beardifier, buildTerrain
├── providers.rs       — IntProvider, FloatProvider, HeightProvider, WeightedList
├── block_set.rs       — block tags as sets of states
├── structure/         — placement, starts, pieces, jigsaw, templates, processors, every structure type
├── entity/            — Mob.finalizeSpawn of structure mobs, spawn variants, enchanting of spawn equipment
└── feature/           — WorldGenRegion, placed and configured features, block behaviour, every feature type, trees
```

## License

MIT or Apache-2.0, at your option. See [LICENSE-MIT](../../LICENSE-MIT) and [LICENSE-APACHE](../../LICENSE-APACHE).

Not an official Minecraft product. Not approved by or associated with Mojang or Microsoft.
