//! Pre-generates a square of a world's Overworld around 0,0 and prints the progress.
//!
//! ```text
//! cargo run --release --example pregenerate -- <server or world folder> [radius in blocks]
//! ```
//!
//! The Minecraft server must be stopped: the world is locked (`session.lock`) while the job runs.
//! An unfinished job is resumed the next time; existing chunks are never overwritten.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use chunker_worldgen::job::{JobSpec, JobState, Shape};
use chunker_worldgen::{World, WorldOptions};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(
        args.next()
            .context("usage: pregenerate <server or world folder> [radius]")?,
    );
    let radius: i32 = args.next().map(|s| s.parse()).transpose()?.unwrap_or(256);

    let world = World::open(WorldOptions {
        path,
        server_jar: None,
        cache_dir: PathBuf::from("target/server-jars"),
        allow_download: true,
    })?;
    let summary = world.summary();
    println!(
        "{}: Minecraft {}, seed {}, server jar {}",
        summary.level_name, summary.version, summary.seed, summary.server_jar
    );

    let job = match world.job() {
        Some(job) if job.state() == JobState::Paused => {
            println!("resuming the unfinished job");
            world.resume()?;
            job
        }
        _ => world.start(JobSpec {
            dimension: "minecraft:overworld".into(),
            center_x: 0,
            center_z: 0,
            radius,
            radius_z: None,
            shape: Shape::Square,
            skip_existing: true,
            threads: 0,
            decorate: true,
        })?,
    };

    loop {
        let s = job.snapshot();
        println!(
            "{:?}: {}/{} chunks ({} skipped), {:.0} chunks/s",
            s.state,
            s.generated_chunks + s.skipped_chunks,
            s.total_chunks,
            s.skipped_chunks,
            s.chunks_per_second
        );
        if !job.is_active() {
            if let Some(e) = s.error {
                anyhow::bail!("job failed: {e}");
            }
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    world.wait();
    Ok(())
}
