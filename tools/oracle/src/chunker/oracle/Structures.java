package chunker.oracle;

import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;
import java.util.concurrent.ConcurrentHashMap;

import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.Registries;
import net.minecraft.nbt.ByteArrayTag;
import net.minecraft.nbt.ByteTag;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.DoubleTag;
import net.minecraft.nbt.FloatTag;
import net.minecraft.nbt.IntArrayTag;
import net.minecraft.nbt.IntTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.LongArrayTag;
import net.minecraft.nbt.LongTag;
import net.minecraft.nbt.ShortTag;
import net.minecraft.nbt.StringTag;
import net.minecraft.nbt.Tag;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.util.datafix.DataFixers;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.StructureManager;
import net.minecraft.world.level.biome.BiomeSource;
import net.minecraft.world.level.chunk.ChunkGeneratorStructureState;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.structure.Structure;
import net.minecraft.world.level.levelgen.structure.StructureStart;
import net.minecraft.world.level.levelgen.structure.pieces.StructurePieceSerializationContext;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager;
import net.minecraft.world.level.storage.LevelStorageSource;

/** Structure starts of single chunks, printed as canonical NBT for chunker-worldgen's golden tests. */
final class Structures {
    private record Context(NoiseBasedChunkGenerator generator, ChunkGeneratorStructureState state, StructureManager manager,
                           StructureTemplateManager templates, StructurePieceSerializationContext serialization) {
    }

    private static final Map<String, Context> CONTEXTS = new ConcurrentHashMap<>();
    private static StructureTemplateManager templateManager;

    private static synchronized StructureTemplateManager templates(RegistryAccess world, ResourceManager resources) throws Exception {
        if (templateManager == null) {
            var storage = LevelStorageSource.createDefault(java.nio.file.Files.createTempDirectory("chunker-oracle")).createAccess("oracle");
            templateManager = new StructureTemplateManager(resources, storage, DataFixers.getDataFixer(), net.minecraft.core.registries.BuiltInRegistries.BLOCK);
        }
        return templateManager;
    }

    private static Context context(RegistryAccess.Frozen world, ResourceManager resources, long seed, BiomeSource source,
                                   NoiseGeneratorSettings settings, RandomState rs) throws Exception {
        String key = seed + "|" + source + "|" + System.identityHashCode(settings);
        StructureTemplateManager templates = templates(world, resources);
        return CONTEXTS.computeIfAbsent(key, k -> {
            var generator = new NoiseBasedChunkGenerator(source, net.minecraft.core.Holder.direct(settings));
            var state = ChunkGeneratorStructureState.createForNormal(rs, seed, ChunkPos.ZERO, source, world.lookupOrThrow(Registries.STRUCTURE_SET));
            state.ensureStructuresGenerated();
            var manager = new StructureManager(null, new WorldOptions(seed, true, false), null);
            return new Context(generator, state, manager, templates, new StructurePieceSerializationContext(resources, world, templates));
        });
    }

    static net.minecraft.world.level.chunk.ChunkGenerator generator(RegistryAccess.Frozen world, ResourceManager resources, long seed, BiomeSource source,
                                                                     NoiseGeneratorSettings settings, RandomState rs) throws Exception {
        return context(world, resources, seed, source, settings, rs).generator;
    }

    static Map<Structure, StructureStart> startsFor(RegistryAccess.Frozen world, ResourceManager resources, long seed, BiomeSource source,
                                                     NoiseGeneratorSettings settings, RandomState rs, String dimension, int cx, int cz, int minY, int height)
        throws Exception {
        return startsOf(world, context(world, resources, seed, source, settings, rs), dimension, cx, cz, minY, height);
    }

    static StructureTemplateManager templateManager(RegistryAccess world, ResourceManager resources) throws Exception {
        return templates(world, resources);
    }

    private static final Map<String, Map<Structure, StructureStart>> START_CACHE = new ConcurrentHashMap<>();

    private static Map<Structure, StructureStart> startsOf(RegistryAccess.Frozen world, Context ctx, String dimension, int cx, int cz, int minY, int height) {
        String key = System.identityHashCode(ctx) + "|" + dimension + "|" + cx + "|" + cz;
        Map<Structure, StructureStart> cached = START_CACHE.get(key);
        if (cached != null) {
            return cached;
        }
        var heightAccessor = LevelHeightAccessor.create(minY, height);
        var chunk = new ProtoChunk(new ChunkPos(cx, cz), UpgradeData.EMPTY, heightAccessor, PalettedContainerFactory.create(world), null);
        ctx.generator.createStructures(world, ctx.state, ctx.manager, chunk, ctx.templates,
            ResourceKey.create(Registries.DIMENSION, Identifier.parse(dimension)));
        Map<Structure, StructureStart> result = Map.copyOf(chunk.getAllStarts());
        START_CACHE.put(key, result);
        return result;
    }

    static String starts(RegistryAccess.Frozen world, ResourceManager resources, long seed, BiomeSource source, NoiseGeneratorSettings settings,
                         RandomState rs, String dimension, int cx, int cz, int minY, int height) throws Exception {
        Context ctx = context(world, resources, seed, source, settings, rs);
        var structures = world.lookupOrThrow(Registries.STRUCTURE);
        TreeMap<String, String> out = new TreeMap<>();
        for (Map.Entry<Structure, StructureStart> e : startsOf(world, ctx, dimension, cx, cz, minY, height).entrySet()) {
            out.put(structures.getKey(e.getKey()).toString(), canonical(e.getValue().createTag(ctx.serialization, new ChunkPos(cx, cz))));
        }
        if (out.isEmpty()) {
            return "none";
        }
        StringBuilder sb = new StringBuilder();
        out.forEach((k, v) -> sb.append(k).append('=').append(v).append(' '));
        return sb.toString().trim();
    }

    private record Ref(String id, int x, int z, StructureStart start) {
    }

    /**
     * Beardifier.forStructuresInChunk with the starts referenced by the chunk (starts within 8 chunks
     * whose box touches it), in a fixed order: structure id, then source chunk x, then z. Vanilla
     * iterates an identity hash map, so its own order is not reproducible.
     */
    static net.minecraft.world.level.levelgen.Beardifier beardifier(RegistryAccess.Frozen world, ResourceManager resources, long seed,
                                                                     BiomeSource source, NoiseGeneratorSettings settings, RandomState rs,
                                                                     String dimension, int cx, int cz, int minY, int height) throws Exception {
        Context ctx = context(world, resources, seed, source, settings, rs);
        var structures = world.lookupOrThrow(Registries.STRUCTURE);
        List<Ref> refs = new ArrayList<>();
        int bx = cx << 4;
        int bz = cz << 4;
        for (int sx = cx - 8; sx <= cx + 8; sx++) {
            for (int sz = cz - 8; sz <= cz + 8; sz++) {
                for (var e : startsOf(world, ctx, dimension, sx, sz, minY, height).entrySet()) {
                    StructureStart start = e.getValue();
                    if (start.isValid() && start.getBoundingBox().intersects(bx, bz, bx + 15, bz + 15)
                        && e.getKey().terrainAdaptation() != net.minecraft.world.level.levelgen.structure.TerrainAdjustment.NONE) {
                        refs.add(new Ref(structures.getKey(e.getKey()).toString(), sx, sz, start));
                    }
                }
            }
        }
        refs.sort(java.util.Comparator.comparing(Ref::id).thenComparingInt(Ref::x).thenComparingInt(Ref::z));
        List<net.minecraft.world.level.levelgen.Beardifier.Rigid> rigids = new ArrayList<>();
        List<net.minecraft.world.level.levelgen.structure.pools.JigsawJunction> junctions = new ArrayList<>();
        net.minecraft.world.level.levelgen.structure.BoundingBox any = null;
        var chunkPos = new ChunkPos(cx, cz);
        for (Ref ref : refs) {
            var adjustment = ref.start().getStructure().terrainAdaptation();
            for (var piece : ref.start().getPieces()) {
                if (!piece.isCloseToChunk(chunkPos, 12)) {
                    continue;
                }
                if (piece instanceof net.minecraft.world.level.levelgen.structure.PoolElementStructurePiece pool) {
                    if (pool.getElement().getProjection() == net.minecraft.world.level.levelgen.structure.pools.StructureTemplatePool.Projection.RIGID) {
                        rigids.add(new net.minecraft.world.level.levelgen.Beardifier.Rigid(pool.getBoundingBox(), adjustment, pool.getGroundLevelDelta()));
                        any = any == null ? piece.getBoundingBox() : net.minecraft.world.level.levelgen.structure.BoundingBox.encapsulating(any, piece.getBoundingBox());
                    }
                    for (var j : pool.getJunctions()) {
                        int jx = j.getSourceX();
                        int jz = j.getSourceZ();
                        if (jx > bx - 12 && jz > bz - 12 && jx < bx + 15 + 12 && jz < bz + 15 + 12) {
                            junctions.add(j);
                            var jb = new net.minecraft.world.level.levelgen.structure.BoundingBox(new net.minecraft.core.BlockPos(jx, j.getSourceGroundY(), jz));
                            any = any == null ? jb : net.minecraft.world.level.levelgen.structure.BoundingBox.encapsulating(any, jb);
                        }
                    }
                } else {
                    rigids.add(new net.minecraft.world.level.levelgen.Beardifier.Rigid(piece.getBoundingBox(), adjustment, 0));
                    any = any == null ? piece.getBoundingBox() : net.minecraft.world.level.levelgen.structure.BoundingBox.encapsulating(any, piece.getBoundingBox());
                }
            }
        }
        if (any == null) {
            return net.minecraft.world.level.levelgen.Beardifier.EMPTY;
        }
        return new net.minecraft.world.level.levelgen.Beardifier(List.copyOf(rigids), List.copyOf(junctions), any.inflatedBy(24));
    }

    /** Ring positions of a concentric rings structure set as `x,z` pairs. */
    static String rings(RegistryAccess.Frozen world, long seed, BiomeSource source, RandomState rs, String setId) {
        var state = ChunkGeneratorStructureState.createForNormal(rs, seed, ChunkPos.ZERO, source, world.lookupOrThrow(Registries.STRUCTURE_SET));
        var set = world.lookupOrThrow(Registries.STRUCTURE_SET).getOrThrow(ResourceKey.create(Registries.STRUCTURE_SET, Identifier.parse(setId)));
        var positions = state.getRingPositionsFor((net.minecraft.world.level.levelgen.structure.placement.ConcentricRingsStructurePlacement) set.value().placement());
        StringBuilder sb = new StringBuilder();
        for (ChunkPos p : positions) {
            sb.append(p.x()).append(',').append(p.z()).append(' ');
        }
        return sb.toString().trim();
    }

    /** Type-tagged, key-sorted NBT text; floats as raw bits so both sides compare exactly. */
    static String canonical(Tag tag) {
        StringBuilder sb = new StringBuilder();
        write(sb, tag);
        return sb.toString();
    }

    private static void write(StringBuilder sb, Tag tag) {
        switch (tag) {
            case CompoundTag c -> {
                List<String> keys = new ArrayList<>(c.keySet());
                java.util.Collections.sort(keys);
                sb.append('{');
                for (int i = 0; i < keys.size(); i++) {
                    if (i > 0) sb.append(',');
                    quote(sb, keys.get(i));
                    sb.append(':');
                    write(sb, c.get(keys.get(i)));
                }
                sb.append('}');
            }
            case ListTag l -> {
                sb.append('[');
                for (int i = 0; i < l.size(); i++) {
                    if (i > 0) sb.append(',');
                    write(sb, l.get(i));
                }
                sb.append(']');
            }
            case ByteTag b -> sb.append(b.value()).append('b');
            case ShortTag s -> sb.append(s.value()).append('s');
            case IntTag i -> sb.append(i.value());
            case LongTag l -> sb.append(l.value()).append('L');
            case FloatTag f -> sb.append("f#").append(Integer.toHexString(Float.floatToRawIntBits(f.value())));
            case DoubleTag d -> sb.append("d#").append(Long.toHexString(Double.doubleToRawLongBits(d.value())));
            case StringTag s -> quote(sb, s.value());
            case ByteArrayTag a -> {
                sb.append("[B;");
                byte[] v = a.getAsByteArray();
                for (int i = 0; i < v.length; i++) { if (i > 0) sb.append(','); sb.append(v[i]); }
                sb.append(']');
            }
            case IntArrayTag a -> {
                sb.append("[I;");
                int[] v = a.getAsIntArray();
                for (int i = 0; i < v.length; i++) { if (i > 0) sb.append(','); sb.append(v[i]); }
                sb.append(']');
            }
            case LongArrayTag a -> {
                sb.append("[L;");
                long[] v = a.getAsLongArray();
                for (int i = 0; i < v.length; i++) { if (i > 0) sb.append(','); sb.append(v[i]); }
                sb.append(']');
            }
            default -> sb.append("?").append(tag.getClass().getSimpleName());
        }
    }

    private static void quote(StringBuilder sb, String s) {
        sb.append('"');
        for (int i = 0; i < s.length(); i++) {
            char ch = s.charAt(i);
            if (ch == '"' || ch == '\\') sb.append('\\');
            sb.append(ch);
        }
        sb.append('"');
    }

    private Structures() {
    }
}
