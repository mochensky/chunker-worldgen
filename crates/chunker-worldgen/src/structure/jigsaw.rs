//! `JigsawStructure` + `JigsawPlacement` (villages, outposts, bastions, ancient cities, trail ruins,
//! trial chambers, abandoned camps), template pools and pool elements.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result, bail};
use chunker_data::Identifier;
use chunker_nbt::{Compound, List, Tag};
use rustc_hash::FxHashMap;
use serde_json::Value;

use super::geom::{BlockPos, BoundingBox, Direction, Mirror, Rotation};
use super::start::{PieceBase, PiecesBuilder};
use super::template::{JigsawInfo, TemplateManager};
use super::util::shuffle;
use super::{GenContext, Stub};
use crate::chunk::HeightmapKind;
use crate::codec::{self, field};
use crate::piece_boilerplate;
use crate::providers::{HeightProvider, WeightedList};
use crate::random::{LegacyRandom, RandomSource};
use crate::registry::WorldgenRegistries;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projection {
    TerrainMatching,
    Rigid,
}

impl Projection {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "rigid" => Projection::Rigid,
            "terrain_matching" => Projection::TerrainMatching,
            other => bail!("unknown projection {other}"),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Projection::Rigid => "rigid",
            Projection::TerrainMatching => "terrain_matching",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiquidSettings {
    IgnoreWaterlogging,
    ApplyWaterlogging,
}

impl LiquidSettings {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "ignore_waterlogging" => LiquidSettings::IgnoreWaterlogging,
            "apply_waterlogging" => LiquidSettings::ApplyWaterlogging,
            other => bail!("unknown liquid settings {other}"),
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            LiquidSettings::IgnoreWaterlogging => "ignore_waterlogging",
            LiquidSettings::ApplyWaterlogging => "apply_waterlogging",
        }
    }
}

#[derive(Debug)]
pub enum ElementKind {
    Empty,
    Single {
        legacy: bool,
        location: Identifier,
        processors: Value,
        override_liquid: Option<LiquidSettings>,
    },
    Feature {
        feature: Value,
    },
    List(Vec<PoolElement>),
}

/// `StructurePoolElement`.
#[derive(Debug)]
pub struct PoolElement {
    pub kind: ElementKind,
    pub projection: Projection,
    /// Processors / placed feature parsed on first placement.
    resolved: Resolved,
}

/// Lazily parsed placement data of a pool element.
#[derive(Default)]
struct Resolved(OnceLock<ResolvedData>);

enum ResolvedData {
    Processors(Vec<Arc<super::processor::Processor>>),
    Feature(Option<Arc<crate::feature::registry::PlacedFeature>>),
    None,
}

impl std::fmt::Debug for Resolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Resolved")
    }
}

/// Converts data pack JSON (processor lists, features given inline) to NBT the way `JsonOps` →
/// codec → `NbtOps` would for the vanilla cases: strings, lists and objects; integral numbers as ints.
pub(crate) fn json_to_nbt(v: &Value) -> Tag {
    match v {
        Value::String(s) => Tag::String(s.clone()),
        Value::Bool(b) => Tag::Byte(*b as i8),
        Value::Number(n) => {
            let text = n.to_string();
            match text.parse::<i32>() {
                Ok(i) => Tag::Int(i),
                Err(_) => Tag::Float(text.parse::<f32>().unwrap_or(0.0)),
            }
        }
        Value::Array(a) => {
            let mut l = List::new();
            for e in a {
                l.push(json_to_nbt(e));
            }
            Tag::List(l)
        }
        Value::Object(o) => {
            let mut c = Compound::new();
            for (k, e) in o {
                c.insert(k.clone(), json_to_nbt(e));
            }
            Tag::Compound(c)
        }
        Value::Null => Tag::Compound(Compound::new()),
    }
}

impl PoolElement {
    fn parse(json: &Value, projection_override: Option<Projection>) -> Result<Self> {
        let ty = codec::as_str(field(json, "element_type")?)?;
        let ty = ty.strip_prefix("minecraft:").unwrap_or(ty);
        let projection = match projection_override {
            Some(p) => p,
            None => match json.get("projection") {
                Some(p) => Projection::parse(codec::as_str(p)?)?,
                None => Projection::Rigid,
            },
        };
        let kind = match ty {
            "empty_pool_element" => ElementKind::Empty,
            "single_pool_element" | "legacy_single_pool_element" => ElementKind::Single {
                legacy: ty == "legacy_single_pool_element",
                location: Identifier::parse(codec::as_str(field(json, "location")?)?)
                    .context("bad template location")?,
                processors: field(json, "processors")?.clone(),
                override_liquid: json
                    .get("override_liquid_settings")
                    .map(|v| LiquidSettings::parse(codec::as_str(v)?))
                    .transpose()?,
            },
            "feature_pool_element" => ElementKind::Feature {
                feature: field(json, "feature")?.clone(),
            },
            "list_pool_element" => {
                let mut list = Vec::new();
                for e in codec::as_array(field(json, "elements")?)? {
                    list.push(PoolElement::parse(e, Some(projection))?);
                }
                if list.is_empty() {
                    bail!("Elements are empty");
                }
                ElementKind::List(list)
            }
            other => bail!("unknown pool element type {other}"),
        };
        Ok(Self {
            kind,
            projection,
            resolved: Resolved::default(),
        })
    }

    /// `StructurePoolElement.CODEC` encoded with `NbtOps`.
    pub fn to_nbt(&self) -> Compound {
        let mut c = Compound::new();
        match &self.kind {
            ElementKind::Empty => {
                c.insert("element_type", "minecraft:empty_pool_element");
                return c;
            }
            ElementKind::Single {
                legacy,
                location,
                processors,
                override_liquid,
            } => {
                c.insert(
                    "element_type",
                    if *legacy {
                        "minecraft:legacy_single_pool_element"
                    } else {
                        "minecraft:single_pool_element"
                    },
                );
                c.insert("location", location.as_str());
                c.insert("processors", json_to_nbt(processors));
                if let Some(l) = override_liquid {
                    c.insert("override_liquid_settings", l.name());
                }
            }
            ElementKind::Feature { feature } => {
                c.insert("element_type", "minecraft:feature_pool_element");
                c.insert("feature", json_to_nbt(feature));
            }
            ElementKind::List(list) => {
                c.insert("element_type", "minecraft:list_pool_element");
                let mut l = List::new();
                for e in list {
                    l.push(e.to_nbt());
                }
                c.insert("elements", l);
            }
        }
        c.insert("projection", self.projection.name());
        c
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.kind, ElementKind::Empty)
    }

    /// `getGroundLevelDelta`.
    pub fn ground_level_delta(&self) -> i32 {
        1
    }

    /// `getShuffledJigsawBlocks(manager, position, rotation, random)`.
    pub fn shuffled_jigsaws(
        &self,
        tm: &TemplateManager,
        pos: BlockPos,
        rotation: Rotation,
        random: &mut dyn RandomSource,
    ) -> Vec<JigsawInfo> {
        match &self.kind {
            ElementKind::Empty => Vec::new(),
            ElementKind::Single { location, .. } => {
                let mut jigsaws = tm.get(location).jigsaws(pos, rotation);
                shuffle(&mut jigsaws, random);
                // Stable sort, highest selection priority first.
                jigsaws.sort_by_key(|j| std::cmp::Reverse(j.selection_priority));
                jigsaws
            }
            ElementKind::Feature { .. } => vec![JigsawInfo {
                pos,
                front: Direction::Down,
                top: Direction::South,
                rollable: true,
                name: None,
                pool: Identifier::vanilla("empty"),
                target: Identifier::vanilla("empty"),
                placement_priority: 0,
                selection_priority: 0,
            }],
            ElementKind::List(list) => list[0].shuffled_jigsaws(tm, pos, rotation, random),
        }
    }

    /// `getBoundingBox(manager, position, rotation)`.
    pub fn bounding_box(&self, tm: &TemplateManager, pos: BlockPos, rotation: Rotation) -> BoundingBox {
        match &self.kind {
            ElementKind::Empty => panic!("Invalid call to EmtyPoolElement.getBoundingBox, filter me!"),
            ElementKind::Single { location, .. } => {
                tm.get(location)
                    .bounding_box(pos, rotation, BlockPos::ZERO, Mirror::None)
            }
            ElementKind::Feature { .. } => BoundingBox::new(pos.x, pos.y, pos.z, pos.x, pos.y, pos.z),
            ElementKind::List(list) => {
                let boxes: Vec<BoundingBox> = list
                    .iter()
                    .filter(|e| !e.is_empty())
                    .map(|e| e.bounding_box(tm, pos, rotation))
                    .collect();
                BoundingBox::encapsulating(boxes.iter())
                    .expect("Unable to calculate boundingbox for ListPoolElement")
            }
        }
    }
}

/// `StructureTemplatePool`.
pub struct Pool {
    pub fallback: Identifier,
    /// Elements repeated by weight (`templates`).
    pub templates: Vec<Arc<PoolElement>>,
    max_size: OnceLock<i32>,
}

impl Pool {
    fn parse(json: &Value) -> Result<Pool> {
        let fallback =
            Identifier::parse(codec::as_str(field(json, "fallback")?)?).context("bad fallback pool")?;
        let mut templates = Vec::new();
        for e in codec::as_array(field(json, "elements")?)? {
            let element = Arc::new(PoolElement::parse(field(e, "element")?, None)?);
            let weight = codec::as_i32(field(e, "weight")?)?;
            for _ in 0..weight {
                templates.push(element.clone());
            }
        }
        Ok(Pool {
            fallback,
            templates,
            max_size: OnceLock::new(),
        })
    }

    pub fn size(&self) -> usize {
        self.templates.len()
    }

    /// `getRandomTemplate`.
    pub fn random_template(&self, random: &mut dyn RandomSource) -> Option<&Arc<PoolElement>> {
        if self.templates.is_empty() {
            None
        } else {
            Some(&self.templates[random.next_int_bounded(self.templates.len() as i32) as usize])
        }
    }

    /// `getShuffledTemplates`.
    pub fn shuffled(&self, random: &mut dyn RandomSource) -> Vec<Arc<PoolElement>> {
        let mut v = self.templates.clone();
        shuffle(&mut v, random);
        v
    }

    /// `getMaxSize`.
    pub fn max_size(&self, tm: &TemplateManager) -> i32 {
        *self.max_size.get_or_init(|| {
            self.templates
                .iter()
                .filter(|t| !t.is_empty())
                .map(|t| t.bounding_box(tm, BlockPos::ZERO, Rotation::None).y_span())
                .max()
                .unwrap_or(0)
        })
    }
}

/// All template pools of the data packs.
pub struct PoolRegistry {
    pub pools: FxHashMap<Identifier, Pool>,
}

impl PoolRegistry {
    pub fn load(reg: &WorldgenRegistries) -> Result<Self> {
        let mut pools = FxHashMap::default();
        for id in reg.packs.list("worldgen/template_pool") {
            let json = reg.packs.require_json("worldgen/template_pool", &id)?;
            pools.insert(
                id.clone(),
                Pool::parse(&json).with_context(|| format!("template pool {id}"))?,
            );
        }
        Ok(Self { pools })
    }
}

/// `PoolAliasBinding`.
pub enum AliasBinding {
    Direct {
        alias: Identifier,
        target: Identifier,
    },
    Random {
        alias: Identifier,
        targets: WeightedList<Identifier>,
    },
    RandomGroup {
        groups: WeightedList<Vec<AliasBinding>>,
    },
}

impl AliasBinding {
    fn parse(json: &Value) -> Result<Self> {
        let id =
            |v: &Value| -> Result<Identifier> { Identifier::parse(codec::as_str(v)?).context("bad pool id") };
        Ok(match codec::type_name(json)?.as_str() {
            "direct" => AliasBinding::Direct {
                alias: id(field(json, "alias")?)?,
                target: id(field(json, "target")?)?,
            },
            "random" => AliasBinding::Random {
                alias: id(field(json, "alias")?)?,
                targets: WeightedList::parse(field(json, "targets")?, id)?,
            },
            "random_group" => AliasBinding::RandomGroup {
                groups: WeightedList::parse(field(json, "groups")?, |v| {
                    codec::as_array(v)?.iter().map(AliasBinding::parse).collect()
                })?,
            },
            other => bail!("unknown pool alias binding {other}"),
        })
    }

    fn resolve(&self, random: &mut dyn RandomSource, out: &mut Vec<(Identifier, Identifier)>) {
        match self {
            AliasBinding::Direct { alias, target } => out.push((alias.clone(), target.clone())),
            AliasBinding::Random { alias, targets } => out.push((
                alias.clone(),
                targets
                    .get_random(random)
                    .expect("empty pool alias targets")
                    .clone(),
            )),
            AliasBinding::RandomGroup { groups } => {
                for b in groups.get_random(random).expect("empty pool alias group") {
                    b.resolve(random, out);
                }
            }
        }
    }
}

/// `JigsawStructure` configuration.
pub struct JigsawConfig {
    pub start_pool: Identifier,
    pub start_jigsaw_name: Option<Identifier>,
    pub max_depth: i32,
    pub start_height: HeightProvider,
    pub use_expansion_hack: bool,
    pub project_start_to_heightmap: Option<HeightmapKind>,
    pub max_distance_horizontal: i32,
    pub max_distance_vertical: i32,
    pub pool_aliases: Vec<AliasBinding>,
    /// `None` = `DimensionPadding.ZERO` (no check); an explicit value is always checked.
    pub dimension_padding: Option<(i32, i32)>,
    pub liquid_settings: LiquidSettings,
}

fn heightmap_by_name(s: &str) -> Result<HeightmapKind> {
    HeightmapKind::ALL
        .iter()
        .copied()
        .find(|k| k.key() == s)
        .with_context(|| format!("unknown heightmap {s}"))
}

impl JigsawConfig {
    pub fn parse(_reg: &WorldgenRegistries, json: &Value) -> Result<Self> {
        let (max_distance_horizontal, max_distance_vertical) = match field(json, "max_distance_from_center")?
        {
            Value::Object(_) => {
                let d = field(json, "max_distance_from_center")?;
                (
                    codec::as_i32(field(d, "horizontal")?)?,
                    codec::opt_i32(d, "vertical", 4064)?,
                )
            }
            v => {
                let n = codec::as_i32(v)?;
                (n, n)
            }
        };
        let dimension_padding = match json.get("dimension_padding") {
            None => None,
            Some(Value::Object(o)) => Some((
                o.get("bottom").map(codec::as_i32).transpose()?.unwrap_or(0),
                o.get("top").map(codec::as_i32).transpose()?.unwrap_or(0),
            )),
            Some(v) => {
                let n = codec::as_i32(v)?;
                Some((n, n))
            }
        };
        Ok(Self {
            start_pool: Identifier::parse(codec::as_str(field(json, "start_pool")?)?)
                .context("bad start pool")?,
            start_jigsaw_name: json
                .get("start_jigsaw_name")
                .map(|v| Identifier::parse(codec::as_str(v)?).context("bad jigsaw name"))
                .transpose()?,
            max_depth: codec::as_i32(field(json, "size")?)?,
            start_height: HeightProvider::parse(field(json, "start_height")?)?,
            use_expansion_hack: codec::as_bool(field(json, "use_expansion_hack")?)?,
            project_start_to_heightmap: json
                .get("project_start_to_heightmap")
                .map(|v| heightmap_by_name(codec::as_str(v)?))
                .transpose()?,
            max_distance_horizontal,
            max_distance_vertical,
            pool_aliases: match json.get("pool_aliases") {
                Some(v) => codec::as_array(v)?
                    .iter()
                    .map(AliasBinding::parse)
                    .collect::<Result<_>>()?,
                None => Vec::new(),
            },
            dimension_padding,
            liquid_settings: match json.get("liquid_settings") {
                Some(v) => LiquidSettings::parse(codec::as_str(v)?)?,
                None => LiquidSettings::ApplyWaterlogging,
            },
        })
    }
}

/// `JigsawJunction`.
#[derive(Clone, Debug)]
pub struct Junction {
    pub source_x: i32,
    pub source_ground_y: i32,
    pub source_z: i32,
    pub delta_y: i32,
    pub dest_projection: Projection,
}

impl PoolElement {
    fn resolved(&self, region: &crate::feature::Region) -> &ResolvedData {
        self.resolved.0.get_or_init(|| {
            let env = region.env;
            let p = crate::feature::registry::Parser::new(&env.reg, &env.biomes);
            match &self.kind {
                ElementKind::Single {
                    processors, location, ..
                } => match super::processor::parse_list(&p, processors) {
                    Ok(list) => ResolvedData::Processors(list),
                    Err(e) => {
                        tracing::warn!("processors of pool element {location}: {e:#}");
                        ResolvedData::Processors(Vec::new())
                    }
                },
                ElementKind::Feature { feature } => ResolvedData::Feature(match p.placed(feature) {
                    Ok(f) => Some(f),
                    Err(e) => {
                        tracing::warn!("feature of pool element: {e:#}");
                        None
                    }
                }),
                _ => ResolvedData::None,
            }
        })
    }

    /// `StructurePoolElement.place(..., keepJigsaws = false)`.
    pub fn place(
        &self,
        ctx: &mut super::build::PieceCtx,
        position: BlockPos,
        rotation: Rotation,
        random: &mut dyn RandomSource,
        liquid: LiquidSettings,
    ) -> bool {
        use super::processor::Processor;
        match &self.kind {
            ElementKind::Empty => true,
            ElementKind::List(list) => list
                .iter()
                .all(|e| e.place(ctx, position, rotation, random, liquid)),
            ElementKind::Feature { .. } => match self.resolved(ctx.region) {
                ResolvedData::Feature(Some(f)) => {
                    crate::feature::placement::place(ctx.region, f, None, random, position)
                }
                _ => false,
            },
            ElementKind::Single {
                legacy,
                location,
                override_liquid,
                ..
            } => {
                let template = ctx.structures.templates.get(location);
                let b = &ctx.region.env.b;
                // `SinglePoolElement.getSettings` (legacy elements move the ignore processor to the end
                // and also ignore air).
                let mut processors: Vec<Arc<Processor>> = Vec::new();
                if !legacy {
                    processors.push(Arc::new(Processor::BlockIgnore(vec![b.structure_block])));
                }
                processors.push(Arc::new(Processor::JigsawReplacement));
                if let ResolvedData::Processors(list) = self.resolved(ctx.region) {
                    processors.extend(list.iter().cloned());
                }
                if self.projection == Projection::TerrainMatching {
                    processors.push(Arc::new(Processor::Gravity {
                        heightmap: HeightmapKind::WorldSurfaceWg,
                        offset: -1,
                    }));
                }
                if *legacy {
                    processors.push(Arc::new(Processor::BlockIgnore(vec![b.air, b.structure_block])));
                }
                let settings = super::place::PlaceSettings {
                    rotation,
                    bounding_box: Some(ctx.chunk_bb),
                    known_shape: true,
                    finalize_entities: true,
                    waterlogging: override_liquid.unwrap_or(liquid) == LiquidSettings::ApplyWaterlogging,
                    processors,
                    ..Default::default()
                };
                // Data markers only feed `handleDataMarker`, a no-op for pool elements.
                super::place::place_in_world(
                    ctx.region,
                    template,
                    position,
                    ctx.reference,
                    &settings,
                    random,
                    18,
                )
            }
        }
    }
}

/// `PoolElementStructurePiece`.
pub struct JigsawPiece {
    pub base: PieceBase,
    pub element: Arc<PoolElement>,
    pub position: BlockPos,
    pub ground_level_delta: i32,
    pub rotation: Rotation,
    pub junctions: Vec<Junction>,
    pub liquid_settings: LiquidSettings,
}

impl super::start::Piece for JigsawPiece {
    piece_boilerplate!();
    fn type_id(&self) -> &'static str {
        "minecraft:jigsaw"
    }
    fn save_extra(&self, tag: &mut Compound) {
        tag.insert("PosX", self.position.x);
        tag.insert("PosY", self.position.y);
        tag.insert("PosZ", self.position.z);
        tag.insert("ground_level_delta", self.ground_level_delta);
        tag.insert("pool_element", self.element.to_nbt());
        tag.insert("rotation", self.rotation.name());
        let mut junctions = List::new();
        for j in &self.junctions {
            let mut c = Compound::new();
            c.insert("source_x", j.source_x);
            c.insert("source_ground_y", j.source_ground_y);
            c.insert("source_z", j.source_z);
            c.insert("delta_y", j.delta_y);
            c.insert("dest_proj", j.dest_projection.name());
            junctions.push(c);
        }
        tag.insert("junctions", junctions);
        if self.liquid_settings != LiquidSettings::ApplyWaterlogging {
            tag.insert("liquid_settings", self.liquid_settings.name());
        }
    }
    fn move_by(&mut self, dx: i32, dy: i32, dz: i32) {
        self.base.bbox.move_by(dx, dy, dz);
        self.position = self.position.offset(dx, dy, dz);
    }
    fn post_process_ported(&self) -> bool {
        true
    }
    /// `PoolElementStructurePiece.postProcess`.
    fn post_process(&self, ctx: &mut super::build::PieceCtx, random: &mut dyn RandomSource) {
        self.element
            .place(ctx, self.position, self.rotation, random, self.liquid_settings);
    }
}

impl JigsawPiece {
    fn new(
        tm: &TemplateManager,
        element: Arc<PoolElement>,
        position: BlockPos,
        ground_level_delta: i32,
        rotation: Rotation,
        bbox: Option<BoundingBox>,
        liquid: LiquidSettings,
    ) -> Self {
        let bbox = bbox.unwrap_or_else(|| element.bounding_box(tm, position, rotation));
        Self {
            base: PieceBase::new(0, bbox, None),
            element,
            position,
            ground_level_delta,
            rotation,
            junctions: Vec::new(),
            liquid_settings: liquid,
        }
    }
}

/// A free space for children: `outer` cells minus every placed box (an exact model of the
/// vanilla `VoxelShape` bookkeeping, since all boxes involved are block aligned).
struct FreeRegion {
    outer: Option<BoundingBox>,
    placed: Vec<BoundingBox>,
}

impl FreeRegion {
    /// `!Shapes.joinIsNotEmpty(free, box.deflate(0.25), ONLY_SECOND)`: every cell of `b` is free.
    fn contains(&self, b: &BoundingBox) -> bool {
        let Some(o) = &self.outer else { return false };
        b.min_x >= o.min_x
            && b.max_x <= o.max_x
            && b.min_y >= o.min_y
            && b.max_y <= o.max_y
            && b.min_z >= o.min_z
            && b.max_z <= o.max_z
            && !self.placed.iter().any(|p| p.intersects(b))
    }
}

/// `PoolAliasLookup.create(bindings, pos, seed)`.
fn alias_lookup(bindings: &[AliasBinding], pos: BlockPos, seed: i64) -> FxHashMap<Identifier, Identifier> {
    let mut map = FxHashMap::default();
    if bindings.is_empty() {
        return map;
    }
    let mut random = LegacyRandom::new(seed).fork_positional().at(pos.x, pos.y, pos.z);
    let mut resolved = Vec::new();
    for b in bindings {
        b.resolve(&mut random, &mut resolved);
    }
    for (k, v) in resolved {
        map.entry(k).or_insert(v);
    }
    map
}

fn lookup<'a>(aliases: &'a FxHashMap<Identifier, Identifier>, id: &'a Identifier) -> &'a Identifier {
    aliases.get(id).unwrap_or(id)
}

/// `JigsawStructure.findGenerationPoint` + `JigsawPlacement.addPieces`.
pub fn find_generation_point<'c>(ctx: &mut GenContext, config: &'c JigsawConfig) -> Option<Stub<'c>> {
    let gen_ctx = ctx.terrain.generation_context_for(ctx.min_y, ctx.height);
    let height = config.start_height.sample(&mut ctx.random, &gen_ctx);
    let position = BlockPos::new(ctx.min_block_x(), height, ctx.min_block_z());
    let aliases = alias_lookup(&config.pool_aliases, position, ctx.seed);
    let registry = ctx.structures.registry.clone();
    let tm = ctx.structures.templates.clone();

    let center_rotation = Rotation::random(&mut ctx.random);
    let center_element = {
        let pools = &registry.pools.pools;
        let start_id = lookup(&aliases, &config.start_pool);
        let center_pool = pools.get(start_id).or_else(|| pools.get(&config.start_pool))?;
        center_pool.random_template(&mut ctx.random)?.clone()
    };
    if center_element.is_empty() {
        return None;
    }
    let anchored = match &config.start_jigsaw_name {
        Some(name) => {
            let jigsaws = center_element.shuffled_jigsaws(&tm, position, center_rotation, &mut ctx.random);
            match jigsaws.iter().find(|j| j.name.as_ref() == Some(name)) {
                Some(j) => j.pos,
                None => {
                    tracing::error!(
                        "No starting jigsaw {name} found in start pool {}",
                        config.start_pool
                    );
                    return None;
                }
            }
        }
        None => position,
    };
    let local_anchor = anchored.sub(position);
    let adjusted = position.sub(local_anchor);
    let mut center = JigsawPiece::new(
        &tm,
        center_element.clone(),
        adjusted,
        center_element.ground_level_delta(),
        center_rotation,
        None,
        config.liquid_settings,
    );
    let bbox = center.base.bbox;
    let center_x = (bbox.max_x + bbox.min_x) / 2;
    let center_z = (bbox.max_z + bbox.min_z) / 2;
    let bottom_y = match config.project_start_to_heightmap {
        None => adjusted.y,
        Some(kind) => {
            let (min, max) = (ctx.min_y, ctx.max_y());
            if !ctx.could_structure_exist_in_column(center_x, center_z, min, max) {
                return None;
            }
            position.y + ctx.first_free_height(center_x, center_z, kind)
        }
    };
    let old_ground = bbox.min_y + center.ground_level_delta;
    super::start::Piece::move_by(&mut center, 0, bottom_y - old_ground, 0);
    if let Some((bottom, top)) = config.dimension_padding {
        let b = center.base.bbox;
        if b.min_y < ctx.min_y + bottom || b.max_y > ctx.max_y() - top {
            return None;
        }
    }
    let center_y = bottom_y + local_anchor.y;
    let start = BlockPos::new(center_x, center_y, center_z);
    Some(Stub::lazy(
        start,
        move |ctx: &mut GenContext, builder: &mut PiecesBuilder| {
            let mut pieces = vec![center];
            if config.max_depth > 0 {
                let (bottom, top) = config.dimension_padding.unwrap_or((0, 0));
                let h = config.max_distance_horizontal;
                let v = config.max_distance_vertical;
                // AABB corners (sorted like the AABB constructor), then converted to block cells.
                let y_a = (center_y - v).max(ctx.min_y + bottom);
                let y_b = (center_y + v + 1).min(ctx.max_y() + 1 - top);
                let (y0, y1) = (y_a.min(y_b), y_a.max(y_b));
                let outer = (y1 > y0).then(|| {
                    BoundingBox::new(center_x - h, y0, center_z - h, center_x + h, y1 - 1, center_z + h)
                });
                let mut frees = vec![FreeRegion {
                    outer,
                    placed: vec![bbox_after_move(&pieces[0])],
                }];
                let mut placer = Placer {
                    pools: &registry.pools.pools,
                    tm: &tm,
                    max_depth: config.max_depth,
                    expansion_hack: config.use_expansion_hack,
                    aliases: &aliases,
                    liquid: config.liquid_settings,
                    queue: BTreeMap::new(),
                };
                placer.try_placing_children(ctx, &mut pieces, &mut frees, 0, 0, 0);
                while let Some((piece, free, depth)) = placer.pop() {
                    placer.try_placing_children(ctx, &mut pieces, &mut frees, piece, free, depth);
                }
            }
            for p in pieces {
                builder.add(Box::new(p));
            }
        },
    ))
}

fn bbox_after_move(p: &JigsawPiece) -> BoundingBox {
    p.base.bbox
}

struct Placer<'a> {
    pools: &'a FxHashMap<Identifier, Pool>,
    tm: &'a TemplateManager,
    max_depth: i32,
    expansion_hack: bool,
    aliases: &'a FxHashMap<Identifier, Identifier>,
    liquid: LiquidSettings,
    /// `SequencedPriorityIterator`: highest priority first, FIFO within a priority.
    queue: BTreeMap<i32, VecDeque<(usize, usize, i32)>>,
}

impl Placer<'_> {
    fn pop(&mut self) -> Option<(usize, usize, i32)> {
        let mut entry = self.queue.last_entry()?;
        let item = entry.get_mut().pop_front();
        if entry.get().is_empty() {
            entry.remove();
        }
        item
    }

    fn try_placing_children(
        &mut self,
        ctx: &mut GenContext,
        pieces: &mut Vec<JigsawPiece>,
        frees: &mut Vec<FreeRegion>,
        source: usize,
        context_free: usize,
        depth: i32,
    ) {
        let source_element = pieces[source].element.clone();
        let source_pos = pieces[source].position;
        let source_rotation = pieces[source].rotation;
        let source_projection = source_element.projection;
        let source_rigid = source_projection == Projection::Rigid;
        let mut source_free: Option<usize> = None;
        let source_bb = pieces[source].base.bbox;
        let source_box_y = source_bb.min_y;
        let jigsaws = source_element.shuffled_jigsaws(self.tm, source_pos, source_rotation, &mut ctx.random);
        'jigsaws: for source_jigsaw in &jigsaws {
            let source_direction = source_jigsaw.front;
            let source_jigsaw_pos = source_jigsaw.pos;
            let target_jigsaw_pos = source_jigsaw_pos.relative(source_direction, 1);
            let source_jigsaw_local_y = source_jigsaw_pos.y - source_box_y;
            let mut source_base_height = i32::MIN;
            let pool_name = lookup(self.aliases, &source_jigsaw.pool);
            let Some(target_pool) = self.pools.get(pool_name) else {
                tracing::warn!("Empty or non-existent pool: {pool_name}");
                continue;
            };
            if target_pool.size() == 0 && pool_name.as_str() != "minecraft:empty" {
                tracing::warn!("Empty or non-existent pool: {pool_name}");
                continue;
            }
            let fallback_name = &target_pool.fallback;
            let Some(fallback) = self.pools.get(fallback_name) else {
                tracing::warn!("Empty or non-existent fallback pool: {fallback_name}");
                continue;
            };
            if fallback.size() == 0 && fallback_name.as_str() != "minecraft:empty" {
                tracing::warn!("Empty or non-existent fallback pool: {fallback_name}");
                continue;
            }
            let attach_inside = source_bb.contains(target_jigsaw_pos);
            let children_free = if attach_inside {
                *source_free.get_or_insert_with(|| {
                    frees.push(FreeRegion {
                        outer: Some(source_bb),
                        placed: Vec::new(),
                    });
                    frees.len() - 1
                })
            } else {
                context_free
            };
            let mut targets: Vec<Arc<PoolElement>> = Vec::new();
            if depth != self.max_depth {
                targets.extend(target_pool.shuffled(&mut ctx.random));
            }
            targets.extend(fallback.shuffled(&mut ctx.random));
            let placement_priority = source_jigsaw.placement_priority;
            for target_element in targets {
                if target_element.is_empty() {
                    break;
                }
                for target_rotation in Rotation::shuffled(&mut ctx.random) {
                    let target_jigsaws = target_element.shuffled_jigsaws(
                        self.tm,
                        BlockPos::ZERO,
                        target_rotation,
                        &mut ctx.random,
                    );
                    let hack_box = target_element.bounding_box(self.tm, BlockPos::ZERO, target_rotation);
                    let expand_to = if self.expansion_hack && hack_box.y_span() <= 16 {
                        target_jigsaws
                            .iter()
                            .map(|tj| {
                                if !hack_box.contains(tj.pos.relative(tj.front, 1)) {
                                    return 0;
                                }
                                let child = self.pools.get(lookup(self.aliases, &tj.pool));
                                let child_size = child.map_or(0, |p| p.max_size(self.tm));
                                let fallback_size = child
                                    .and_then(|p| self.pools.get(&p.fallback))
                                    .map_or(0, |p| p.max_size(self.tm));
                                child_size.max(fallback_size)
                            })
                            .max()
                            .unwrap_or(0)
                    } else {
                        0
                    };
                    for target_jigsaw in &target_jigsaws {
                        if !source_jigsaw.can_attach(target_jigsaw) {
                            continue;
                        }
                        let target_local = target_jigsaw.pos;
                        let raw_target_pos = target_jigsaw_pos.sub(target_local);
                        let raw_target_bb =
                            target_element.bounding_box(self.tm, raw_target_pos, target_rotation);
                        let raw_target_y = raw_target_bb.min_y;
                        let target_projection = target_element.projection;
                        let target_rigid = target_projection == Projection::Rigid;
                        let target_jigsaw_local_y = target_local.y;
                        let delta_y =
                            source_jigsaw_local_y - target_jigsaw_local_y + source_direction.step().1;
                        let target_box_y = if source_rigid && target_rigid {
                            source_box_y + delta_y
                        } else {
                            if source_base_height == i32::MIN {
                                source_base_height = ctx.first_free_height(
                                    source_jigsaw_pos.x,
                                    source_jigsaw_pos.z,
                                    HeightmapKind::WorldSurfaceWg,
                                );
                            }
                            source_base_height - target_jigsaw_local_y
                        };
                        let y_offset = target_box_y - raw_target_y;
                        let mut target_bb = raw_target_bb.moved(0, y_offset, 0);
                        let target_pos = raw_target_pos.offset(0, y_offset, 0);
                        if expand_to > 0 {
                            let new_size = (expand_to + 1).max(target_bb.max_y - target_bb.min_y);
                            target_bb.encapsulate_pos(BlockPos::new(
                                target_bb.min_x,
                                target_bb.min_y + new_size,
                                target_bb.min_z,
                            ));
                        }
                        if !frees[children_free].contains(&target_bb) {
                            continue;
                        }
                        frees[children_free].placed.push(target_bb);
                        let source_ground_delta = pieces[source].ground_level_delta;
                        let target_ground_delta = if target_rigid {
                            source_ground_delta - delta_y
                        } else {
                            target_element.ground_level_delta()
                        };
                        let mut target_piece = JigsawPiece::new(
                            self.tm,
                            target_element.clone(),
                            target_pos,
                            target_ground_delta,
                            target_rotation,
                            Some(target_bb),
                            self.liquid,
                        );
                        let junction_y = if source_rigid {
                            source_box_y + source_jigsaw_local_y
                        } else if target_rigid {
                            target_box_y + target_jigsaw_local_y
                        } else {
                            if source_base_height == i32::MIN {
                                source_base_height = ctx.first_free_height(
                                    source_jigsaw_pos.x,
                                    source_jigsaw_pos.z,
                                    HeightmapKind::WorldSurfaceWg,
                                );
                            }
                            source_base_height + delta_y / 2
                        };
                        pieces[source].junctions.push(Junction {
                            source_x: target_jigsaw_pos.x,
                            source_ground_y: junction_y - source_jigsaw_local_y + source_ground_delta,
                            source_z: target_jigsaw_pos.z,
                            delta_y,
                            dest_projection: target_projection,
                        });
                        target_piece.junctions.push(Junction {
                            source_x: source_jigsaw_pos.x,
                            source_ground_y: junction_y - target_jigsaw_local_y + target_ground_delta,
                            source_z: source_jigsaw_pos.z,
                            delta_y: -delta_y,
                            dest_projection: source_projection,
                        });
                        pieces.push(target_piece);
                        if depth < self.max_depth {
                            self.queue.entry(placement_priority).or_default().push_back((
                                pieces.len() - 1,
                                children_free,
                                depth + 1,
                            ));
                        }
                        continue 'jigsaws;
                    }
                }
            }
        }
    }
}
