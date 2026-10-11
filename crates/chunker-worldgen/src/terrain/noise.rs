//! `NoiseChunk` and `NoiseBasedChunkGenerator.doFill`.

use std::sync::Arc;

use anyhow::Result;
use chunker_data::Identifier;
use chunker_data::blocks::BlockState;

use super::aquifer::{Aquifer, AquiferSamplers, FluidBlocks, FluidPicker, NoiseBasedAquifer};
use crate::chunk::{BlockEnv, HeightmapKind, ProtoChunk};
use crate::density::sampler::SamplerRef;
use crate::density::{DensityVolume, SamplerContext};
use crate::random::PositionalRandomFactory;
use crate::random_state::RandomState;
use crate::settings::NoiseGeneratorSettings;

/// Per-dimension noise terrain generator state (compiled samplers, fluid picker).
pub struct NoiseTerrain {
    pub settings: Arc<NoiseGeneratorSettings>,
    pub state: Arc<RandomState>,
    pub env: Arc<BlockEnv>,
    pub final_density: SamplerRef,
    aquifer: Option<AquiferSamplers>,
    aquifer_random: PositionalRandomFactory,
    pub picker: FluidPicker,
    pub fluid_blocks: FluidBlocks,
}

/// Per-chunk evaluation state: one cached sampler context shared by noise, aquifer and surface.
pub struct NoiseChunk {
    pub ctx: SamplerContext,
    pub volume: DensityVolume,
    pub aquifer: Aquifer,
}

impl NoiseTerrain {
    pub fn new(
        settings: Arc<NoiseGeneratorSettings>,
        state: Arc<RandomState>,
        env: Arc<BlockEnv>,
    ) -> Result<Self> {
        let blocks = &env.blocks;
        let fluid_blocks = FluidBlocks {
            air: BlockState::AIR,
            water_default: blocks.vanilla_default("water"),
            lava_default: blocks.vanilla_default("lava"),
        };
        let picker = FluidPicker::new(
            settings.sea_level,
            settings.default_fluid,
            fluid_blocks.lava_default,
        );
        let aquifer = settings.aquifers.as_ref().map(|a| AquiferSamplers {
            barrier: state.sampler(&a.barrier),
            floodedness: state.sampler(&a.fluid_level_floodedness),
            spread: state.sampler(&a.fluid_level_spread),
            lava: state.sampler(&a.lava),
            exclusion: state.sampler(&a.exclusion),
            surface_level: state.sampler(&a.surface_level),
        });
        Ok(Self {
            final_density: state.sampler(&settings.router.final_density),
            aquifer_random: state.random_factory("minecraft:aquifer"),
            aquifer,
            picker,
            fluid_blocks,
            settings,
            state,
            env,
        })
    }

    /// `NoiseSettings.clampToHeightAccessor`: noise range limited to the chunk's build height.
    pub fn clamped_noise_range(&self, chunk_min_y: i32, chunk_height: i32) -> (i32, i32) {
        let n = self.settings.noise;
        let min_y = n.min_y.max(chunk_min_y);
        let height = (n.min_y + n.height).min(chunk_min_y + chunk_height) - min_y;
        (min_y, height)
    }

    /// `new NoiseChunk(...)` for a chunk column; `None` when the noise range is empty.
    pub fn noise_chunk(
        &self,
        chunk_x: i32,
        chunk_z: i32,
        chunk_min_y: i32,
        chunk_height: i32,
        beardifier: Option<crate::density::sampler::SamplerRef>,
    ) -> Option<NoiseChunk> {
        let (min_y, height) = self.clamped_noise_range(chunk_min_y, chunk_height);
        if height <= 0 {
            return None;
        }
        let fields = crate::density::context::UserFields {
            beardifier,
            ..Default::default()
        };
        Some(self.noise_chunk_for(
            DensityVolume::blocks(16, height, 16, chunk_x << 4, min_y, chunk_z << 4),
            fields,
        ))
    }

    /// `new NoiseChunk(randomState, beardifier, settings, fluidPicker, blender, volume)`.
    pub fn noise_chunk_for(
        &self,
        volume: DensityVolume,
        fields: crate::density::context::UserFields,
    ) -> NoiseChunk {
        let mut ctx = SamplerContext::cached(fields);
        let aquifer = match &self.aquifer {
            Some(s) => Aquifer::Noise(Box::new(NoiseBasedAquifer::new(
                &mut ctx,
                s.clone(),
                self.aquifer_random,
                &volume,
                self.picker,
                self.fluid_blocks,
                self.env.blocks.clone(),
            ))),
            None => Aquifer::Disabled(self.picker),
        };
        NoiseChunk { ctx, volume, aquifer }
    }

    /// `NoiseBasedChunkGenerator.iterateNoiseColumn`: the blocks of one column top-down until
    /// `tester` matches. Returns the y above the first matching block (`getBaseHeight`), and
    /// fills `column` (bottom-up, from the clamped noise min y) when given (`getBaseColumn`).
    pub fn iterate_column(
        &self,
        x: i32,
        z: i32,
        dim_min_y: i32,
        dim_height: i32,
        tester: Option<crate::chunk::HeightmapKind>,
        mut column: Option<&mut Vec<BlockState>>,
    ) -> (Option<i32>, i32) {
        let (min_y, height) = self.clamped_noise_range(dim_min_y, dim_height);
        if height <= 0 {
            return (None, min_y);
        }
        let vol = DensityVolume::blocks(1, height, 1, x, min_y, z);
        let mut nc = self.noise_chunk_for(vol, Default::default());
        let mut density = nc.ctx.acquire(vol.size());
        self.final_density.sample_volume(&mut nc.ctx, &mut density, &vol);
        if let Some(c) = column.as_deref_mut() {
            c.clear();
            c.resize(height as usize, BlockState::AIR);
        }
        let default_block = self.settings.default_block;
        let mut result = None;
        for y in (0..vol.size_y).rev() {
            let by = vol.block_y(y);
            let d = density[vol.index(0, y, 0)];
            let state = nc
                .aquifer
                .compute_substance(&mut nc.ctx, x, by, z, d as f64)
                .unwrap_or(default_block);
            if let Some(c) = column.as_deref_mut() {
                c[y as usize] = state;
            }
            if let Some(kind) = tester
                && crate::chunk::is_opaque_in(&self.env, kind, state)
            {
                result = Some(by + 1);
                break;
            }
        }
        nc.ctx.release(density);
        (result, min_y)
    }

    /// `doFill`: final density -> aquifer -> default block, top-down per column.
    pub fn do_fill(&self, nc: &mut NoiseChunk, chunk: &mut ProtoChunk) {
        chunk.heightmaps.ensure(HeightmapKind::OceanFloorWg);
        chunk.heightmaps.ensure(HeightmapKind::WorldSurfaceWg);
        let vol = nc.volume;
        let mut density = nc.ctx.acquire(vol.size());
        let t = super::profile::Timer::start();
        self.final_density.sample_volume(&mut nc.ctx, &mut density, &vol);
        t.lap(super::profile::Phase::FillDensity);
        let default_block = self.settings.default_block;
        for z in 0..vol.size_z {
            let bz = vol.block_z(z);
            for x in 0..vol.size_x {
                let bx = vol.block_x(x);
                for y in (0..vol.size_y).rev() {
                    let by = vol.block_y(y);
                    let d = density[vol.index(x, y, z)];
                    let state = nc
                        .aquifer
                        .compute_substance(&mut nc.ctx, bx, by, bz, d as f64)
                        .unwrap_or(default_block);
                    if state != BlockState::AIR {
                        chunk.set_block_raw(x, by, z, state);
                        crate::chunk::heightmap_update(chunk, HeightmapKind::OceanFloorWg, x, by, z, state);
                        crate::chunk::heightmap_update(chunk, HeightmapKind::WorldSurfaceWg, x, by, z, state);
                        if nc.aquifer.should_schedule_fluid_update() && chunk.blocks.flags(state).has_fluid()
                        {
                            chunk.mark_post_processing(bx, by, bz);
                        }
                    }
                }
            }
        }
        nc.ctx.release(density);
    }
}

impl BlockEnv {
    pub fn load(reg: &crate::registry::WorldgenRegistries) -> Result<Arc<BlockEnv>> {
        Ok(Arc::new(BlockEnv {
            blocks: reg.blocks.clone(),
            motion_blocking: (*reg.block_tag(&Identifier::vanilla("blocks_motion_in_heightmap"))?).clone(),
            motion_blocking_no_leaves: (*reg
                .block_tag(&Identifier::vanilla("blocks_motion_in_heightmap_no_leaves"))?)
            .clone(),
        }))
    }
}
