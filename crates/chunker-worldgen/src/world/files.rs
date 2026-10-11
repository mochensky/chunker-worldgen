//! WorldFiles discovery: paths, `level.dat`, world gen settings, data packs, dimensions, `session.lock`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::entity::{Difficulty, SpawnSettings};
use anyhow::{Context, Result, bail};
use chunker_data::vanilla::ServerJar;
use chunker_data::{DataPacks, Identifier, McVersion, PackSource};
use chunker_nbt::{Compound, Tag};
use serde_json::Value;

/// An opened world folder (held locked while generating).
pub struct WorldFiles {
    pub dir: PathBuf,
    pub server_dir: Option<PathBuf>,
    pub level_name: String,
    pub data_version: i32,
    pub version: McVersion,
    pub seed: i64,
    pub generate_structures: bool,
    pub enabled_packs: Vec<String>,
    /// Raw `dimensions` map from the world gen settings (NBT converted to JSON).
    pub dimensions: Vec<(Identifier, Value)>,
    /// `region-file-compression` from server.properties (informational; chunker writes zlib).
    pub region_compression: Option<String>,
    /// Difficulty and time of the world, for `finalizeSpawn` of structure mobs.
    pub spawn: SpawnSettings,
    lock: Option<fs::File>,
}

/// `level.dat` difficulty and game time, and the overworld clock (`data/minecraft/world_clocks.dat`).
fn spawn_settings(dir: &Path, data: &Compound) -> SpawnSettings {
    let difficulty = data
        .get_compound("difficulty_settings")
        .and_then(|d| d.get_str("difficulty"))
        .and_then(Difficulty::by_name)
        .or_else(|| data.get_i32("Difficulty").and_then(Difficulty::by_id))
        .unwrap_or(Difficulty::Normal);
    let clocks = fs::File::open(dir.join("data/minecraft/world_clocks.dat"))
        .ok()
        .and_then(|f| chunker_nbt::read_compressed(f).ok());
    let clock_time = clocks
        .as_ref()
        .and_then(|c| {
            c.get_compound("data")?
                .get_compound("minecraft:overworld")?
                .get_i64("total_ticks")
        })
        .or_else(|| data.get_i64("DayTime"))
        .unwrap_or(0);
    SpawnSettings {
        difficulty,
        clock_time,
        game_time: data.get_i64("Time").unwrap_or(0),
        halloween: is_halloween(),
    }
}

/// `SpecialDates.isHalloween()`: October 31 (UTC here; vanilla uses the server's time zone).
fn is_halloween() -> bool {
    let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) else {
        return false;
    };
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = (now.as_secs() / 86400) as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    month == 10 && day == 31
}

/// Removes the Windows `\\?\` verbatim prefix added by `canonicalize`, for readable paths.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest.to_owned()),
        _ => p,
    }
}

/// Converts NBT (as written by `JsonOps`-compatible codecs) into JSON for the shared parsers.
pub fn nbt_to_json(tag: &Tag) -> Value {
    match tag {
        Tag::Byte(v) => Value::from(*v as i64),
        Tag::Short(v) => Value::from(*v as i64),
        Tag::Int(v) => Value::from(*v as i64),
        Tag::Long(v) => Value::from(*v),
        Tag::Float(v) => serde_json::Number::from_f64(*v as f64)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Tag::Double(v) => serde_json::Number::from_f64(*v)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Tag::String(s) => Value::String(s.clone()),
        Tag::ByteArray(a) => Value::Array(a.iter().map(|v| Value::from(*v as i64)).collect()),
        Tag::IntArray(a) => Value::Array(a.iter().map(|v| Value::from(*v as i64)).collect()),
        Tag::LongArray(a) => Value::Array(a.iter().map(|v| Value::from(*v)).collect()),
        Tag::List(l) => Value::Array(l.iter().map(nbt_to_json).collect()),
        Tag::Compound(c) => Value::Object(c.iter().map(|(k, v)| (k.to_owned(), nbt_to_json(v))).collect()),
    }
}

fn read_properties(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
        .collect()
}

impl WorldFiles {
    /// Opens a world from either a server directory (with `server.properties`) or a world directory.
    pub fn open(path: &Path) -> Result<WorldFiles> {
        let path = strip_verbatim(
            fs::canonicalize(path).with_context(|| format!("{} does not exist", path.display()))?,
        );
        let (dir, server_dir, props) = if path.join("level.dat").is_file() {
            let parent = path.parent().map(Path::to_path_buf);
            let props = parent
                .as_ref()
                .map(|p| read_properties(&p.join("server.properties")))
                .unwrap_or_default();
            let server = parent.filter(|p| p.join("server.properties").is_file());
            (path.clone(), server, props)
        } else if path.join("server.properties").is_file() {
            let props = read_properties(&path.join("server.properties"));
            let level = props
                .iter()
                .find(|(k, _)| k == "level-name")
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| "world".into());
            (path.join(level), Some(path.clone()), props)
        } else {
            bail!(
                "{} is neither a world folder (level.dat) nor a server folder (server.properties)",
                path.display()
            );
        };
        if !dir.join("level.dat").is_file() {
            bail!(
                "no level.dat in {} — start the server once so it creates the world, then stop it and run chunker",
                dir.display()
            );
        }
        let level = chunker_nbt::read_compressed(fs::File::open(dir.join("level.dat"))?)
            .context("reading level.dat")?;
        let data = level.get_compound("Data").context("level.dat has no Data")?;
        let data_version = data.get_i32("DataVersion").unwrap_or(0);
        let version = McVersion::from_data_version(data_version).with_context(|| {
            format!(
                "world data version {data_version} is not supported (supported: {:?})",
                McVersion::ALL
            )
        })?;
        let level_name = data.get_str("LevelName").unwrap_or("world").to_owned();
        let enabled_packs: Vec<String> = data
            .get_compound("DataPacks")
            .and_then(|d| d.get_list("Enabled"))
            .map(|l| l.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect())
            .unwrap_or_else(|| vec!["vanilla".to_owned()]);

        // 26.x keeps world gen settings in data/minecraft/world_gen_settings.dat; older worlds in level.dat.
        let wgs_path = dir.join("data/minecraft/world_gen_settings.dat");
        let wgs: Compound = if wgs_path.is_file() {
            let c = chunker_nbt::read_compressed(fs::File::open(&wgs_path)?)
                .context("reading world_gen_settings.dat")?;
            c.get_compound("data")
                .cloned()
                .context("world_gen_settings.dat has no data")?
        } else if let Some(c) = data.get_compound("WorldGenSettings") {
            c.clone()
        } else {
            bail!("world has no world gen settings");
        };
        let seed = wgs.get_i64("seed").context("world gen settings without seed")?;
        let generate_structures = wgs
            .get_bool("generate_structures")
            .or(wgs.get_bool("generate_features"))
            .unwrap_or(true);
        let dimensions = match wgs.get_compound("dimensions") {
            Some(d) => d
                .iter()
                .filter_map(|(k, v)| Some((Identifier::parse(k)?, nbt_to_json(v))))
                .collect(),
            None => Vec::new(),
        };
        let spawn = spawn_settings(&dir, data);
        let region_compression = props
            .iter()
            .find(|(k, _)| k == "region-file-compression")
            .map(|(_, v)| v.clone());
        Ok(WorldFiles {
            dir,
            server_dir,
            level_name,
            data_version,
            version,
            seed,
            generate_structures,
            enabled_packs,
            dimensions,
            region_compression,
            spawn,
            lock: None,
        })
    }

    /// Takes `session.lock` like the server does. Fails if the server (or another tool) runs.
    pub fn lock(&mut self) -> Result<()> {
        if self.lock.is_some() {
            return Ok(());
        }
        let path = self.dir.join("session.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => {
                self.lock = Some(file);
                Ok(())
            }
            Err(_) => bail!(
                "the world is in use ({} is locked). Stop the Minecraft server before pre-generating",
                path.display()
            ),
        }
    }

    pub fn unlock(&mut self) {
        if let Some(f) = self.lock.take() {
            let _ = f.unlock();
        }
    }

    pub fn is_locked_by_us(&self) -> bool {
        self.lock.is_some()
    }

    /// Checks whether another process holds `session.lock` (best effort, without keeping it).
    pub fn is_in_use(&self) -> bool {
        if self.lock.is_some() {
            return false;
        }
        let path = self.dir.join("session.lock");
        let Ok(file) = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
        else {
            return false;
        };
        match file.try_lock() {
            Ok(()) => {
                let _ = file.unlock();
                false
            }
            Err(_) => true,
        }
    }

    /// `DimensionType.getStorageFolder`: `dimensions/<ns>/<path>/region`.
    pub fn region_dir(&self, dimension: &Identifier) -> PathBuf {
        self.dir
            .join("dimensions")
            .join(dimension.namespace())
            .join(dimension.path())
            .join("region")
    }

    /// Builds the data pack stack in the order configured in `level.dat`.
    pub fn data_packs(&self, jar: &ServerJar) -> Result<DataPacks> {
        let mut packs = DataPacks::new();
        for name in &self.enabled_packs {
            if name == "vanilla" {
                packs.push(jar.vanilla_pack()?);
            } else if let Some(file) = name.strip_prefix("file/") {
                let p = self.dir.join("datapacks").join(file);
                if p.is_dir() {
                    packs.push(PackSource::directory(p));
                } else if p.is_file() {
                    packs.push(PackSource::zip_file(&p)?);
                } else {
                    tracing::warn!("enabled data pack {name} not found, skipping");
                }
            } else if let Some(pack) = jar.builtin_pack(name)? {
                packs.push(pack);
            } else {
                tracing::warn!("enabled data pack {name} is not supported, skipping");
            }
        }
        if packs.sources().is_empty() {
            packs.push(jar.vanilla_pack()?);
        }
        Ok(packs)
    }
}

impl Drop for WorldFiles {
    fn drop(&mut self) {
        self.unlock();
    }
}
