//! Locating the vanilla server jar for a version.
//!
//! chunker-worldgen never redistributes Mojang data. At runtime it reads the built-in data pack straight
//! from the server jar that already sits next to the world (vanilla, Paper, Purpur, ...), from a
//! local cache, or downloads the official jar from Mojang's servers.

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha1::{Digest, Sha1};

use crate::{McVersion, PackSource};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

/// The inner server jar (the one with `data/` and classes) loaded in memory.
pub struct ServerJar {
    pub version: McVersion,
    pub origin: String,
    bytes: Arc<[u8]>,
}

impl ServerJar {
    pub fn bytes(&self) -> &Arc<[u8]> {
        &self.bytes
    }

    /// The vanilla data pack (`data/` at the jar root).
    pub fn vanilla_pack(&self) -> Result<PackSource> {
        PackSource::zip_bytes(format!("vanilla ({})", self.origin), self.bytes.clone(), "")
    }

    /// A built-in feature pack (`data/minecraft/datapacks/<name>/`), e.g. `trade_rebalance`.
    pub fn builtin_pack(&self, name: &str) -> Result<Option<PackSource>> {
        let base = self.vanilla_pack()?;
        let prefix = format!("data/minecraft/datapacks/{name}/");
        let pack = base.with_prefix(&prefix).context("prefix")?;
        let PackSource::Zip { index, .. } = &pack else {
            return Ok(None);
        };
        Ok((!index.is_empty()).then_some(pack))
    }
}

fn read_zip_entry(bytes: &[u8], name: &str) -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).ok()?;
    let mut entry = zip.by_name(name).ok()?;
    let mut out = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut out).ok()?;
    Some(out)
}

/// Accepts either an inner server jar or a bundler jar; returns the inner jar bytes if it
/// matches `version`.
fn inner_jar_for(bytes: Vec<u8>, version: McVersion) -> Option<Vec<u8>> {
    if let Some(v) = read_zip_entry(&bytes, "version.json") {
        let json: Value = serde_json::from_slice(&v).ok()?;
        if json.get("id").and_then(Value::as_str) == Some(version.id()) {
            // Bundler jars also have version.json; only accept jars that carry the data pack.
            let mut zip = zip::ZipArchive::new(Cursor::new(&bytes)).ok()?;
            if zip
                .by_name("data/minecraft/worldgen/noise_settings/overworld.json")
                .is_ok()
            {
                return Some(bytes);
            }
        }
    }
    let nested = format!("META-INF/versions/{0}/server-{0}.jar", version.id());
    read_zip_entry(&bytes, &nested).and_then(|inner| inner_jar_for(inner, version))
}

/// Candidate jar locations inside a server directory, most specific first.
fn candidates(server_dir: &Path, version: McVersion) -> Vec<PathBuf> {
    let v = version.id();
    let mut out = vec![
        server_dir
            .join("versions")
            .join(v)
            .join(format!("server-{v}.jar")),
        server_dir.join("cache").join(format!("mojang_{v}.jar")),
        server_dir
            .join("libraries/com/mojang/minecraft")
            .join(v)
            .join(format!("minecraft-server-{v}.jar")),
    ];
    if let Ok(entries) = fs::read_dir(server_dir) {
        let mut jars: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jar"))
            .collect();
        jars.sort();
        out.extend(jars);
    }
    out
}

/// Finds the server jar: explicit path, server directory, cache, then download.
pub fn locate(
    version: McVersion,
    explicit: Option<&Path>,
    server_dir: Option<&Path>,
    cache_dir: &Path,
    allow_download: bool,
) -> Result<ServerJar> {
    if let Some(path) = explicit {
        let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let inner = inner_jar_for(bytes, version)
            .with_context(|| format!("{} is not a Minecraft {version} server jar", path.display()))?;
        return Ok(ServerJar {
            version,
            origin: path.display().to_string(),
            bytes: inner.into(),
        });
    }
    let cached = cache_dir
        .join(version.id())
        .join(format!("server-{}.jar", version.id()));
    let mut search: Vec<PathBuf> = server_dir.map(|d| candidates(d, version)).unwrap_or_default();
    search.push(cached.clone());
    for path in search {
        if !path.is_file() {
            continue;
        }
        let Ok(bytes) = fs::read(&path) else { continue };
        if let Some(inner) = inner_jar_for(bytes, version) {
            tracing::info!("using server jar {}", path.display());
            return Ok(ServerJar {
                version,
                origin: path.display().to_string(),
                bytes: inner.into(),
            });
        }
    }
    if !allow_download {
        bail!("no Minecraft {version} server jar found (pass --server-jar or allow downloading)");
    }
    tracing::info!("downloading Minecraft {version} server jar from Mojang");
    let bundler = download_server_jar(version)?;
    let inner = inner_jar_for(bundler, version).context("downloaded jar has no inner server jar")?;
    if let Some(parent) = cached.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&cached, &inner)?;
    Ok(ServerJar {
        version,
        origin: cached.display().to_string(),
        bytes: inner.into(),
    })
}

fn http_get(url: &str) -> Result<Vec<u8>> {
    let resp = ureq::get(url).call().with_context(|| format!("GET {url}"))?;
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn download_server_jar(version: McVersion) -> Result<Vec<u8>> {
    let manifest: Value = serde_json::from_slice(&http_get(MANIFEST_URL)?)?;
    let entry = manifest["versions"]
        .as_array()
        .and_then(|v| v.iter().find(|e| e["id"] == version.id()))
        .with_context(|| format!("version {version} not in Mojang manifest"))?;
    let meta: Value = serde_json::from_slice(&http_get(entry["url"].as_str().context("url")?)?)?;
    let server = &meta["downloads"]["server"];
    let jar = http_get(server["url"].as_str().context("server url")?)?;
    let digest: String = Sha1::digest(&jar).iter().map(|b| format!("{b:02x}")).collect();
    if Some(digest.as_str()) != server["sha1"].as_str() {
        bail!("sha1 mismatch for downloaded server jar");
    }
    Ok(jar)
}
