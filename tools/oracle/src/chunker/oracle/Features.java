package chunker.oracle;

import java.lang.reflect.Field;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.atomic.AtomicReferenceArray;
import java.util.stream.Collectors;

import it.unimi.dsi.fastutil.ints.IntArraySet;
import it.unimi.dsi.fastutil.ints.IntSet;
import it.unimi.dsi.fastutil.objects.ObjectArraySet;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderSet;
import net.minecraft.core.Registry;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.SectionPos;
import net.minecraft.core.registries.Registries;
import net.minecraft.resources.Identifier;
import net.minecraft.resources.ResourceKey;
import net.minecraft.server.level.GenerationChunkHolder;
import net.minecraft.server.level.ServerChunkCache;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.WorldGenRegion;
import net.minecraft.server.packs.resources.ResourceManager;
import net.minecraft.util.StaticCache2D;
import net.minecraft.world.flag.FeatureFlagSet;
import net.minecraft.world.flag.FeatureFlags;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.StructureManager;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeGenerationSettings;
import net.minecraft.world.level.biome.BiomeResolver;
import net.minecraft.world.level.biome.BiomeSource;
import net.minecraft.world.level.biome.FeatureSorter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.server.level.ChunkResult;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.LightChunk;
import net.minecraft.world.level.chunk.LightChunkGetter;
import net.minecraft.world.level.chunk.PalettedContainerFactory;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.chunk.status.ChunkPyramid;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.chunk.status.ChunkStep;
import net.minecraft.world.level.dimension.DimensionType;
import net.minecraft.world.level.levelgen.GenerationStep;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.RandomSupport;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.WorldgenRandom;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.placement.FeaturePlacer;
import net.minecraft.world.level.levelgen.placement.PlacedFeature;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.Structure;
import net.minecraft.world.level.levelgen.structure.StructureStart;
import net.minecraft.world.level.lighting.LevelLightEngine;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.dedicated.DedicatedServer;

/**
 * The `features` step of one chunk on vanilla classes: a real {@link WorldGenRegion} over 3x3 terrain chunks
 * (plus structure-start-only chunks out to 8), built on a constructor-less fake {@link ServerLevel}.
 * Light data is absent, like for a vanilla chunk that has not reached `initialize_light`.
 */
final class Features {
    private static final sun.misc.Unsafe UNSAFE;

    static {
        try {
            Field f = sun.misc.Unsafe.class.getDeclaredField("theUnsafe");
            f.setAccessible(true);
            UNSAFE = (sun.misc.Unsafe) f.get(null);
        } catch (Exception e) {
            throw new RuntimeException(e);
        }
    }

    @SuppressWarnings("unchecked")
    private static <T> T allocate(Class<T> c) {
        try {
            return (T) UNSAFE.allocateInstance(c);
        } catch (InstantiationException e) {
            throw new RuntimeException(e);
        }
    }

    private static void setField(Object target, Class<?> owner, String name, Object value) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        f.set(target, value);
    }

    private static Object getField(Object target, Class<?> owner, String name) throws Exception {
        Field f = owner.getDeclaredField(name);
        f.setAccessible(true);
        return f.get(target);
    }

    /** Never constructed normally: {@link #allocate} skips the constructor, the overrides only read our fields. */
    static final class FakeChunkSource extends ServerChunkCache {
        RandomState randomState;
        ChunkGenerator generator;

        private FakeChunkSource() {
            super(null, null, null, null, null, null, 0, 0, false, null);
        }

        @Override
        public RandomState randomState() {
            return this.randomState;
        }

        @Override
        public ChunkGenerator getGenerator() {
            return this.generator;
        }
    }

    static final class FakeLevel extends ServerLevel {
        long seed;
        RegistryAccess access;
        DimensionType dimensionType;
        FakeChunkSource source;
        LevelLightEngine light;
        BiomeResolver uncached;
        MinecraftServer server;
        int seaLevel;
        int minY;
        int height;

        private FakeLevel() {
            super(null, null, null, null, null, null, false, 0L, null, false);
        }

        @Override
        public long getSeed() {
            return this.seed;
        }

        @Override
        public ServerChunkCache getChunkSource() {
            return this.source;
        }

        @Override
        public RegistryAccess registryAccess() {
            return this.access;
        }

        @Override
        public DimensionType dimensionType() {
            return this.dimensionType;
        }

        @Override
        public BiomeResolver uncachedBiomeResolver() {
            return this.uncached;
        }

        @Override
        public LevelLightEngine getLightEngine() {
            return this.light;
        }

        @Override
        public MinecraftServer getServer() {
            return this.server;
        }

        @Override
        public FeatureFlagSet enabledFeatures() {
            return FeatureFlags.DEFAULT_FLAGS;
        }

        @Override
        public int getSeaLevel() {
            return this.seaLevel;
        }

        int nextEntityId;
        StructureManager structures;

        @Override
        public StructureManager structureManager() {
            return this.structures;
        }

        @Override
        public int getNextEntityId() {
            return ++this.nextEntityId;
        }

        @Override
        public int getMinY() {
            return this.minY;
        }

        @Override
        public int getHeight() {
            return this.height;
        }

        @Override
        public void updatePOIOnBlockStateChange(final net.minecraft.core.BlockPos pos, final net.minecraft.world.level.block.state.BlockState oldState,
                                                final net.minecraft.world.level.block.state.BlockState newState) {
        }
    }

    static final class FakeHolder extends GenerationChunkHolder {
        FakeHolder(ChunkPos pos, ChunkAccess chunk, ChunkStatus status) throws Exception {
            super(pos);
            @SuppressWarnings("unchecked")
            var futures = (AtomicReferenceArray<CompletableFuture<ChunkResult<ChunkAccess>>>) getField(this, GenerationChunkHolder.class, "futures");
            for (int i = 0; i <= status.getIndex(); i++) {
                futures.set(i, CompletableFuture.completedFuture(ChunkResult.of(chunk)));
            }
        }

        @Override
        protected void addSaveDependency(final CompletableFuture<?> sync) {
        }

        @Override
        public int getTicketLevel() {
            return 0;
        }

        @Override
        public int getQueueLevel() {
            return 0;
        }
    }

    record Result(String key, List<ProtoChunk> chunks, List<String> log) {
    }

    /** Records every `setBlock` call (position and state id) in order. */
    static final class LoggingRegion extends WorldGenRegion {
        final List<String> log = new ArrayList<>();

        LoggingRegion(final ServerLevel level, final StaticCache2D<GenerationChunkHolder> cache, final ChunkStep step, final ChunkAccess center) {
            super(level, cache, step, center);
        }

        /**
         * `WorldGenRegion.getCurrentDifficultyAt` with the `spawnsettings` values. The fake level has no clocks:
         * the moon follows the overworld moon timeline (one phase per 24000 ticks) in dimensions that have it.
         */
        @Override
        public net.minecraft.world.DifficultyInstance getCurrentDifficultyAt(final net.minecraft.core.BlockPos pos) {
            boolean moon = this.getLevel().dimension() == net.minecraft.world.level.Level.OVERWORLD;
            float brightness = moon ? DimensionType.MOON_BRIGHTNESS_PER_PHASE[(int) (clockTime / 24000L % 8L)] : 1.0F;
            return new net.minecraft.world.DifficultyInstance(difficulty, clockTime, 0L, brightness);
        }

        @Override
        public boolean setBlock(final net.minecraft.core.BlockPos pos, final net.minecraft.world.level.block.state.BlockState state, final int flags,
                                final int limit) {
            this.log.add(pos.getX() + "," + pos.getY() + "," + pos.getZ() + "=" + Block.BLOCK_STATE_REGISTRY.getId(state));
            return super.setBlock(pos, state, flags, limit);
        }
    }

    private static Result last;
    static WorldGenRegion lastRegion;

    /** `spawnsettings <difficulty> <overworld clock ticks> <game time>`: the world state seen by `finalizeSpawn`. */
    static net.minecraft.world.Difficulty difficulty = net.minecraft.world.Difficulty.NORMAL;
    static long clockTime;
    static long gameTime;

    static String spawnSettings(String[] a) {
        difficulty = net.minecraft.world.Difficulty.byName(a[1]);
        clockTime = Long.parseLong(a[2]);
        gameTime = Long.parseLong(a[3]);
        last = null;
        return difficulty.getSerializedName() + " " + clockTime + " " + gameTime;
    }

    /**
     * Canonical NBT of an entity after a vanilla load and save (so both sides get the same defaults), without
     * UUIDs, with attributes and modifiers sorted by id. Values zombies draw from their own unseeded random
     * (`Zombie.handleAttributes`) are masked: vanilla itself never reproduces them.
     */
    static String normalizeEntity(net.minecraft.nbt.CompoundTag tag) {
        var level = lastRegion;
        try (var reporter = new net.minecraft.util.ProblemReporter.ScopedCollector(org.slf4j.LoggerFactory.getLogger("oracle"))) {
            var entity = net.minecraft.world.entity.EntityType.loadEntityRecursive(
                net.minecraft.world.level.storage.TagValueInput.create(reporter, level.registryAccess(), tag), level.getLevel(),
                net.minecraft.world.entity.EntitySpawnReason.LOAD, e -> e);
            if (entity == null) {
                return "unloadable:" + Structures.canonical(tag);
            }
            var output = net.minecraft.world.level.storage.TagValueOutput.createWithContext(reporter, level.registryAccess());
            entity.save(output);
            var saved = output.buildResult();
            clean(saved);
            return Structures.canonical(saved);
        }
    }

    private static void clean(net.minecraft.nbt.CompoundTag tag) {
        tag.remove("UUID");
        String id = tag.getStringOr("id", "");
        boolean zombie = id.equals("minecraft:zombie") || id.equals("minecraft:husk") || id.equals("minecraft:drowned")
            || id.equals("minecraft:zombie_villager") || id.equals("minecraft:zombified_piglin");
        if (zombie) {
            tag.remove("Health");
            tag.remove("CanBreakDoors");
        }
        if (id.equals("minecraft:shulker")) {
            // Never snapped: the yaw comes from the unseeded random of the LivingEntity constructor.
            tag.remove("Rotation");
        }
        var attributes = tag.getListOrEmpty("attributes");
        List<net.minecraft.nbt.CompoundTag> sorted = new ArrayList<>();
        for (int i = 0; i < attributes.size(); i++) {
            var attribute = attributes.getCompoundOrEmpty(i).copy();
            String attributeId = attribute.getStringOr("id", "");
            if (zombie && attributeId.equals("minecraft:spawn_reinforcements")) {
                attribute.remove("base");
            }
            if (zombie && attributeId.equals("minecraft:max_health")) {
                // Only leaders (an unseeded roll) get a max health instance.
                continue;
            }
            var modifiers = attribute.getListOrEmpty("modifiers");
            List<net.minecraft.nbt.CompoundTag> kept = new ArrayList<>();
            for (int j = 0; j < modifiers.size(); j++) {
                var modifier = modifiers.getCompoundOrEmpty(j);
                String modifierId = modifier.getStringOr("id", "");
                boolean random = modifierId.equals("minecraft:zombie_random_spawn_bonus") || modifierId.equals("minecraft:leader_zombie_bonus")
                    || modifierId.equals("minecraft:random_spawn_bonus") && attributeId.equals("minecraft:knockback_resistance");
                if (!(zombie && random)) {
                    kept.add(modifier);
                }
            }
            kept.sort(java.util.Comparator.comparing(m -> m.getStringOr("id", "")));
            var list = new net.minecraft.nbt.ListTag();
            list.addAll(kept);
            if (kept.isEmpty()) {
                attribute.remove("modifiers");
            } else {
                attribute.put("modifiers", list);
            }
            sorted.add(attribute);
        }
        if (!attributes.isEmpty()) {
            sorted.sort(java.util.Comparator.comparing(a -> a.getStringOr("id", "")));
            var list = new net.minecraft.nbt.ListTag();
            list.addAll(sorted);
            tag.put("attributes", list);
        }
        var passengers = tag.getListOrEmpty("Passengers");
        for (int i = 0; i < passengers.size(); i++) {
            clean(passengers.getCompoundOrEmpty(i));
        }
    }

    /** `normentity <hex of a named root compound>`: `normalizeEntity` for an entity of ours. */
    static String normEntity(String hex) throws Exception {
        byte[] bytes = java.util.HexFormat.of().parseHex(hex);
        var tag = net.minecraft.nbt.NbtIo.read(new java.io.DataInputStream(new java.io.ByteArrayInputStream(bytes)));
        return normalizeEntity(tag);
    }

    /** `itemtest <item id>`: encodes a default stack of the item with the registry ops. */
    static String itemTest(String id) {
        try {
            var item = net.minecraft.core.registries.BuiltInRegistries.ITEM.getValue(net.minecraft.resources.Identifier.parse(id));
            var ops = Oracle.full.createSerializationContext(net.minecraft.nbt.NbtOps.INSTANCE);
            return "encoded " + net.minecraft.world.item.ItemStack.CODEC.encodeStart(ops, new net.minecraft.world.item.ItemStack(item));
        } catch (Throwable t) {
            StringBuilder sb = new StringBuilder("failed " + t);
            for (var f : t.getStackTrace()) if (f.getClassName().startsWith("net.minecraft")) sb.append(" at ").append(f);
            return sb.toString();
        }
    }

    /** `entitytest <snbt>`: creates an entity in the last region like `StructureTemplate.placeEntities`, with the error if any. */
    static String entityTest(String snbt) throws Exception {
        var tag = net.minecraft.nbt.TagParser.parseCompoundFully(snbt);
        var level = lastRegion;
        try {
            var entity = net.minecraft.world.entity.EntityType.create(
                net.minecraft.world.level.storage.TagValueInput.create(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess(), tag),
                level.getLevel(), new net.minecraft.world.entity.EntitySpawnRequest(net.minecraft.world.entity.EntitySpawnReason.STRUCTURE, false)).orElseThrow();
            if (entity instanceof net.minecraft.world.entity.Mob mob) {
                mob.finalizeSpawn(level, level.getCurrentDifficultyAt(entity.blockPosition()), net.minecraft.world.entity.EntitySpawnReason.STRUCTURE, null);
            }
            var output = net.minecraft.world.level.storage.TagValueOutput.createWithContext(net.minecraft.util.ProblemReporter.DISCARDING, level.registryAccess());
            entity.saveWithoutId(output);
            return "created " + entity.getType() + " " + output.buildResult();
        } catch (Throwable t) {
            StringBuilder sb = new StringBuilder("failed " + t);
            for (Throwable c = t; c != null; c = c.getCause()) {
                sb.append(" CAUSE ").append(c);
                for (var f : c.getStackTrace()) if (f.getClassName().startsWith("net.minecraft")) sb.append(" at ").append(f);
            }
            return sb.toString();
        }
    }

    /**
     * `features <seed> <source> <settings> <dimension> <cx> <cz> <dimMinY> <dimHeight> <structures 0|1> <features all|none|id,id..>`
     * Prints the 9 chunks (dz-major from -1,-1) after the center chunk's decoration.
     */
    static String run(String[] a) throws Exception {
        Result r = generate(a);
        StringBuilder sb = new StringBuilder();
        for (ProtoChunk chunk : r.chunks) {
            if (sb.length() > 0) {
                sb.append(" ;; ");
            }
            sb.append(Oracle.describeChunk(chunk)).append(" @ ").append(extras(chunk));
        }
        if (System.getenv("CHUNKER_ORACLE_LOG") != null || a[10].startsWith("place:")) {
            sb.append(" ## ").append(String.join(";", r.log));
        }
        return sb.toString();
    }

    /** `featuresection <same args...> <chunk index 0..8> <section index>`: the 4096 state ids of one section. */
    static String section(String[] a) throws Exception {
        Result r = generate(Arrays.copyOf(a, 11));
        ProtoChunk chunk = r.chunks.get(Integer.parseInt(a[11]));
        LevelChunkSection s = chunk.getSections()[Integer.parseInt(a[12])];
        StringBuilder sb = new StringBuilder();
        for (int y = 0; y < 16; y++) {
            for (int z = 0; z < 16; z++) {
                for (int x = 0; x < 16; x++) {
                    if (sb.length() > 0) sb.append(' ');
                    sb.append(Block.BLOCK_STATE_REGISTRY.getId(s.getBlockState(x, y, z)));
                }
            }
        }
        return sb.toString();
    }

    /** Block entities, scheduled ticks and entities of a chunk as canonical text. */
    private static String extras(ProtoChunk chunk) {
        StringBuilder sb = new StringBuilder();
        List<net.minecraft.core.BlockPos> positions = new ArrayList<>(chunk.getBlockEntitiesPos());
        positions.sort(Comparable::compareTo);
        sb.append("be=[");
        for (int i = 0; i < positions.size(); i++) {
            if (i > 0) sb.append(',');
            var tag = chunk.getBlockEntityNbtForSaving(positions.get(i), Oracle.full);
            sb.append(tag == null ? "null" : Structures.canonical(tag));
        }
        sb.append("] bt=[");
        appendTicks(sb, ((net.minecraft.world.ticks.ProtoChunkTicks<Block>) chunk.getBlockTicks()).pack(0L), (Block t) -> net.minecraft.core.registries.BuiltInRegistries.BLOCK.getKey(t).toString());
        sb.append("] ft=[");
        appendTicks(sb, ((net.minecraft.world.ticks.ProtoChunkTicks<net.minecraft.world.level.material.Fluid>) chunk.getFluidTicks()).pack(0L), (net.minecraft.world.level.material.Fluid t) -> net.minecraft.core.registries.BuiltInRegistries.FLUID.getKey(t).toString());
        sb.append("] entities=[");
        List<String> entities = new ArrayList<>();
        for (var tag : chunk.getEntities()) {
            entities.add(normalizeEntity(tag.copy()));
        }
        sb.append(String.join(",", entities)).append(']');
        return sb.toString();
    }

    private static <T> void appendTicks(StringBuilder sb, List<net.minecraft.world.ticks.SavedTick<T>> ticks, java.util.function.Function<T, String> name) {
        List<String> out = new ArrayList<>();
        for (var t : ticks) {
            out.add(name.apply(t.type()) + "@" + t.pos().getX() + "," + t.pos().getY() + "," + t.pos().getZ() + ":" + t.delay() + ":" + t.priority().getValue());
        }
        Collections.sort(out);
        sb.append(String.join(",", out));
    }

    private static synchronized Result generate(String[] a) throws Exception {
        String key = String.join(" ", Arrays.copyOfRange(a, 1, a.length)) + " " + difficulty + " " + clockTime + " " + gameTime;
        if (last != null && last.key.equals(key)) {
            return last;
        }
        long seed = Long.parseLong(a[1]);
        String sourceName = a[2];
        String settingsId = a[3];
        String dimension = a[4];
        int cx = Integer.parseInt(a[5]);
        int cz = Integer.parseInt(a[6]);
        int minY = Integer.parseInt(a[7]);
        int height = Integer.parseInt(a[8]);
        // "0" no structures, "1" all, otherwise a comma list of structure ids to place (starts and
        // terrain adaptation always include every structure).
        boolean structures = !a[9].equals("0");
        Set<String> structureFilter = a[9].equals("0") || a[9].equals("1") ? null : Set.of(a[9].split(","));
        String featureFilter = a[10];

        RegistryAccess.Frozen world = Oracle.world;
        RegistryAccess.Frozen full = Oracle.full;
        ResourceManager resources = Oracle.resources;
        NoiseGeneratorSettings ns = Oracle.settings(settingsId);
        RandomState rs = Oracle.state(seed, settingsId);
        BiomeSource source = Oracle.biomeSource(sourceName);
        ChunkGenerator generator = Structures.generator(world, resources, seed, source, ns, rs);
        var heightAccessor = LevelHeightAccessor.create(minY, height);
        var containers = PalettedContainerFactory.create(world);

        // Structure-start chunks out to 8 (references need them), terrain chunks for the 3x3.
        Map<Long, ProtoChunk> chunks = new HashMap<>();
        for (int x = cx - 9; x <= cx + 9; x++) {
            for (int z = cz - 9; z <= cz + 9; z++) {
                int d = Math.max(Math.abs(x - cx), Math.abs(z - cz));
                ProtoChunk chunk;
                if (d <= 1) {
                    chunk = Oracle.buildTerrain(seed, sourceName, settingsId, x, z, "carvers", minY, height, structures ? dimension : null);
                } else {
                    chunk = new ProtoChunk(new ChunkPos(x, z), UpgradeData.EMPTY, heightAccessor, containers, null);
                }
                if (structures) {
                    chunk.setAllStarts(Structures.startsFor(full, resources, seed, source, ns, rs, dimension, x, z, minY, height));
                }
                chunks.put(ChunkPos.pack(x, z), chunk);
            }
        }
        if (structures) {
            for (int x = cx - 1; x <= cx + 1; x++) {
                for (int z = cz - 1; z <= cz + 1; z++) {
                    ProtoChunk target = chunks.get(ChunkPos.pack(x, z));
                    int bx = x << 4;
                    int bz = z << 4;
                    for (int sx = x - 8; sx <= x + 8; sx++) {
                        for (int sz = z - 8; sz <= z + 8; sz++) {
                            long key2 = ChunkPos.pack(sx, sz);
                            for (StructureStart start : chunks.get(key2).getAllStarts().values()) {
                                if (start.isValid() && start.getBoundingBox().intersects(bx, bz, bx + 15, bz + 15)) {
                                    target.addReferenceForStructure(start.getStructure(), key2);
                                }
                            }
                        }
                    }
                }
            }
        }
        for (ProtoChunk chunk : chunks.values()) {
            int d = Math.max(Math.abs(chunk.getPos().x() - cx), Math.abs(chunk.getPos().z() - cz));
            chunk.setPersistedStatus(d <= 1 ? ChunkStatus.TERRAIN : ChunkStatus.STRUCTURE_STARTS);
        }

        FakeLevel level = allocate(FakeLevel.class);
        level.seed = seed;
        level.access = full;
        level.dimensionType = full.lookupOrThrow(Registries.DIMENSION_TYPE)
            .getOrThrow(ResourceKey.create(Registries.DIMENSION_TYPE, Identifier.parse(dimension))).value();
        // Game time 0: scheduled ticks then save with their plain delay.
        var levelData = allocate(net.minecraft.world.level.storage.PrimaryLevelData.class);
        // Difficulty for `finalizeSpawn` of template entities (normal, like a fresh server).
        setField(levelData, net.minecraft.world.level.storage.PrimaryLevelData.class, "settings", new net.minecraft.world.level.LevelSettings("oracle",
            net.minecraft.world.level.GameType.SURVIVAL, new net.minecraft.world.level.LevelSettings.DifficultySettings(difficulty, false, false), false,
            net.minecraft.world.level.WorldDataConfiguration.DEFAULT));
        setField(levelData, net.minecraft.world.level.storage.PrimaryLevelData.class, "gameTime", gameTime);
        setField(level, net.minecraft.world.level.Level.class, "levelData", levelData);
        setField(level, ServerLevel.class, "serverLevelData", levelData);
        setField(level, net.minecraft.world.level.Level.class, "dimension", ResourceKey.create(Registries.DIMENSION, Identifier.parse(dimension)));
        level.minY = minY;
        level.height = height;
        level.seaLevel = ns.seaLevel();
        level.uncached = source.createUncachedResolver(rs);
        level.source = allocate(FakeChunkSource.class);
        level.source.randomState = rs;
        level.source.generator = generator;
        DedicatedServer server = allocate(DedicatedServer.class);
        setField(server, MinecraftServer.class, "structureTemplateManager", Structures.templateManager(world, resources));
        level.server = server;
        level.light = new LevelLightEngine(new LightChunkGetter() {
            @Override
            public LightChunk getChunkForLighting(final int x, final int z) {
                return null;
            }

            @Override
            public net.minecraft.world.level.BlockGetter getLevel() {
                return level;
            }
        }, true, level.dimensionType.hasSkyLight());

        ChunkStep step = ChunkPyramid.GENERATION_PYRAMID.getStepTo(ChunkStatus.FEATURES);
        int radius = step.getAccumulatedRadiusOf(ChunkStatus.EMPTY);
        StaticCache2D<GenerationChunkHolder> holders = StaticCache2D.create(cx, cz, radius, (x, z) -> {
            try {
                ProtoChunk chunk = chunks.get(ChunkPos.pack(x, z));
                int d = Math.max(Math.abs(x - cx), Math.abs(z - cz));
                return new FakeHolder(new ChunkPos(x, z), chunk, d <= 1 ? ChunkStatus.TERRAIN : ChunkStatus.STRUCTURE_STARTS);
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        });
        ProtoChunk center = chunks.get(ChunkPos.pack(cx, cz));
        LoggingRegion region = new LoggingRegion(level, holders, step, center);
        StructureManager structureManager = new StructureManager(region, new WorldOptions(seed, structures, false), null);
        level.structures = structureManager;
        // Mob brains read environment attributes; the static (dimension/biome) layers are enough.
        level.setEnvironmentAttributes(net.minecraft.world.attribute.EnvironmentAttributeSystem.builder().addStaticLayers(level).build());
        if (featureFilter.equals("vanilla")) {
            generator.applyBiomeDecoration(region, center, structureManager);
        } else if (featureFilter.startsWith("place:")) {
            // place:<configured feature>@x,y,z@seed: one feature with a fresh legacy random.
            String[] parts = featureFilter.substring(6).split("@");
            String[] xyz = parts[1].split(",");
            var feature = full.lookupOrThrow(Registries.FEATURE)
                .getOrThrow(ResourceKey.create(Registries.FEATURE, Identifier.parse(parts[0]))).value();
            var random = new WorldgenRandom(new net.minecraft.world.level.levelgen.LegacyRandomSource(Long.parseLong(parts[2])));
            feature.place(region, generator, random, new net.minecraft.core.BlockPos(Integer.parseInt(xyz[0]), Integer.parseInt(xyz[1]), Integer.parseInt(xyz[2])));
        } else {
            decorate(generator, region, center, structureManager, featureFilter, structureFilter);
        }

        List<ProtoChunk> out = new ArrayList<>();
        for (int dz = -1; dz <= 1; dz++) {
            for (int dx = -1; dx <= 1; dx++) {
                out.add(chunks.get(ChunkPos.pack(cx + dx, cz + dz)));
            }
        }
        last = new Result(key, out, region.log);
        lastRegion = region;
        return last;
    }

    @SuppressWarnings("unchecked")
    static List<FeatureSorter.StepFeatureData> featuresPerStep(ChunkGenerator generator) throws Exception {
        var supplier = (java.util.function.Supplier<List<FeatureSorter.StepFeatureData>>) getField(generator, ChunkGenerator.class, "featuresPerStep");
        return supplier.get();
    }

    @SuppressWarnings("unchecked")
    static BiomeGenerationSettings generationSettings(ChunkGenerator generator, Holder<Biome> biome) throws Exception {
        var getter = (java.util.function.Function<Holder<Biome>, BiomeGenerationSettings>) getField(generator, ChunkGenerator.class, "generationSettingsGetter");
        return getter.apply(biome);
    }

    /** `ChunkGenerator.applyBiomeDecoration` with a placed feature filter (structures follow the manager flag). */
    private static void decorate(ChunkGenerator generator, WorldGenRegion level, ChunkAccess chunk, StructureManager structureManager, String filter,
                                 Set<String> structureFilter) throws Exception {
        Set<String> allowed = filter.equals("all") ? null : filter.equals("none") ? Set.of() : Set.of(filter.split(","));
        ChunkPos centerPos = chunk.getPos();
        SectionPos sectionPos = SectionPos.of(centerPos, level.getMinSectionY());
        net.minecraft.core.BlockPos origin = sectionPos.origin();
        Registry<Structure> structuresRegistry = level.registryAccess().lookupOrThrow(Registries.STRUCTURE);
        Map<Integer, List<Structure>> structuresByStep = structuresRegistry.stream().collect(Collectors.groupingBy(structure -> structure.step().ordinal()));
        List<FeatureSorter.StepFeatureData> featureList = featuresPerStep(generator);
        WorldgenRandom random = new WorldgenRandom(new XoroshiroRandomSource(RandomSupport.generateUniqueSeed()));
        long decorationSeed = random.setDecorationSeed(level.getSeed(), origin.getX(), origin.getZ());
        Set<Holder<Biome>> possibleBiomes = new ObjectArraySet<>();
        ChunkPos.rangeClosed(sectionPos.chunk(), 1).forEach(chunkPos -> {
            ChunkAccess chunkInRange = level.getChunk(chunkPos.x(), chunkPos.z());
            for (LevelChunkSection section : chunkInRange.getSections()) {
                section.getBiomes().getAll(possibleBiomes::add);
            }
        });
        possibleBiomes.retainAll(generator.getBiomeSource().possibleBiomes());
        int featureStepCount = featureList.size();
        FeaturePlacer placer = new FeaturePlacer(level, generator);
        Registry<PlacedFeature> featureRegistry = level.registryAccess().lookupOrThrow(Registries.PLACED_FEATURE);
        int generationSteps = Math.max(GenerationStep.Decoration.values().length, featureStepCount);
        BoundingBox writable = writableArea(chunk);
        for (int stepIndex = 0; stepIndex < generationSteps; stepIndex++) {
            int index = 0;
            if (structureManager.shouldGenerateStructures()) {
                for (Structure structure : structuresByStep.getOrDefault(stepIndex, Collections.emptyList())) {
                    random.setFeatureSeed(decorationSeed, index, stepIndex);
                    String sid = structuresRegistry.getKey(structure).toString();
                    if (structureFilter == null || structureFilter.contains(sid)) {
                        structureManager.startsForStructure(sectionPos.x(), sectionPos.z(), structure)
                            .forEach(start -> start.placeInChunk(level, structureManager, generator, random, writable, centerPos));
                    }
                    index++;
                }
            }
            if (stepIndex < featureStepCount) {
                IntSet possibleFeaturesThisStep = new IntArraySet();
                for (Holder<Biome> biome : possibleBiomes) {
                    List<HolderSet<PlacedFeature>> featuresInBiome = generationSettings(generator, biome).features();
                    if (stepIndex < featuresInBiome.size()) {
                        HolderSet<PlacedFeature> featuresInBiomeThisStep = featuresInBiome.get(stepIndex);
                        FeatureSorter.StepFeatureData stepFeatureData = featureList.get(stepIndex);
                        featuresInBiomeThisStep.stream().map(Holder::value)
                            .forEach(f -> possibleFeaturesThisStep.add(stepFeatureData.indexMapping().applyAsInt(f)));
                    }
                }
                int[] indexArray = possibleFeaturesThisStep.toIntArray();
                Arrays.sort(indexArray);
                FeatureSorter.StepFeatureData stepFeatureData = featureList.get(stepIndex);
                for (int globalIndexOfFeature : indexArray) {
                    PlacedFeature feature = stepFeatureData.features().get(globalIndexOfFeature);
                    String id = featureRegistry.getResourceKey(feature).map(k -> k.identifier().toString()).orElse("?");
                    if (allowed != null && !allowed.contains(id)) {
                        continue;
                    }
                    random.setFeatureSeed(decorationSeed, globalIndexOfFeature, stepIndex);
                    placer.placeWithBiomeCheck(feature, random, origin);
                }
            }
        }
    }

    private static BoundingBox writableArea(ChunkAccess chunk) {
        ChunkPos chunkPos = chunk.getPos();
        int targetBlockX = chunkPos.getMinBlockX();
        int targetBlockZ = chunkPos.getMinBlockZ();
        LevelHeightAccessor heightAccessor = chunk.getHeightAccessorForGeneration();
        int minY = heightAccessor.getMinY() + 1;
        int maxY = heightAccessor.getMaxY();
        return new BoundingBox(targetBlockX, minY, targetBlockZ, targetBlockX + 15, maxY, targetBlockZ + 15);
    }

    /** `featureorder <seed> <source> <settings>`: placed feature ids per step in global index order. */
    static String order(String[] a) throws Exception {
        long seed = Long.parseLong(a[1]);
        RegistryAccess.Frozen world = Oracle.world;
        NoiseGeneratorSettings ns = Oracle.settings(a[3]);
        RandomState rs = Oracle.state(seed, a[3]);
        BiomeSource source = Oracle.biomeSource(a[2]);
        ChunkGenerator generator = Structures.generator(world, Oracle.resources, seed, source, ns, rs);
        Registry<PlacedFeature> registry = world.lookupOrThrow(Registries.PLACED_FEATURE);
        List<String> steps = new ArrayList<>();
        for (var step : featuresPerStep(generator)) {
            steps.add(step.features().stream().map(f -> registry.getResourceKey(f).map(k -> k.identifier().toString()).orElse("?"))
                .collect(Collectors.joining(" ")));
        }
        return String.join(" | ", steps);
    }

    private Features() {
    }
}
