//! `cargo xtask <command>`
//!
//! * `vanilla [--version 26.3] [--no-decompile] [--no-datagen]` — downloads the official server
//!   jar, extracts the inner jar, libraries and the built-in data pack, runs the vanilla data
//!   generator (reports) and decompiles the server with Vineflower into `vanilla/<version>/`.
//!   Requires Java 25+ on PATH for datagen/decompile. Nothing in `vanilla/` is committed.
//! * `blocks [--version 26.3]` — regenerates `crates/chunker-data/data/<version>/blocks.txt`
//!   from `vanilla/<version>/generated/reports`.
//! * `oracle [--version 26.3]` — compiles the Java oracle in `tools/oracle` against the server jar.
//! * `extract [--version 26.3]` — runs the oracle to regenerate the other tables in
//!   `crates/chunker-data/data/<version>/` (block flags, colours, classes, transforms, shapes,
//!   block entities, default entity attributes, item enchantability).

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha1::{Digest, Sha1};

const MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const VINEFLOWER_URL: &str =
    "https://repo1.maven.org/maven2/org/vineflower/vineflower/1.11.1/vineflower-1.11.1.jar";
const DEFAULT_VERSION: &str = "26.3";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first() else {
        print_help();
        return Ok(());
    };
    let version = flag_value(&args, "--version").unwrap_or_else(|| DEFAULT_VERSION.to_owned());
    let root = workspace_root();
    match cmd.as_str() {
        "vanilla" => {
            let decompile = !args.iter().any(|a| a == "--no-decompile");
            let datagen = !args.iter().any(|a| a == "--no-datagen");
            vanilla(&root, &version, datagen, decompile)
        }
        "blocks" => blocks(&root, &version),
        "oracle" => oracle(&root, &version),
        "extract" => extract(&root, &version),
        _ => {
            print_help();
            bail!("unknown command {cmd}")
        }
    }
}

fn print_help() {
    eprintln!(
        "usage: cargo xtask <vanilla|blocks|oracle|extract> [--version {DEFAULT_VERSION}] [--no-decompile] [--no-datagen]"
    );
}

fn flag_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn http_get(url: &str) -> Result<Vec<u8>> {
    let resp = ureq::get(url).call().with_context(|| format!("GET {url}"))?;
    let mut bytes = Vec::new();
    resp.into_reader().read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn sha1_hex(data: &[u8]) -> String {
    let digest = Sha1::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn vanilla(root: &Path, version: &str, datagen: bool, decompile: bool) -> Result<()> {
    let dir = root.join("vanilla").join(version);
    fs::create_dir_all(&dir)?;
    let bundler_path = dir.join("server-bundler.jar");
    let inner_path = dir.join(format!("server-{version}.jar"));

    if !bundler_path.exists() {
        println!("fetching version manifest");
        let manifest: Value = serde_json::from_slice(&http_get(MANIFEST_URL)?)?;
        let entry = manifest["versions"]
            .as_array()
            .and_then(|v| v.iter().find(|e| e["id"] == version))
            .with_context(|| format!("version {version} not in manifest"))?;
        let meta_bytes = http_get(entry["url"].as_str().unwrap())?;
        fs::write(dir.join("version.json"), &meta_bytes)?;
        let meta: Value = serde_json::from_slice(&meta_bytes)?;
        let server = &meta["downloads"]["server"];
        println!("downloading server jar ({} bytes)", server["size"]);
        let jar = http_get(server["url"].as_str().unwrap())?;
        let expected = server["sha1"].as_str().unwrap();
        if sha1_hex(&jar) != expected {
            bail!("sha1 mismatch for server jar");
        }
        fs::write(&bundler_path, jar)?;
    }

    let bundler = fs::read(&bundler_path)?;
    let mut zip = zip::ZipArchive::new(Cursor::new(bundler))?;
    if !inner_path.exists() {
        let name = format!("META-INF/versions/{version}/server-{version}.jar");
        let mut entry = zip
            .by_name(&name)
            .with_context(|| format!("{name} missing in bundler"))?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        fs::write(&inner_path, bytes)?;
    }
    let libs = dir.join("libs");
    if !libs.exists() {
        fs::create_dir_all(&libs)?;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i)?;
            let name = entry.name().to_owned();
            if name.starts_with("META-INF/libraries/") && name.ends_with(".jar") {
                let file = name.rsplit('/').next().unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                fs::write(libs.join(file), bytes)?;
            }
        }
    }

    let data_dir = dir.join("data");
    if !data_dir.exists() {
        println!("extracting built-in data pack");
        let mut inner = zip::ZipArchive::new(Cursor::new(fs::read(&inner_path)?))?;
        for i in 0..inner.len() {
            let mut entry = inner.by_index(i)?;
            let name = entry.name().to_owned();
            if name.starts_with("data/") && !name.ends_with('/') {
                let out = dir.join(&name);
                fs::create_dir_all(out.parent().unwrap())?;
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes)?;
                fs::File::create(out)?.write_all(&bytes)?;
            }
        }
    }

    if datagen && !dir.join("generated/reports/blocks.json").exists() {
        println!("running vanilla data generator (reports)");
        let work = dir.join("datagen-work");
        fs::create_dir_all(&work)?;
        let status = Command::new("java")
            .current_dir(&work)
            .arg("-DbundlerMainClass=net.minecraft.data.Main")
            .arg("-jar")
            .arg(&bundler_path)
            .arg("--reports")
            .arg("--output")
            .arg(dir.join("generated"))
            .status()
            .context("failed to start java (Java 25+ required)")?;
        if !status.success() {
            bail!("data generator failed");
        }
        let _ = fs::remove_dir_all(&work);
    }

    if decompile && !dir.join("src/net").exists() {
        let vf = root.join("vanilla").join("vineflower.jar");
        if !vf.exists() {
            println!("downloading vineflower");
            fs::write(&vf, http_get(VINEFLOWER_URL)?)?;
        }
        println!("decompiling (this takes a few minutes)");
        let src = dir.join("src");
        fs::create_dir_all(&src)?;
        let mut cmd = Command::new("java");
        cmd.arg("-Xmx4G")
            .arg("-jar")
            .arg(&vf)
            .args(["-dgs=1", "-rsy=1", "-ind=    ", "-log=WARN", "-thr=8"]);
        for lib in fs::read_dir(&libs)? {
            cmd.arg(format!("-e={}", lib?.path().display()));
        }
        cmd.arg(&inner_path).arg(&src);
        let status = cmd.status().context("failed to start java")?;
        if !status.success() {
            bail!("vineflower failed");
        }
    }

    if dir.join("generated/reports/blocks.json").exists() {
        blocks(root, version)?;
    }
    println!("vanilla {version} ready in {}", dir.display());
    Ok(())
}

/// Writes the compact block table: one line per block in registry order:
/// `name|prop=v1,v2;prop2=a,b|default_state_offset`.
fn blocks(root: &Path, version: &str) -> Result<()> {
    let reports = root.join("vanilla").join(version).join("generated/reports");
    let blocks: Value = serde_json::from_slice(&fs::read(reports.join("blocks.json"))?)?;
    let registries: Value = serde_json::from_slice(&fs::read(reports.join("registries.json"))?)?;
    let entries = registries["minecraft:block"]["entries"]
        .as_object()
        .context("block registry")?;
    let mut ordered: Vec<(&String, i64)> = entries
        .iter()
        .map(|(k, v)| (k, v["protocol_id"].as_i64().unwrap()))
        .collect();
    ordered.sort_by_key(|(_, id)| *id);

    let mut out = String::new();
    out.push_str(&format!(
        "# Generated by `cargo xtask blocks --version {version}` from vanilla reports. Do not edit.\n"
    ));
    let mut next_state = 0i64;
    for (name, _) in ordered {
        let block = &blocks[name.as_str()];
        let states = block["states"].as_array().context("states")?;
        let first = states[0]["id"].as_i64().unwrap();
        if first != next_state {
            bail!("non-contiguous state ids at {name}");
        }
        let default = states
            .iter()
            .position(|s| s["default"].as_bool() == Some(true))
            .unwrap_or(0);
        let mut props = Vec::new();
        if let Some(p) = block["properties"].as_object() {
            for (k, vals) in p {
                let vals: Vec<&str> = vals
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                props.push(format!("{k}={}", vals.join(",")));
            }
        }
        let short = name.strip_prefix("minecraft:").unwrap_or(name);
        out.push_str(&format!("{short}|{}|{default}\n", props.join(";")));
        next_state += states.len() as i64;
    }
    let target = root.join("crates/chunker-data/data").join(version);
    fs::create_dir_all(&target)?;
    fs::write(target.join("blocks.txt"), out)?;
    println!("wrote {} block states", next_state);
    Ok(())
}

fn oracle(root: &Path, version: &str) -> Result<()> {
    let vdir = root.join("vanilla").join(version);
    let jar = vdir.join(format!("server-{version}.jar"));
    if !jar.exists() {
        bail!("run `cargo xtask vanilla --version {version}` first");
    }
    let src = root.join("tools/oracle/src");
    let out = root.join("tools/oracle/build");
    fs::create_dir_all(&out)?;
    let mut sources = Vec::new();
    collect_java(&src, &mut sources)?;
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut cp = vec![jar.display().to_string()];
    for lib in fs::read_dir(vdir.join("libs"))? {
        cp.push(lib?.path().display().to_string());
    }
    let status = Command::new("javac")
        .arg("-proc:none")
        .arg("-nowarn")
        .arg("-cp")
        .arg(cp.join(sep))
        .arg("-d")
        .arg(&out)
        .args(&sources)
        .status()
        .context("javac not found")?;
    if !status.success() {
        bail!("javac failed");
    }
    fs::write(out.join("classpath.txt"), cp.join(sep))?;
    println!("oracle compiled into {}", out.display());
    Ok(())
}

fn collect_java(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_java(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "java") {
            out.push(path);
        }
    }
    Ok(())
}

fn run_oracle(root: &Path, commands: &str) -> Result<Vec<String>> {
    let build = root.join("tools/oracle/build");
    let cp = fs::read_to_string(build.join("classpath.txt")).context("run `cargo xtask oracle` first")?;
    let sep = if cfg!(windows) { ";" } else { ":" };
    let work = root.join("target/oracle-work");
    fs::create_dir_all(&work)?;
    let mut child = Command::new("java")
        .current_dir(&work)
        .arg("-cp")
        .arg(format!("{}{sep}{}", build.display(), cp.trim()))
        .arg("chunker.oracle.Oracle")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()?;
    child.stdin.take().unwrap().write_all(commands.as_bytes())?;
    let out = child.wait_with_output()?;
    let text = String::from_utf8(out.stdout)?;
    let lines: Vec<String> = text
        .lines()
        .filter(|l| l.starts_with("ok") || l.starts_with("error"))
        .map(str::to_owned)
        .collect();
    if let Some(e) = lines.iter().find(|l| l.starts_with("error")) {
        bail!("oracle: {e}");
    }
    Ok(lines)
}

fn extract(root: &Path, version: &str) -> Result<()> {
    oracle(root, version)?;
    let lines = run_oracle(root, "blockflags\n")?;
    let flags: Vec<&str> = lines[0].split(' ').skip(1).collect();
    let mut out = format!(
        "# Generated by `cargo xtask extract --version {version}` from vanilla block classes. Do not edit.\n"
    );
    for f in &flags {
        out.push_str(f);
        out.push('\n');
    }
    let target = root.join("crates/chunker-data/data").join(version);
    fs::create_dir_all(&target)?;
    fs::write(target.join("block_flags.txt"), out)?;
    println!("wrote flags for {} block states", flags.len());

    let lines = run_oracle(root, "blockcolors\n")?;
    let colors: Vec<&str> = lines[0].split(' ').skip(1).collect();
    let mut out = format!(
        "# Generated by `cargo xtask extract --version {version}` (MapColor per block state). Do not edit.\n"
    );
    for c in &colors {
        out.push_str(c);
        out.push('\n');
    }
    fs::write(target.join("block_colors.txt"), out)?;
    println!("wrote map colors for {} block states", colors.len());

    let lines = run_oracle(root, "blockclasses\n")?;
    let classes: Vec<&str> = lines[0].split(' ').skip(1).collect();
    let mut out = format!(
        "# Generated by `cargo xtask extract --version {version}` (block class chain per block). Do not edit.\n"
    );
    for c in &classes {
        out.push_str(c);
        out.push('\n');
    }
    fs::write(target.join("block_classes.txt"), out)?;
    println!("wrote classes for {} blocks", classes.len());

    let lines = run_oracle(root, "blocktransforms\n")?;
    let transforms: Vec<&str> = lines[0].split(' ').skip(1).collect();
    let mut out = format!(
        "# Generated by `cargo xtask extract --version {version}` (state id: rotate cw90,cw180,ccw90; mirror left_right,front_back). Do not edit.\n"
    );
    for t in &transforms {
        out.push_str(t);
        out.push('\n');
    }
    fs::write(target.join("block_transforms.txt"), out)?;
    println!("wrote transforms for {} block states", transforms.len());

    let be_file = target.join("block_entities.nbt");
    let lines = run_oracle(root, &format!("blockentities {}\n", be_file.display()))?;
    println!(
        "wrote default block entities for {} blocks",
        lines[0].trim_start_matches("ok ")
    );

    let lines = run_oracle(root, "blockshapes\n")?;
    let shapes: Vec<&str> = lines[0].split(' ').skip(1).collect();
    let mut out = format!(
        "# Generated by `cargo xtask extract --version {version}` (shape tests per block state, see BlockShapes). Do not edit.\n"
    );
    for s in &shapes {
        out.push_str(s);
        out.push('\n');
    }
    fs::write(target.join("block_shapes.txt"), out)?;
    println!("wrote shape tests for {} block states", shapes.len());

    let lines = run_oracle(root, "entityattributes\n")?;
    let entities: Vec<&str> = lines[0].split(' ').skip(1).collect();
    let mut out = format!(
        "# Generated by `cargo xtask extract --version {version}` (entity type|attribute=default base as raw f64 bits). Do not edit.\n"
    );
    for e in &entities {
        out.push_str(e);
        out.push('\n');
    }
    fs::write(target.join("entity_attributes.txt"), out)?;
    println!("wrote default attributes for {} entity types", entities.len());

    let lines = run_oracle(root, "itemenchantable\n")?;
    let items: Vec<&str> = lines[0].split(' ').skip(1).collect();
    let mut out = format!(
        "# Generated by `cargo xtask extract --version {version}` (item=enchantable value). Do not edit.\n"
    );
    for i in &items {
        out.push_str(i);
        out.push('\n');
    }
    fs::write(target.join("item_enchantable.txt"), out)?;
    println!("wrote enchantability for {} items", items.len());
    Ok(())
}
