package chunker.oracle;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.io.PrintStream;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executor;
import java.util.concurrent.Executors;

import net.minecraft.SharedConstants;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderGetter;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.LayeredRegistryAccess;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.RegistryDataLoader;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.Bootstrap;
import net.minecraft.server.RegistryLayer;
import net.minecraft.server.packs.PackType;
import net.minecraft.server.packs.repository.PackRepository;
import net.minecraft.server.packs.repository.ServerPacksSource;
import net.minecraft.server.packs.resources.CloseableResourceManager;
import net.minecraft.server.packs.resources.MultiPackResourceManager;
import net.minecraft.tags.TagLoader;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.densityfunction.DensityBuffer;
import net.minecraft.world.level.levelgen.densityfunction.DensityFunction;
import net.minecraft.world.level.levelgen.densityfunction.DensitySampler;
import net.minecraft.world.level.levelgen.densityfunction.DensityVolume;
import net.minecraft.world.level.levelgen.densityfunction.SamplerContext;
import net.minecraft.world.level.levelgen.synth.NormalNoise;

/**
 * chunker-worldgen's Java oracle: evaluates vanilla worldgen with the real server classes and prints
 * results in a line based text format (floats as raw int bits) for chunker-worldgen's golden tests.
 *
 * Reads commands from stdin, one per line:
 *   density <seed> <noise_settings> <function> <x> <y> <z>
 *   volume  <seed> <noise_settings> <function> <minX> <minY> <minZ> <sizeX> <sizeY> <sizeZ> [stepX stepY stepZ]
 * where <function> is a density function id or `router.<field>`.
 */
public final class Oracle {
    static RegistryAccess.Frozen world;
    static CloseableResourceManager resources;
    /** Static + world registries (piece serialization needs both). */
    static RegistryAccess.Frozen full;
    private static final Map<String, RandomState> STATES = new ConcurrentHashMap<>();

    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        loadRegistries();
        PrintStream out = new PrintStream(System.out, false, "UTF-8");
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in, "UTF-8"));
        String line;
        while ((line = in.readLine()) != null) {
            line = line.trim();
            if (line.isEmpty() || line.startsWith("#")) {
                continue;
            }
            try {
                out.println(handle(line.split("\\s+")));
            } catch (Throwable t) {
                StringBuilder trace = new StringBuilder();
                Throwable root = t;
                while (root.getCause() != null) root = root.getCause();
                var frames = root.getStackTrace();
                for (int i = 0; i < Math.min(12, frames.length); i++) trace.append(" at ").append(frames[i]);
                out.println("error " + t + (root != t ? " caused by " + root : "") + trace);
            }
            out.flush();
        }
        System.exit(0);
    }

    static Object staticField(Class<?> owner, String name) throws Exception {
        var f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f.get(null);
    }

    private static void loadRegistries() {
        PackRepository repo = ServerPacksSource.createVanillaTrustedRepository();
        repo.reload();
        repo.setSelected(List.of("vanilla"));
        resources = new MultiPackResourceManager(PackType.SERVER_DATA, repo.openAllSelected());
        LayeredRegistryAccess<RegistryLayer> layers = RegistryLayer.createRegistryAccess();
        List<Registry.PendingTags<?>> staticTags = TagLoader.loadTagsForExistingRegistries(resources, layers.getLayer(RegistryLayer.STATIC));
        RegistryAccess.Frozen loadContext = layers.getAccessForLoading(RegistryLayer.WORLD);
        List<HolderLookup.RegistryLookup<?>> lookups = TagLoader.buildUpdatedLookups(loadContext, staticTags);
        Executor executor = Executors.newFixedThreadPool(4);
        world = RegistryDataLoader.load(resources, lookups, RegistryDataLoader.WORLD_REGISTRIES, executor).join();
        staticTags.forEach(Registry.PendingTags::apply);
        full = layers.replaceFrom(RegistryLayer.WORLD, world).compositeAccess();
        // Item (and other) default components, bound by ReloadableServerResources on a real server.
        net.minecraft.core.registries.BuiltInRegistries.DATA_COMPONENT_INITIALIZERS.build(full)
            .forEach(net.minecraft.core.component.DataComponentInitializers.PendingComponents::apply);
    }

    static NoiseGeneratorSettings settings(String id) {
        return world.lookupOrThrow(Registries.NOISE_SETTINGS)
            .getOrThrow(ResourceKey.create(Registries.NOISE_SETTINGS, Identifier.parse(id)))
            .value();
    }

    static RandomState state(long seed, String settingsId) {
        return STATES.computeIfAbsent(seed + "|" + settingsId, k -> {
            HolderGetter<NormalNoise> noises = world.lookupOrThrow(Registries.NOISE);
            return RandomState.create(noises, seed, settings(settingsId));
        });
    }

    private static final Map<String, net.minecraft.world.level.biome.BiomeSource> SOURCES = new ConcurrentHashMap<>();

    /** Memoized so the structure context caches (keyed by source identity) are shared between commands. */
    static net.minecraft.world.level.biome.BiomeSource biomeSource(String name) {
        return SOURCES.computeIfAbsent(name, Oracle::createBiomeSource);
    }

    private static net.minecraft.world.level.biome.BiomeSource createBiomeSource(String name) {
        var biomes = world.lookupOrThrow(Registries.BIOME);
        if (name.equals("the_end")) {
            return net.minecraft.world.level.biome.TheEndBiomeSource.create(biomes);
        }
        var presets = world.lookupOrThrow(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST);
        return net.minecraft.world.level.biome.MultiNoiseBiomeSource.createFromPreset(
            presets.getOrThrow(ResourceKey.create(Registries.MULTI_NOISE_BIOME_SOURCE_PARAMETER_LIST, Identifier.withDefaultNamespace(name)))
        );
    }

    /** Runs the vanilla terrain step pieces on a fresh ProtoChunk (no structures, empty blender). */
    static net.minecraft.world.level.chunk.ProtoChunk buildTerrain(long seed, String sourceName, String settingsId, int cx, int cz, String stage, int dimMinY, int dimHeight)
        throws Exception {
        return buildTerrain(seed, sourceName, settingsId, cx, cz, stage, dimMinY, dimHeight, null);
    }

    /** A non-null dimension adds the beardifier of the structures around the chunk. */
    static net.minecraft.world.level.chunk.ProtoChunk buildTerrain(long seed, String sourceName, String settingsId, int cx, int cz, String stage, int dimMinY, int dimHeight,
                                                                           String dimension) throws Exception {
        NoiseGeneratorSettings ns = settings(settingsId);
        RandomState rs = state(seed, settingsId);
        var source = biomeSource(sourceName);
        var pos = new net.minecraft.world.level.ChunkPos(cx, cz);
        var heightAccessor = net.minecraft.world.level.LevelHeightAccessor.create(dimMinY, dimHeight);
        var clamped = ns.noiseSettings().clampToHeightAccessor(heightAccessor);
        int minY = dimMinY, height = dimHeight;
        var containers = net.minecraft.world.level.chunk.PalettedContainerFactory.create(world);
        var chunk = new net.minecraft.world.level.chunk.ProtoChunk(pos, net.minecraft.world.level.chunk.UpgradeData.EMPTY, heightAccessor, containers, null);
        var pool = rs.acquireDensityBufferPool();
        var climate = rs.createClimateSampler(SamplerContext.builder().enableCaches().useBufferArena(pool).build());
        chunk.fillBiomesFromNoise(source.createResolverForChunk(climate, cx << 2, minY >> 2, cz << 2, 4, height >> 2, 4));
        rs.releaseDensityBufferPool(pool);
        chunk.setPersistedStatus(net.minecraft.world.level.chunk.status.ChunkStatus.BIOMES);

        var gen = new net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator(source, Holder.direct(ns));
        var pickerMethod = net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator.class.getDeclaredMethod("createFluidPicker", NoiseGeneratorSettings.class);
        pickerMethod.setAccessible(true);
        var picker = (net.minecraft.world.level.levelgen.Aquifer.FluidPicker) pickerMethod.invoke(null, ns);
        var volume = new DensityVolume(16, clamped.height(), 16, pos.getMinBlockX(), clamped.minY(), pos.getMinBlockZ());
        try (var noiseChunk = new net.minecraft.world.level.levelgen.NoiseChunk(
            rs, dimension == null ? net.minecraft.world.level.levelgen.Beardifier.EMPTY
                : Structures.beardifier(full, resources, seed, source, ns, rs, dimension, cx, cz, dimMinY, dimHeight), ns, picker, net.minecraft.world.level.levelgen.blending.Blender.empty(), volume)) {
            var doFill = net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator.class.getDeclaredMethod(
                "doFill", net.minecraft.world.level.levelgen.NoiseChunk.class, net.minecraft.world.level.chunk.ChunkAccess.class);
            doFill.setAccessible(true);
            doFill.invoke(gen, noiseChunk, chunk);
            if (stage.equals("surface") || stage.equals("carvers")) {
                var neighbors = new java.util.HashMap<Long, net.minecraft.world.level.chunk.ProtoChunk>();
                for (int dz = -1; dz <= 1; dz++) {
                    for (int dx = -1; dx <= 1; dx++) {
                        if (dx == 0 && dz == 0) {
                            neighbors.put(net.minecraft.world.level.ChunkPos.pack(cx, cz), chunk);
                            continue;
                        }
                        var np = new net.minecraft.world.level.ChunkPos(cx + dx, cz + dz);
                        var n = new net.minecraft.world.level.chunk.ProtoChunk(np, net.minecraft.world.level.chunk.UpgradeData.EMPTY, heightAccessor, containers, null);
                        var npool = rs.acquireDensityBufferPool();
                        var nclimate = rs.createClimateSampler(SamplerContext.builder().enableCaches().useBufferArena(npool).build());
                        n.fillBiomesFromNoise(source.createResolverForChunk(nclimate, (cx + dx) << 2, minY >> 2, (cz + dz) << 2, 4, height >> 2, 4));
                        rs.releaseDensityBufferPool(npool);
                        n.setPersistedStatus(net.minecraft.world.level.chunk.status.ChunkStatus.BIOMES);
                        neighbors.put(net.minecraft.world.level.ChunkPos.pack(cx + dx, cz + dz), n);
                    }
                }
                var uncached = source.createUncachedResolver(rs);
                net.minecraft.world.level.biome.BiomeResolver regionBiomes = (qx, qy, qz) -> {
                    var c = neighbors.get(net.minecraft.world.level.ChunkPos.pack(qx >> 2, qz >> 2));
                    return c != null ? c.getNoiseBiome(qx, qy, qz) : uncached.getNoiseBiome(qx, qy, qz);
                };
                var biomeManager = new net.minecraft.world.level.biome.BiomeManager(regionBiomes, net.minecraft.world.level.biome.BiomeManager.obfuscateSeed(seed));
                java.util.Set<Holder<net.minecraft.world.level.biome.Biome>> possible = new java.util.HashSet<>();
                for (var n : neighbors.values()) {
                    n.collectBiomesInPalette(possible);
                }
                var genContext = new net.minecraft.world.level.levelgen.WorldGenerationContext(gen, chunk.getHeightAccessorForGeneration());
                rs.surfaceSystem().buildSurface(rs, biomeManager, genContext, chunk, noiseChunk, ns.materialRule().value(), possible);
                if (stage.equals("carvers")) {
                    var carvers = net.minecraft.world.level.levelgen.NoiseBasedChunkGenerator.class.getDeclaredMethod(
                        "generateCarvers",
                        net.minecraft.world.level.chunk.ChunkAccess.class,
                        net.minecraft.world.level.levelgen.blending.Blender.class,
                        net.minecraft.world.level.levelgen.NoiseChunk.class,
                        RandomState.class,
                        net.minecraft.world.level.biome.BiomeManager.class,
                        net.minecraft.server.level.WorldGenRegion.class,
                        net.minecraft.world.level.levelgen.material.rule.MaterialRule.class
                    );
                    carvers.setAccessible(true);
                    carvers.invoke(gen, chunk, net.minecraft.world.level.levelgen.blending.Blender.empty(), noiseChunk, rs, biomeManager, null, ns.materialRule().value());
                    net.minecraft.world.level.levelgen.Heightmap.primeHeightmaps(chunk, java.util.EnumSet.of(
                        net.minecraft.world.level.levelgen.Heightmap.Types.MOTION_BLOCKING,
                        net.minecraft.world.level.levelgen.Heightmap.Types.MOTION_BLOCKING_NO_LEAVES,
                        net.minecraft.world.level.levelgen.Heightmap.Types.OCEAN_FLOOR,
                        net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE));
                }
            }
        }
        return chunk;
    }

    static String describeChunk(net.minecraft.world.level.chunk.ProtoChunk chunk) {
        StringBuilder sb = new StringBuilder();
        var registry = net.minecraft.world.level.block.Block.BLOCK_STATE_REGISTRY;
        long all = 1125899906842597L;
        StringBuilder sections = new StringBuilder();
        var sectionArray = chunk.getSections();
        for (int si = 0; si < sectionArray.length; si++) {
            long h = 1125899906842597L;
            for (int y = 0; y < 16; y++) {
                for (int z = 0; z < 16; z++) {
                    for (int x = 0; x < 16; x++) {
                        int id = registry.getId(sectionArray[si].getBlockState(x, y, z));
                        h = 31 * h + id;
                        all = 31 * all + id;
                    }
                }
            }
            sections.append(' ').append(h);
        }
        sb.append(all);
        for (var type : net.minecraft.world.level.levelgen.Heightmap.Types.values()) {
            long h = 1125899906842597L;
            boolean present = false;
            for (var e : chunk.getHeightmaps()) {
                if (e.getKey() == type) {
                    present = true;
                    for (int z = 0; z < 16; z++) {
                        for (int x = 0; x < 16; x++) {
                            h = 31 * h + e.getValue().getFirstAvailable(x, z);
                        }
                    }
                }
            }
            sb.append(' ').append(type.getSerializationKey()).append('=').append(present ? Long.toString(h) : "-");
        }
        long pp = 1125899906842597L;
        int count = 0;
        for (var list : chunk.getPostProcessing()) {
            if (list != null) {
                for (int i = 0; i < list.size(); i++) {
                    pp = 31 * pp + list.getShort(i);
                    count++;
                }
            }
            pp = 31 * pp + 7;
        }
        sb.append(" post=").append(count).append(':').append(pp);
        sb.append(" |").append(sections);
        return sb.toString();
    }

    private static DensityFunction function(String settingsId, String name) {
        if (name.startsWith("router.")) {
            var router = settings(settingsId).noiseRouter();
            return switch (name.substring(7)) {
                case "temperature" -> router.temperature();
                case "vegetation" -> router.vegetation();
                case "continents" -> router.continents();
                case "erosion" -> router.erosion();
                case "depth" -> router.depth();
                case "ridges" -> router.ridges();
                case "chunk_surface_level" -> router.chunkSurfaceLevel();
                case "final_density" -> router.finalDensity();
                default -> throw new IllegalArgumentException("unknown router field " + name);
            };
        }
        Holder<DensityFunction> holder = world.lookupOrThrow(Registries.DENSITY_FUNCTION)
            .getOrThrow(ResourceKey.create(Registries.DENSITY_FUNCTION, Identifier.parse(name)));
        return new net.minecraft.world.level.levelgen.densityfunction.DensityFunctions.HolderHolder(holder);
    }

    private static String handle(String[] a) throws Exception {
        switch (a[0]) {
            case "density": {
                long seed = Long.parseLong(a[1]);
                RandomState rs = state(seed, a[2]);
                DensitySampler sampler = rs.getSampler(function(a[2], a[3]));
                int x = Integer.parseInt(a[4]), y = Integer.parseInt(a[5]), z = Integer.parseInt(a[6]);
                float uncached = sampler.sampleValue(SamplerContext.EMPTY_UNCACHED, x, y, z);
                float cached = sampler.sampleValue(SamplerContext.builder().enableCaches().build(), x, y, z);
                return "ok " + Float.floatToRawIntBits(uncached) + " " + Float.floatToRawIntBits(cached);
            }
            case "volume": {
                long seed = Long.parseLong(a[1]);
                RandomState rs = state(seed, a[2]);
                DensitySampler sampler = rs.getSampler(function(a[2], a[3]));
                int[] v = new int[a.length - 4];
                for (int i = 0; i < v.length; i++) {
                    v[i] = Integer.parseInt(a[4 + i]);
                }
                DensityVolume volume = v.length >= 9
                    ? new DensityVolume(v[3], v[4], v[5], v[0], v[1], v[2], v[6], v[7], v[8])
                    : new DensityVolume(v[3], v[4], v[5], v[0], v[1], v[2]);
                DensityBuffer buffer = DensityBuffer.createUnpooled(volume.size());
                sampler.sampleVolume(SamplerContext.builder().enableCaches().build(), buffer, volume);
                StringBuilder sb = new StringBuilder("ok");
                long hash = 1125899906842597L;
                for (int i = 0; i < volume.size(); i++) {
                    hash = 31 * hash + Float.floatToRawIntBits(buffer.get(i));
                }
                sb.append(' ').append(hash);
                int samples = Math.min(volume.size(), 8);
                for (int i = 0; i < samples; i++) {
                    int idx = (int) ((long) i * (volume.size() - 1) / Math.max(1, samples - 1));
                    sb.append(' ').append(idx).append(':').append(Float.floatToRawIntBits(buffer.get(idx)));
                }
                return sb.toString();
            }
            case "biomes": {
                // biomes <seed> <source:overworld|nether|the_end> <noise_settings> <chunkX> <chunkZ>
                long seed = Long.parseLong(a[1]);
                NoiseGeneratorSettings ns = settings(a[3]);
                RandomState rs = state(seed, a[3]);
                net.minecraft.world.level.biome.BiomeSource source = biomeSource(a[2]);
                int chunkX = Integer.parseInt(a[4]), chunkZ = Integer.parseInt(a[5]);
                int minY = ns.noiseSettings().minY(), height = ns.noiseSettings().height();
                var pool = rs.acquireDensityBufferPool();
                var climate = rs.createClimateSampler(SamplerContext.builder().enableCaches().useBufferArena(pool).build());
                int minQx = chunkX << 2, minQz = chunkZ << 2, minQy = minY >> 2;
                var resolver = source.createResolverForChunk(climate, minQx, minQy, minQz, 4, height >> 2, 4);
                StringBuilder sb = new StringBuilder("ok");
                String last = null;
                int run = 0;
                for (int section = 0; section < (height >> 4); section++) {
                    for (int x = 0; x < 4; x++) {
                        for (int y = 0; y < 4; y++) {
                            for (int z = 0; z < 4; z++) {
                                String name = resolver.getNoiseBiome(minQx + x, minQy + section * 4 + y, minQz + z)
                                    .unwrapKey().orElseThrow().identifier().getPath();
                                if (name.equals(last)) {
                                    run++;
                                } else {
                                    if (last != null) {
                                        sb.append(' ').append(last).append('*').append(run);
                                    }
                                    last = name;
                                    run = 1;
                                }
                            }
                        }
                    }
                }
                sb.append(' ').append(last).append('*').append(run);
                rs.releaseDensityBufferPool(pool);
                return sb.toString();
            }
            case "terrain": {
                // terrain <seed> <source> <noise_settings> <chunkX> <chunkZ> <stage:fill|surface> <dimMinY> <dimHeight>
                long seed = Long.parseLong(a[1]);
                // optional 10th argument: dimension id, adds the beardifier of the surrounding structures
                var chunk = buildTerrain(seed, a[2], a[3], Integer.parseInt(a[4]), Integer.parseInt(a[5]), a[6], Integer.parseInt(a[7]), Integer.parseInt(a[8]),
                    a.length > 9 ? a[9] : null);
                return "ok " + describeChunk(chunk);
            }
            case "blockcolors": {
                // Map color (0xRRGGBB, 0 = none) per block state id.
                StringBuilder sb = new StringBuilder("ok");
                var registry = net.minecraft.world.level.block.Block.BLOCK_STATE_REGISTRY;
                for (int id = 0; id < registry.size(); id++) {
                    var state = registry.byId(id);
                    var color = state.getMapColor(net.minecraft.world.level.EmptyBlockGetter.INSTANCE, net.minecraft.core.BlockPos.ZERO);
                    sb.append(' ').append(Integer.toHexString(color == null ? 0 : color.col));
                }
                return sb.toString();
            }
            case "blockflags": {
                // One hex word per block state id, see BlockFlags in chunker-data.
                StringBuilder sb = new StringBuilder("ok");
                var registry = net.minecraft.world.level.block.Block.BLOCK_STATE_REGISTRY;
                for (int id = 0; id < registry.size(); id++) {
                    var state = registry.byId(id);
                    var fluid = state.getFluidState();
                    long flags = 0;
                    if (state.isAir()) flags |= 1;
                    if (!fluid.isEmpty()) flags |= 1 << 1;
                    if (fluid.is(net.minecraft.tags.FluidTags.WATER)) flags |= 1 << 2;
                    if (fluid.is(net.minecraft.tags.FluidTags.LAVA)) flags |= 1 << 3;
                    if (fluid.isSource()) flags |= 1 << 4;
                    if (state.liquid()) flags |= 1 << 5;
                    if (state.isSolid()) flags |= 1 << 6;
                    if (state.canOcclude()) flags |= 1 << 7;
                    if (state.canBeReplaced()) flags |= 1 << 8;
                    if (state.isRandomlyTicking()) flags |= 1 << 9;
                    if (state.hasBlockEntity()) flags |= 1 << 10;
                    if (state.isSolidRender()) flags |= 1 << 11;
                    if (state.propagatesSkylightDown()) flags |= 1L << 24;
                    // StructureTemplate.addToLists: static full collision shape.
                    if (!state.getBlock().hasDynamicShape()
                        && state.isCollisionShapeFullBlock(net.minecraft.world.level.EmptyBlockGetter.INSTANCE, net.minecraft.core.BlockPos.ZERO)) flags |= 1L << 25;
                    if (state.isFaceSturdy(net.minecraft.world.level.EmptyBlockGetter.INSTANCE, net.minecraft.core.BlockPos.ZERO, net.minecraft.core.Direction.UP)) flags |= 1L << 26;
                    if (state.isFaceSturdy(net.minecraft.world.level.EmptyBlockGetter.INSTANCE, net.minecraft.core.BlockPos.ZERO, net.minecraft.core.Direction.DOWN)) flags |= 1L << 27;
                    var empty = net.minecraft.world.level.EmptyBlockGetter.INSTANCE;
                    var zero = net.minecraft.core.BlockPos.ZERO;
                    var dirs = net.minecraft.core.Direction.values();
                    for (int d = 0; d < 6; d++) {
                        if (state.isFaceSturdy(empty, zero, dirs[d])) flags |= 1L << (28 + d);
                    }
                    if (state.isFaceSturdy(empty, zero, net.minecraft.core.Direction.UP, net.minecraft.world.level.block.SupportType.CENTER)) flags |= 1L << 34;
                    if (state.isFaceSturdy(empty, zero, net.minecraft.core.Direction.UP, net.minecraft.world.level.block.SupportType.RIGID)) flags |= 1L << 35;
                    if (state.isCollisionShapeFullBlock(empty, zero)) flags |= 1L << 36;
                    if (state.isRedstoneConductor(empty, zero)) flags |= 1L << 37;
                    if (state.isSuffocating(empty, zero)) flags |= 1L << 38;
                    var ft = fluid.getType();
                    long fluidType = ft == net.minecraft.world.level.material.Fluids.WATER ? 1 : ft == net.minecraft.world.level.material.Fluids.FLOWING_WATER ? 2
                        : ft == net.minecraft.world.level.material.Fluids.LAVA ? 3 : ft == net.minecraft.world.level.material.Fluids.FLOWING_LAVA ? 4 : 0;
                    flags |= fluidType << 39;
                    if (!fluid.isEmpty() && fluid.getValue(net.minecraft.world.level.material.FlowingFluid.FALLING)) flags |= 1L << 42;
                    if (state.getCollisionShape(empty, zero).isEmpty()) flags |= 1L << 43;
                    if (net.minecraft.world.level.block.Block.isFaceFull(state.getCollisionShape(empty, zero), net.minecraft.core.Direction.UP)) flags |= 1L << 44;
                    for (int d = 0; d < 6; d++) {
                        if (net.minecraft.world.level.block.Block.isShapeFullBlock(state.getFaceOcclusionShape(dirs[d]))) flags |= 1L << (45 + d);
                    }
                    for (int d = 0; d < 6; d++) {
                        if (net.minecraft.world.level.block.Block.isFaceFull(state.getCollisionShape(empty, zero), dirs[d])) flags |= 1L << (51 + d);
                    }
                    if (!state.getCollisionShape(empty, zero).getFaceShape(net.minecraft.core.Direction.UP).isEmpty()) flags |= 1L << 57;
                    if (state.getBlock() instanceof net.minecraft.world.level.block.LiquidBlockContainer) flags |= 1L << 58;
                    if (state.getBlock() instanceof net.minecraft.world.level.block.SimpleWaterloggedBlock) flags |= 1L << 59;
                    if (net.minecraft.world.level.block.Block.isShapeFullBlock(state.getShape(empty, zero))) flags |= 1L << 60;
                    if (state.isFaceSturdy(empty, zero, net.minecraft.core.Direction.DOWN, net.minecraft.world.level.block.SupportType.CENTER)) flags |= 1L << 61;
                    flags |= (long) (fluid.getAmount() & 15) << 12;
                    flags |= (long) (state.getLightEmission() & 15) << 16;
                    flags |= (long) (state.getLightDampening() & 15) << 20;
                    sb.append(' ').append(Long.toHexString(flags));
                }
                return sb.toString();
            }
            case "structures": {
                // structures <seed> <source> <noise_settings> <dimension> <chunkX> <chunkZ> <dimMinY> <dimHeight>
                return "ok " + Structures.starts(full, resources, Long.parseLong(a[1]), biomeSource(a[2]), settings(a[3]), state(Long.parseLong(a[1]), a[3]),
                    a[4], Integer.parseInt(a[5]), Integer.parseInt(a[6]), Integer.parseInt(a[7]), Integer.parseInt(a[8]));
            }
            case "beardkernel": {
                var f = net.minecraft.world.level.levelgen.Beardifier.class.getDeclaredField("BEARD_KERNEL");
                f.setAccessible(true);
                float[] kernel = (float[]) f.get(null);
                StringBuilder sb = new StringBuilder("ok");
                for (float v : kernel) {
                    sb.append(' ').append(Integer.toHexString(Float.floatToRawIntBits(v)));
                }
                return sb.toString();
            }
            case "rings": {
                // rings <seed> <source> <noise_settings> <structure_set>
                return "ok " + Structures.rings(world, Long.parseLong(a[1]), biomeSource(a[2]), state(Long.parseLong(a[1]), a[3]), a[4]);
            }
            case "features":
                return "ok " + Features.run(a);
            case "itemtest":
                return "ok " + Features.itemTest(a[1]);
            case "spawnsettings":
                return "ok " + Features.spawnSettings(a);
            case "normentity":
                return "ok " + Features.normEntity(a[1]);
            case "entityattributes": {
                // Per entity type with default attributes: id|attribute=raw double bits,...
                StringBuilder sb = new StringBuilder("ok");
                for (var type : net.minecraft.core.registries.BuiltInRegistries.ENTITY_TYPE) {
                    @SuppressWarnings("unchecked")
                    var living = (net.minecraft.world.entity.EntityType<? extends net.minecraft.world.entity.LivingEntity>) type;
                    if (!net.minecraft.world.entity.ai.attributes.DefaultAttributes.hasSupplier(type)) continue;
                    var supplier = net.minecraft.world.entity.ai.attributes.DefaultAttributes.getSupplier(living);
                    sb.append(' ').append(net.minecraft.core.registries.BuiltInRegistries.ENTITY_TYPE.getKey(type)).append('|');
                    boolean first = true;
                    for (var attribute : net.minecraft.core.registries.BuiltInRegistries.ATTRIBUTE.listElements().toList()) {
                        if (!supplier.hasAttribute(attribute)) continue;
                        if (!first) sb.append(',');
                        first = false;
                        sb.append(attribute.key().identifier()).append('=').append(Long.toHexString(Double.doubleToRawLongBits(supplier.getBaseValue(attribute))));
                    }
                }
                return sb.toString();
            }
            case "itemenchantable": {
                // Items with an `enchantable` component: id=value.
                StringBuilder sb = new StringBuilder("ok");
                for (var item : net.minecraft.core.registries.BuiltInRegistries.ITEM) {
                    var enchantable = item.components().get(net.minecraft.core.component.DataComponents.ENCHANTABLE);
                    if (enchantable != null) {
                        sb.append(' ').append(net.minecraft.core.registries.BuiltInRegistries.ITEM.getKey(item)).append('=').append(enchantable.value());
                    }
                }
                return sb.toString();
            }
            case "entitytest":
                return "ok " + Features.entityTest(String.join(" ", java.util.Arrays.copyOfRange(a, 1, a.length)));
            case "featuresection":
                return "ok " + Features.section(a);
            case "featureorder":
                return "ok " + Features.order(a);
            case "blockentities": {
                // blockentities <file>: gzip NBT {block name: default block entity saveWithFullMetadata at 0,0,0}.
                var root = new net.minecraft.nbt.CompoundTag();
                var loaded = new net.minecraft.nbt.CompoundTag();
                StringBuilder failed = new StringBuilder();
                for (var block : net.minecraft.core.registries.BuiltInRegistries.BLOCK) {
                    if (block instanceof net.minecraft.world.level.block.EntityBlock eb) {
                        String name = net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(block).getPath();
                        try {
                            var be = eb.newBlockEntity(net.minecraft.core.BlockPos.ZERO, block.defaultBlockState());
                            if (be != null) {
                                root.put(name, be.saveWithFullMetadata(full));
                                // The same block entity loaded from empty NBT: the base of template loading.
                                var fresh = eb.newBlockEntity(net.minecraft.core.BlockPos.ZERO, block.defaultBlockState());
                                fresh.loadWithComponents(net.minecraft.world.level.storage.TagValueInput.create(
                                    net.minecraft.util.ProblemReporter.DISCARDING, full, new net.minecraft.nbt.CompoundTag()));
                                loaded.put(name, fresh.saveWithFullMetadata(full));
                            }
                        } catch (Throwable t) {
                            Throwable c = t;
                            while (c.getCause() != null) c = c.getCause();
                            failed.append(' ').append(name).append(':').append(c);
                        }
                    }
                }
                // Vault configs need bound item components, which this harness doesn't set up.
                var both = new net.minecraft.nbt.CompoundTag();
                both.put("fresh", root);
                both.put("loaded", loaded);
                net.minecraft.nbt.NbtIo.writeCompressed(both, java.nio.file.Path.of(a[1]));
                return "ok " + root.size() + " (skipped:" + failed + ")";
            }
            case "blockshapes": {
                // Per state, hex bits of shape tests on its collision face DOWN (as the block above a wall):
                // 0 = WallBlock.TEST_SHAPE_POST covered, 1-4 = TEST_SHAPES_WALL north, east, south, west covered.
                var post = (net.minecraft.world.phys.shapes.VoxelShape) staticField(net.minecraft.world.level.block.WallBlock.class, "TEST_SHAPE_POST");
                @SuppressWarnings("unchecked")
                var walls = (java.util.Map<net.minecraft.core.Direction, net.minecraft.world.phys.shapes.VoxelShape>) staticField(
                    net.minecraft.world.level.block.WallBlock.class, "TEST_SHAPES_WALL");
                var dirs = new net.minecraft.core.Direction[] { net.minecraft.core.Direction.NORTH, net.minecraft.core.Direction.EAST,
                    net.minecraft.core.Direction.SOUTH, net.minecraft.core.Direction.WEST };
                StringBuilder sb = new StringBuilder("ok");
                var registry = net.minecraft.world.level.block.Block.BLOCK_STATE_REGISTRY;
                for (int id = 0; id < registry.size(); id++) {
                    var state = registry.byId(id);
                    var face = state.getCollisionShape(net.minecraft.world.level.EmptyBlockGetter.INSTANCE, net.minecraft.core.BlockPos.ZERO)
                        .getFaceShape(net.minecraft.core.Direction.DOWN);
                    int bits = 0;
                    if (!net.minecraft.world.phys.shapes.Shapes.joinIsNotEmpty(post, face, net.minecraft.world.phys.shapes.BooleanOp.ONLY_FIRST)) bits |= 1;
                    for (int d = 0; d < 4; d++) {
                        if (!net.minecraft.world.phys.shapes.Shapes.joinIsNotEmpty(walls.get(dirs[d]), face, net.minecraft.world.phys.shapes.BooleanOp.ONLY_FIRST)) bits |= 2 << d;
                    }
                    sb.append(' ').append(Integer.toHexString(bits));
                }
                return sb.toString();
            }
            case "blocktransforms": {
                // Per state whose rotation or mirror differs: id:cw90,cw180,ccw90,left_right,front_back.
                StringBuilder sb = new StringBuilder("ok");
                var registry = net.minecraft.world.level.block.Block.BLOCK_STATE_REGISTRY;
                var rots = new net.minecraft.world.level.block.Rotation[] {
                    net.minecraft.world.level.block.Rotation.CLOCKWISE_90, net.minecraft.world.level.block.Rotation.CLOCKWISE_180,
                    net.minecraft.world.level.block.Rotation.COUNTERCLOCKWISE_90 };
                var mirrors = new net.minecraft.world.level.block.Mirror[] {
                    net.minecraft.world.level.block.Mirror.LEFT_RIGHT, net.minecraft.world.level.block.Mirror.FRONT_BACK };
                for (int id = 0; id < registry.size(); id++) {
                    var state = registry.byId(id);
                    int[] out = new int[5];
                    boolean differs = false;
                    for (int i = 0; i < 3; i++) out[i] = registry.getId(state.rotate(rots[i]));
                    for (int i = 0; i < 2; i++) out[3 + i] = registry.getId(state.mirror(mirrors[i]));
                    for (int v : out) differs |= v != id;
                    if (differs) {
                        sb.append(' ').append(id).append(':').append(out[0]).append(',').append(out[1]).append(',').append(out[2])
                            .append(',').append(out[3]).append(',').append(out[4]);
                    }
                }
                return sb.toString();
            }
            case "blockclasses": {
                // Per block in registry order: the class chain up to Block (simple names).
                StringBuilder sb = new StringBuilder("ok");
                for (var block : net.minecraft.core.registries.BuiltInRegistries.BLOCK) {
                    StringBuilder chain = new StringBuilder();
                    for (Class<?> c = block.getClass(); c != null && c != net.minecraft.world.level.block.state.BlockBehaviour.class; c = c.getSuperclass()) {
                        if (chain.length() > 0) chain.append('>');
                        chain.append(c.getSimpleName());
                    }
                    sb.append(' ').append(net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(block).getPath()).append('=').append(chain);
                }
                return sb.toString();
            }
            default:
                throw new IllegalArgumentException("unknown command " + a[0]);
        }
    }

    private Oracle() {
    }
}
