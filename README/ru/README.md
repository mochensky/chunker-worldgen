# chunker-worldgen

Побитово точная реимплементация генерации мира Minecraft: Java Edition на Rust. Для заданного сида, датапаков и измерения генерирует те же чанки, что и ванильный сервер, блок в блок, и записывает их в ванильном формате.

Поддерживаемая версия: **Minecraft 26.3** (data version 5023). Код рассчитан на поддержку других версий.

## Возможности

- **Биомы** — multi-noise источники с пресетами Верхнего мира и Незера, Энд, фиксированный и шахматный источники.
- **Рельеф** — noise fill, водоносные слои (aquifers), правила поверхности, рудные жилы, пещеры и каньоны, карты высот.
- **Структуры** — стартовые точки и ссылки всех типов структур: jigsaw-структуры (деревни, древние города, испытательные камеры, бастионы, ...), крепости Края с кольцами, шахты, крепости Незера, океанские монументы, особняки, города Края и остальные.
- **Подстройка рельефа** — beardifier: рельеф под деревнями, древними городами и другими структурами формируется как в ванилле.
- **Фичи** — весь шаг `features`: все размещаемые фичи (руды, деревья, озёра, жеоды, ...), части структур с процессорами, блок-сущности с таблицами лута, запланированные тики.
- **Мобы структур** — жители, железные големы, кошки и животные в деревнях, пиглины в бастионах, утопленники в подводных руинах, иллагеры в особняках и остальные, с ванильной донастройкой (`Mob.finalizeSpawn`): варианты по биому, типы жителей, снаряжение, зачарования, куриные жокеи и наездники на zombie nautilus.
- **Ванильные данные во время работы** — worldgen JSON и шаблоны структур читаются из официального jar сервера вместе с датапаками самого мира.
- **Ванильные форматы** — NBT чанков, region-файлы Anvil (`.mca` / `.mcc`), чтение и запись NBT.
- **Проверка по ванилле** — тесты запускают настоящие классы сервера и сравнивают результаты побитово.
- **Предгенерация** — `World` открывает папку сервера или мира и параллельно генерирует область с прогрессом, паузой, продолжением после перезапуска и превью карты.
- **Скорость** — около 5 мс на чанк рельефа в одном потоке, около 1500 чанков в секунду со структурами и фичами на 24 потоках.

## Требования

- Rust 1.90+
- Доступ в интернет при первом запуске (чтобы скачать jar сервера у Mojang) или jar сервера на диске
- Java 25 (только для запуска проверочных тестов)

## Установка

### Вариант 1: crates.io (рекомендуется)

```toml
[dependencies]
chunker-worldgen = "0.2"
```

Остальные крейты подтянутся как его зависимости; добавляйте их сами, только если используете напрямую:

```toml
chunker-data = "0.2"
chunker-anvil = "0.2"
chunker-nbt = "0.2"
```

Что делает каждый крейт:

- `chunker-worldgen` — генерация мира
- `chunker-data` — идентификаторы, датапаки, теги, реестр блоков, поиск и загрузка jar сервера
- `chunker-anvil` — region-файлы и сжатие чанков
- `chunker-nbt` — NBT

Генерация сильно нагружает процессор, поэтому собирайте в release. Чтобы debug-сборки тоже были пригодны, включите оптимизацию этих крейтов в dev-профиле:

```toml
[profile.dev.package.chunker-worldgen]
opt-level = 3
```

### Вариант 2: Git или локальная копия

Самый свежий код из репозитория:

```toml
[dependencies]
chunker-worldgen = { git = "https://github.com/mochensky/chunker-worldgen" }
```

Или локальная копия:

```bash
git clone https://github.com/mochensky/chunker-worldgen
```

```toml
[dependencies]
chunker-worldgen = { path = "../chunker-worldgen/crates/chunker-worldgen" }
chunker-data = { path = "../chunker-worldgen/crates/chunker-data" }
```

> **Важно:** `World` и задания — основная точка входа; низкоуровневые модули открывают много внутренностей и могут измениться до первого релиза.

## Предгенерация мира

`World` открывает папку сервера или мира и генерирует его область целиком, как ванилла: рельеф, структуры, фичи и мобов структур. Чанки записываются в region-файлы мира со статусом `features`; свет и спавн животных сервер добавит сам при их загрузке.

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

> **Важно:** сначала остановите сервер. Задание берёт `session.lock` так же, как сервер, и не запустится, если мир занят.

Что делает задание:

- **Порядок** — сначала регионы ближе к центру; рельеф параллельно, затем шаг `features` в 9 фаз, чтобы соседние чанки никогда не декорировались одновременно.
- **Существующие чанки** — никогда не перезаписываются. С `skip_existing` они пропускаются и служат соседям источником данных для чтения.
- **Пауза и продолжение** — `world.pause()`, `world.resume()`, `world.cancel()`. Прогресс хранится в `<мир>/.chunker/job.json`: `World::open` находит незаконченное задание, а `resume()` продолжает его после перезапуска.
- **Только рельеф** — с `decorate: false` чанки сохраняются со статусом `terrain`, это примерно на треть быстрее; структуры и фичи сервер расставит при первой загрузке чанков.
- **Сложность и время** — читаются из мира (`level.dat` и `data/minecraft/world_clocks.dat`); от них зависит снаряжение мобов структур.
- **Превью карты** — `world.previews(&dimension)` хранит картинку карты 128x128 на регион (ванильные цвета карты и биомы) и обновляет её во время работы задания. В Незере карта показывает пол под бедроковым потолком.
- **Оценка размера** — `world.estimate_chunk_bytes(&spec)` генерирует пробные чанки области так же, как задание, и возвращает размер чанка на диске в байтах; оценка учитывает измерение, датапаки и версию.

Поля `JobSpec`:

| Поле | Значение |
|---|---|
| `dimension` | id измерения, например `minecraft:overworld`, `minecraft:the_nether` |
| `center_x`, `center_z` | центр в координатах блоков |
| `radius` | радиус в блоках (у прямоугольника — половина ширины по X) |
| `radius_z` | половина длины по Z у `Rectangle`; `None` для остальных форм |
| `shape` | `Square`, `Circle` или `Rectangle`; чанк входит в область, если в неё попадает его центр |
| `skip_existing` | не трогать уже существующие чанки |
| `threads` | число рабочих потоков, `0` — все ядра |
| `decorate` | выполнять шаг `features` (структуры, фичи, мобы) |

Полная программа — [`examples/pregenerate.rs`](../../crates/chunker-worldgen/examples/pregenerate.rs):

```bash
cargo run --release --example pregenerate -- <папка сервера или мира> 512
```

## Низкоуровневый API

Части, из которых собрано задание, можно использовать напрямую. Шаги ниже генерируют рельеф отдельных чанков в памяти; полная программа — [`examples/terrain.rs`](../../crates/chunker-worldgen/examples/terrain.rs).

### Шаг 1: Загрузите ванильные данные

`vanilla::locate` ищет jar сервера: явно указанный путь, папку сервера (`versions/<v>/server-<v>.jar`, `cache/mojang_<v>.jar` у Paper, любой jar в папке), папку кэша. Если ничего не найдено и загрузка разрешена, он скачивает jar у Mojang и проверяет его SHA-1.

```rust
use std::sync::Arc;
use chunker_data::{DataPacks, McVersion, PackSource, blocks::BlockRegistry, vanilla};
use chunker_worldgen::registry::WorldgenRegistries;

let version = McVersion::V26_3;
let jar = vanilla::locate(version, None, Some(server_dir), cache_dir, true)?;

let mut packs = DataPacks::new();
packs.push(jar.vanilla_pack()?);
// Датапаки мира добавляются сверху, в порядке из level.dat
packs.push(PackSource::directory(world_dir.join("datapacks/my_pack")));

let registries = Arc::new(WorldgenRegistries::new(packs, BlockRegistry::for_version(version)));
```

### Шаг 2: Создайте генератор

Генератору нужны настройки шума и источник биомов измерения, те же, что в `world_gen_settings.dat` мира.

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

Генератор можно использовать из нескольких потоков одновременно, поэтому чанки можно генерировать параллельно.

### Шаг 3: Сгенерируйте чанк

Правила поверхности и карверы читают биомы соседних чанков, поэтому сначала вычисляются биомы чанков 3x3 вокруг нужного.

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

В `beardifier` передаются стартовые точки структур вокруг чанка; без них рельеф генерируется так, будто структур нет.

### Шаг 4: Сохраните чанк

```rust
use chunker_anvil::{Compression, Region, RegionPos};
use chunker_worldgen::chunk::serialize::write_chunk;

let mut region = Region::empty(RegionPos::of_chunk(cx, cz));
let nbt = write_chunk(&chunk, &biomes, version.data_version(), 0);
let index = (cx & 31) as usize + (cz & 31) as usize * 32;
region.set_nbt(index, &nbt, Compression::Zlib, chunker_anvil::now_timestamp());
region.save(&world_dir.join("dimensions/minecraft/overworld/region"))?;
```

Сервер, открывший мир, продолжает генерацию сохранённых чанков с их статуса: здесь с `terrain`, поэтому фичи, свет и мобов он добавит сам. Структур в таких чанках не будет: стартовые точки структур относятся к более раннему шагу, см. [Структуры и фичи](#структуры-и-фичи).

### Шаг 5: Запустите пример

```bash
cargo run --release --example terrain -- <сид> <чанк x> <чанк z> <папка вывода>
```

Все аргументы необязательные. Со значениями по умолчанию вы увидите:

```
chunk  -12  -56: surface at y=70   minecraft:grass_block[snowy=false]
chunk  -11  -56: surface at y=62   minecraft:water[level=0]
...
wrote target/terrain-example/r.-1.-2.mca
```

### Сложность и время мира

Мобы структур зависят от мира: броня и зачарования выпадают чаще с ростом сложности и возраста мира. `World` читает их из мира сам; в низкоуровневом API задайте их в `FeatureEnv` измерения перед декорированием из `level.dat` (`difficulty_settings`, `Time`) и часов Верхнего мира в `data/minecraft/world_clocks.dat`:

```rust
use chunker_worldgen::entity::{Difficulty, SpawnSettings};

features.spawn = SpawnSettings {
    difficulty: Difficulty::Hard,
    clock_time: 1_920_000,
    game_time: 1_920_000,
    halloween: false,
};
```

По умолчанию это новый мир на нормальной сложности. Несколько значений ванилла берёт из незасеянного генератора самой сущности (бонусные атрибуты зомби, поворот шалкеров); с конкретным запуском ванилы они совпасть не могут и берутся из генератора, засеянного позицией сущности.

### Другие измерения

Незер и Энд работают так же, со своими настройками и источниками биомов:

| Измерение | Настройки шума | Источник биомов |
|---|---|---|
| Верхний мир | `minecraft:overworld` (а также `amplified`, `large_biomes`) | `{"type": "minecraft:multi_noise", "preset": "minecraft:overworld"}` |
| Незер | `minecraft:nether` | `{"type": "minecraft:multi_noise", "preset": "minecraft:nether"}` |
| Энд | `minecraft:end` | `{"type": "minecraft:the_end"}` |

Используйте высоту мира из типа измерения: `-64` и `384` для Верхнего мира, `0` и `256` для Незера и Энда.

### Чтение region-файлов и NBT

[`examples/read_region.rs`](../../crates/chunker-worldgen/examples/read_region.rs) выводит список чанков region-файла:

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

`chunker-nbt` читает и пишет обычный и сжатый NBT: `read_root`, `read_compressed`, `write_root`, `write_compressed` (например, `level.dat`).

### Структуры и фичи

Декорированному чанку нужны соседи: он декорируется вместе с 8 чанками вокруг, а стартовые точки структур берутся из чанков на расстоянии до 8. Задания делают это сами. Если нужны сами генераторы, `world.dimension(&id)` возвращает `Dimension` со всеми ними: `terrain` (`TerrainGenerator`), `structures` (`StructureGenerator`) и `features` (`FeatureEnv`, его использует `feature::decorate`).

## Что не реализовано

- `light` — сервер пересчитывает свет чанков, сохранённых без него.
- Начальный спавн мобов (шаг `spawn`) — сервер выполняет его при первой загрузке чанка.
- Смешивание со старыми чанками, плоский и отладочный типы мира.

## Производительность

Замеры на Ryzen 9 9900X:

- рельеф: около 5 мс на чанк в одном потоке, около 2100 чанков в секунду на 24 потоках;
- рельеф со структурами и фичами: около 1500 чанков в секунду на 24 потоках.

Для сравнения, ванильный сервер и Chunky обычно генерируют 100-300 чанков в секунду.

Однопоточный бенчмарк с разбивкой по этапам генерации:

```bash
cargo run --release --example bench_terrain -- 500
```

## Разработка

### Подготовка

```bash
cargo xtask vanilla    # скачать сервер 26.3, извлечь данные, запустить отчёты datagen, декомпилировать
cargo xtask oracle     # скомпилировать Java-оракул против jar сервера
cargo test --release --workspace
```

`cargo xtask vanilla` складывает всё в `vanilla/26.3/` (исключено из git): jar сервера и библиотеки, извлечённую папку `data/`, отчёты datagen и декомпилированные исходники в `src/`.

> **Важно:** код и данные Mojang нельзя коммитить.

Ещё две команды пересоздают таблицы, встроенные в `crates/chunker-data/data/<версия>/`:

```bash
cargo xtask blocks     # blocks.txt из отчётов datagen
cargo xtask extract    # остальные таблицы, через оракул
```

### Тесты

[`tools/oracle`](../../tools/oracle) — небольшая программа на Java, которая запускает настоящие классы сервера и отвечает на запросы: значения функций плотности, биомы, рельеф чанка, стартовые точки структур, результат декорирования области 3x3. Тесты в `crates/chunker-worldgen/tests/` обращаются к нему и сравнивают результаты побитово: биты float, id состояний блоков, карты высот, блок-сущности, тики и сущности.

| Тест | Что проверяет |
|---|---|
| `oracle_density` | функции noise router, точечную и объёмную выборку, все пресеты настроек шума |
| `oracle_biomes` | список параметров Верхнего мира по отчёту datagen, биомы чанков для четырёх источников |
| `oracle_terrain` | noise fill, правила поверхности и карверы в Верхнем мире, Незере и Энде |
| `oracle_structures` | кольца крепостей Края, все стартовые точки структур с их частями |
| `oracle_beardifier` | ядро beardifier, рельеф под структурами, которые его подстраивают |
| `oracle_features` | порядок фич, декорированные чанки в разных биомах, части структур, полное декорирование |

Сущности сравниваются как полный NBT: обе стороны загружаются и снова сохраняются ванильными классами, поэтому значения по умолчанию не важны, важно только то, что задала генерация. Значения, которые ванилла берёт из незасеянного генератора сущности, в сравнение не входят.

Без `vanilla/` и скомпилированного оракула эти тесты пропускаются, поэтому `cargo test` работает всегда.

Каждый оракул — это JVM, которая загружает весь сервер. Одновременно запускается не больше `CHUNKER_ORACLES` (по умолчанию 2) на тестовый бинарник, каждый с кучей `CHUNKER_ORACLE_XMX` (по умолчанию `2g`). Если JVM падает, тест завершается с ошибкой "oracle exited early"; её stderr сохраняется в `target/oracle-work/`.

### Дополнительные проверки

Медленные и исследовательские проверки помечены `#[ignore]` и настраиваются переменными окружения:

```bash
# Части заданных структур
CHUNKER_STRUCTURES=minecraft:monument cargo test --release --test oracle_features structure_pieces_match_vanilla -- --ignored

# Всё, что делает applyBiomeDecoration, вместе со структурами
CHUNKER_DIM=nether CHUNKER_STRUCTURES=1 cargo test --release --test oracle_features full_decoration_matches_vanilla -- --ignored

# Одна фича в заданной точке
CHUNKER_PLACE=<фича>@x,y,z@seed CHUNKER_FEATURE_CHUNKS=cx,cz cargo test --release --test oracle_features single_feature -- --ignored
```

Другие переменные:

- `CHUNKER_DIM` — `overworld`, `nether` или `end`
- `CHUNKER_PIECE`, `CHUNKER_ALL_CHUNKS`, `CHUNKER_GRID_R` — сужают или расширяют `structure_pieces_match_vanilla`
- `CHUNKER_FEATURES` — фичи для `feature_hits`, который ищет чанки, где ванилла их размещает
- `CHUNKER_STRUCTURE_GRID`, `CHUNKER_STRUCTURE_SETS`, `CHUNKER_STRUCTURE_CHUNKS` — область поиска `oracle_structures`
- `CHUNKER_ORACLE_LOG=1` — при расхождении фич выводит первый отличающийся `setBlock`
- `CHUNKER_SABOTAGE=1` — намеренно ломает нашу сторону в тестах beardifier и фич; тогда они должны падать
- `CHUNKER_DIFFICULTY`, `CHUNKER_CLOCK`, `CHUNKER_GAME_TIME` — состояние мира для мобов структур (`hard` и часы `1920000` дают больше всего снаряжения и зачарований)
- `CHUNKER_ENTITY_LOG=<файл>` — дописывает ванильные сущности каждого сравнённого чанка, чтобы видеть, что покрыл прогон
- `CHUNKER_LOG_WINDOW=<n>` — выводит `n` вызовов `setBlock` вокруг первого расхождения

### Добавление шага генерации

1. Сначала добавьте команду оракула в `tools/oracle/src/chunker/oracle/` и тест.
2. Перенесите ванильный код и меняйте его, пока тест не пройдёт.
3. Намеренно сломайте его (сид + 1, пропущенный шаг) и убедитесь, что тест падает. Тест, который после этого проходит, ничего не проверяет.

### Правила побитовой точности

1. Функции плотности в 26.3 работают с `float`, координаты шума — с `double`. Сохраняйте типы операндов и порядок вычислений Java-кода: `int * float` приводит int, `float * double` расширяет float. Не используйте `mul_add`.
2. Не используйте `f32::max/min/signum/round` из Rust. Используйте `math::{max_f32, min_f32, signum_f32, round_f32}`: они повторяют поведение Java для NaN, знакового нуля и округления. То же для `floor_div` и `floor_mod`.
3. Целочисленная арифметика в Java переполняется по кругу. Используйте `wrapping_*` везде, где возможно переполнение (хэши, сиды).
4. Точечная выборка (`sample_value`) и объёмная (`sample_volume`) в ванилле дают разные результаты (`interpolated` накапливает шаги в объёмном режиме, `mul` пропускает второй аргумент при 0 только в точечном). Перенесены обе; всегда вызывайте ту, что вызывает ванилла.
5. Кэши меняют результаты: ячейка кэша, заполненная объёмной выборкой, отвечает на последующие точечные выборки внутри этого объёма. Порядок выборок через один `SamplerContext` должен совпадать с ванильным: построение aquifer, `doFill`, `buildSurface` (компиляция его правил заполняет плотности рудных жил), карверы.
6. Gson сохраняет десятичный текст чисел JSON, поэтому `Codec.FLOAT` — это `Float.parseFloat(text)`, а не `(float) Double.parseDouble(text)`. Используйте `codec::as_f32`.
7. Поиск в R-дереве `Climate` начинается с последнего результата текущего потока (`ThreadLocal` в ванилле), поэтому при равенстве результат зависит от истории. У нас тоже thread local.
8. Порядок компиляции правил поверхности и кэширование ленивых условий в точности следуют `MaterialRuleContext`.
9. `Math.sin` и `Math.cos` — интринсики HotSpot. `math::trig` округляет корректно, что совпадает с Java кроме примерно 0,15% входов. `Math.pow` и `Math.log` могут отличаться от системной libm на один ulp; они используются всего в нескольких местах.
10. В коде фич сохраняйте сравнения Java для NaN (`!(d < 0.0)` — не то же самое, что `d >= 0.0`), обходите `HashSet` через `java::JavaHashSet`, сохраняйте стабильность сортировок и сверяйте декомпилированные циклы с байткодом.
11. Ванилла меняет общие части структур в `postProcess` (позиции шаблонов, высоту земли у scattered-структур, флаги сундуков и спавнеров). Решает первый чанк, который размещает часть; это повторено через внутреннюю изменяемость.
12. Порядок `postProcess`: структуры по шагу генерации и порядку в реестре, стартовые точки одной структуры — в порядке `LongOpenHashSet` ссылок чанка. Все стартовые точки чанка используют одну общую область записи.

Декомпилированные исходники не всегда точны. Если код выглядит странно, проверьте байткод:

```bash
javap -c -p -cp vanilla/26.3/server-26.3.jar net.minecraft.world.level.levelgen.SomeClass
```

### Добавление версии Minecraft

`chunker_data::McVersion` перечисляет поддерживаемые версии. У каждой версии свои таблицы в `crates/chunker-data/data/<версия>/` и, где поведение ваниллы отличается, свои ветки в коде генерации.

1. Выполните `cargo xtask vanilla --version <v>` и `cargo xtask extract --version <v>`.
2. Добавьте вариант в `McVersion`.
3. Сравните декомпилированные пакеты `levelgen` с предыдущей версией и перенесите различия.
4. Расширьте тесты на новую версию.

### Стиль кода

- `cargo fmt` и `cargo clippy --workspace --all-targets` должны проходить без предупреждений (проверяется в CI).
- Doc-комментарии называют ванильный оригинал (``/// `Aquifer.computeSubstance` ``), чтобы исходник было легко найти.
- Комментарии объясняют, почему код повторяет особенность ваниллы, а не что он делает.
- Никаких `unwrap()` на данных датапаков: возвращайте ошибки `anyhow` с контекстом.

Для горячего кода: не клонируйте `Arc` на каждый блок или выборку, разрешайте обращения к реестрам один раз на мир, берите буферы из пула `SamplerContext` вместо выделения памяти на каждый чанк.

## Источники данных

### Jar сервера и датапаки

Worldgen JSON (настройки шума, функции плотности, биомы, структуры, фичи, теги) и шаблоны структур читаются во время работы из jar сервера. Датапаки добавляются сверху в порядке `DataPacks.Enabled` из `level.dat`:

- `vanilla` — корень jar;
- `file/<имя>` — `<мир>/datapacks/<имя>` (папка или zip);
- любое другое имя — встроенный feature pack в jar (`data/minecraft/datapacks/<имя>/`).

Более поздние датапаки переопределяют ранние; теги объединяются, если в них не указан `replace`.

### Встроенные таблицы

Часть данных задаётся в Java-коде, а не в датапаке. Они извлекаются один раз для каждой версии и встраиваются из `crates/chunker-data/data/<версия>/`:

- `blocks.txt` — блоки, свойства и состояния в порядке реестра;
- `block_flags.txt` — свойства каждого состояния: воздух, жидкость, твёрдость, заменяемость, свет, прочные грани, коллизия, waterlogging и другие;
- `block_colors.txt` — цвет каждого состояния на карте;
- `block_classes.txt` — цепочка Java-классов каждого блока;
- `block_transforms.txt` — результат `rotate` и `mirror` для каждого состояния;
- `block_shapes.txt` — проверки соединения стен;
- `block_entities.nbt` — новая блок-сущность каждого блока до и после загрузки;
- `entity_attributes.txt` — значения атрибутов по умолчанию для каждого типа сущности;
- `item_enchantable.txt` — значение `enchantable` каждого предмета.

Списки параметров Верхнего мира и Незера ванилла тоже строит в коде. Они перенесены на Rust (`biome/overworld_builder.rs`) и проверяются по отчёту datagen.

## Особенности формата мира

- Измерения хранятся в `<мир>/dimensions/<namespace>/<path>/region`, включая Верхний мир.
- Настройки генерации мира лежат в `<мир>/data/minecraft/world_gen_settings.dat`. В старых мирах они были в `level.dat`.
- Статусы чанков: `empty`, `structure_starts`, `structure_references`, `biomes`, `terrain`, `features`, `initialize_light`, `light`, `spawn`, `full`. Шаг `terrain` заменил `noise`, `surface` и `carvers`.
- Состояние блока по умолчанию записывается в палитру просто id блока, любое другое — как `{"id": ..., "properties": {...}}`. Палитра, где есть и то и другое, оборачивает строки в `{"": "minecraft:stone"}`.
- Чанки размером от 256 секторов записываются в `c.<x>.<z>.mcc` рядом с region-файлом.

## Структура проекта

```
├── crates/
│   ├── chunker-worldgen/      — генерация мира
│   │   ├── examples/          — pregenerate, terrain, read_region, bench_terrain
│   │   └── tests/             — проверочные тесты по ванилле
│   ├── chunker-data/          — датапаки, теги, реестр блоков, поиск jar сервера
│   │   └── data/<версия>/     — встроенные таблицы
│   ├── chunker-anvil/         — region-файлы
│   └── chunker-nbt/           — NBT
├── tools/oracle/              — Java-оракул, запускающий настоящие классы сервера
├── xtask/                     — команды vanilla, blocks, oracle и extract
└── vanilla/<версия>/          — сервер, извлечённые данные и декомпилированные исходники (не в git)
```

Модули `chunker-worldgen` и ванильные классы, которые они переносят:

```
src/
├── world/             — World: открытие папки сервера или мира, датапаки, измерения, управление заданиями
├── job/               — задания предгенерации: порядок регионов, фазы рельефа и декорирования, запись регионов, продолжение
├── preview.rs         — превью карты: ванильные цвета карты, карты биомов, PNG-тайлы
├── math/              — Mth, семантика java.lang.Math, Interval, корректно округлённые sin/cos
├── java.rs            — порядок обхода java.util.HashSet
├── random/            — LegacyRandomSource, XoroshiroRandomSource, WorldgenRandom, RandomSupport
├── noise/             — шумы Перлина, симплекс, normal и blended
├── density/           — функции плотности: парсер, DensityFunctionCompiler, сэмплеры, CubicSpline
├── random_state.rs    — RandomState
├── settings.rs        — NoiseGeneratorSettings, NoiseRouter
├── biome/             — Climate, источники биомов, OverworldBiomeBuilder, BiomeManager
├── chunk/             — ProtoChunk, LevelChunkSection, Heightmap, сериализация чанка
├── terrain/           — NoiseChunk, Aquifer, правила поверхности, карверы, Beardifier, buildTerrain
├── providers.rs       — IntProvider, FloatProvider, HeightProvider, WeightedList
├── block_set.rs       — теги блоков как множества состояний
├── structure/         — размещение, стартовые точки, части, jigsaw, шаблоны, процессоры, все типы структур
├── entity/            — Mob.finalizeSpawn мобов структур, варианты при спавне, зачарование снаряжения
└── feature/           — WorldGenRegion, размещаемые и настраиваемые фичи, поведение блоков, все типы фич, деревья
```

## Лицензия

MIT или Apache-2.0, на ваш выбор. См. [LICENSE-MIT](../../LICENSE-MIT) и [LICENSE-APACHE](../../LICENSE-APACHE).

Не является официальным продуктом Minecraft. Не одобрено и не связано с Mojang или Microsoft.
