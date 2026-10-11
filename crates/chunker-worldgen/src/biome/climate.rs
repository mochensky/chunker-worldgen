//! `Climate`: quantised parameter points and the R-tree used by multi-noise biome lookup.

use std::cell::RefCell;

use rustc_hash::FxHashMap;

/// `Climate.quantizeCoord`.
#[inline]
pub fn quantize(coord: f32) -> i64 {
    (coord * 10000.0) as i64
}

/// `Climate.unquantizeCoord`.
#[inline]
pub fn unquantize(coord: i64) -> f32 {
    coord as f32 / 10000.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parameter {
    pub min: i64,
    pub max: i64,
}

impl Parameter {
    pub fn point(v: f32) -> Self {
        Self::span(v, v)
    }

    pub fn span(min: f32, max: f32) -> Self {
        assert!(!(min > max), "min > max: {min} {max}");
        Self {
            min: quantize(min),
            max: quantize(max),
        }
    }

    pub fn span_params(min: Parameter, max: Parameter) -> Self {
        assert!(min.min <= max.max, "min > max");
        Self {
            min: min.min,
            max: max.max,
        }
    }

    #[inline(always)]
    pub fn distance(&self, target: i64) -> i64 {
        let above = target - self.max;
        let below = self.min - target;
        if above > 0 { above } else { below.max(0) }
    }

    fn union(&self, other: Option<Parameter>) -> Parameter {
        match other {
            None => *self,
            Some(o) => Parameter {
                min: self.min.min(o.min),
                max: self.max.max(o.max),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParameterPoint {
    pub temperature: Parameter,
    pub humidity: Parameter,
    pub continentalness: Parameter,
    pub erosion: Parameter,
    pub depth: Parameter,
    pub weirdness: Parameter,
    pub offset: i64,
}

impl ParameterPoint {
    fn space(&self) -> [Parameter; 7] {
        [
            self.temperature,
            self.humidity,
            self.continentalness,
            self.erosion,
            self.depth,
            self.weirdness,
            Parameter {
                min: self.offset,
                max: self.offset,
            },
        ]
    }
}

/// `Climate.TargetPoint`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetPoint {
    pub temperature: i64,
    pub humidity: i64,
    pub continentalness: i64,
    pub erosion: i64,
    pub depth: i64,
    pub weirdness: i64,
}

impl TargetPoint {
    /// `Climate.target(...)`.
    pub fn new(
        temperature: f32,
        humidity: f32,
        continentalness: f32,
        erosion: f32,
        depth: f32,
        weirdness: f32,
    ) -> Self {
        Self {
            temperature: quantize(temperature),
            humidity: quantize(humidity),
            continentalness: quantize(continentalness),
            erosion: quantize(erosion),
            depth: quantize(depth),
            weirdness: quantize(weirdness),
        }
    }

    fn to_array(self) -> [i64; 7] {
        [
            self.temperature,
            self.humidity,
            self.continentalness,
            self.erosion,
            self.depth,
            self.weirdness,
            0,
        ]
    }
}

#[derive(Debug)]
struct Node {
    space: [Parameter; 7],
    /// Leaf value index into `ParameterList::values`, or `None` for sub trees.
    leaf: Option<usize>,
    children: Vec<usize>,
}

impl Node {
    #[inline(always)]
    fn distance(&self, target: &[i64; 7]) -> i64 {
        let mut d = 0i64;
        for i in 0..7 {
            let x = self.space[i].distance(target[i]);
            d = d.wrapping_add(x.wrapping_mul(x));
        }
        d
    }
}

thread_local! {
    /// `RTree.lastResult` is a `ThreadLocal` in vanilla; searches start from the previous result.
    static LAST_RESULT: RefCell<FxHashMap<usize, usize>> = RefCell::new(FxHashMap::default());
}

/// `Climate.ParameterList<T>` with its `RTree` index.
#[derive(Debug)]
pub struct ParameterList<T> {
    pub values: Vec<(ParameterPoint, T)>,
    nodes: Vec<Node>,
    root: usize,
    /// Node index of the leaf for each value.
    tree_id: usize,
}

static NEXT_TREE_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

impl<T: Clone> ParameterList<T> {
    pub fn new(values: Vec<(ParameterPoint, T)>) -> Self {
        Self::with_children_per_node(values, 19)
    }

    pub fn with_children_per_node(values: Vec<(ParameterPoint, T)>, per_node: usize) -> Self {
        assert!(
            !values.is_empty(),
            "Need at least one value to build the search tree."
        );
        let mut nodes: Vec<Node> = Vec::new();
        let leaves: Vec<usize> = values
            .iter()
            .enumerate()
            .map(|(i, (p, _))| {
                nodes.push(Node {
                    space: p.space(),
                    leaf: Some(i),
                    children: Vec::new(),
                });
                nodes.len() - 1
            })
            .collect();
        let root = build(&mut nodes, leaves, per_node);
        let tree_id = NEXT_TREE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            values,
            nodes,
            root,
            tree_id,
        }
    }

    /// `findValue` (R-tree search warm-started with this thread's previous result).
    pub fn find_value(&self, target: &TargetPoint) -> &T {
        let target = target.to_array();
        let candidate = LAST_RESULT.with(|m| m.borrow().get(&self.tree_id).copied());
        let leaf = self.search(self.root, &target, candidate);
        LAST_RESULT.with(|m| m.borrow_mut().insert(self.tree_id, leaf));
        &self.values[self.nodes[leaf].leaf.unwrap()].1
    }

    /// `findValueBruteForce` (for tests).
    pub fn find_value_brute_force(&self, target: &TargetPoint) -> &T {
        let fitness = |p: &ParameterPoint| {
            let sq = |p: Parameter, t: i64| {
                let d = p.distance(t);
                d * d
            };
            sq(p.temperature, target.temperature)
                + sq(p.humidity, target.humidity)
                + sq(p.continentalness, target.continentalness)
                + sq(p.erosion, target.erosion)
                + sq(p.depth, target.depth)
                + sq(p.weirdness, target.weirdness)
                + p.offset * p.offset
        };
        let mut best = &self.values[0];
        let mut best_fitness = fitness(&best.0);
        for v in &self.values[1..] {
            let f = fitness(&v.0);
            if f < best_fitness {
                best_fitness = f;
                best = v;
            }
        }
        &best.1
    }

    fn search(&self, node: usize, target: &[i64; 7], candidate: Option<usize>) -> usize {
        let n = &self.nodes[node];
        if n.leaf.is_some() {
            return node;
        }
        let mut min_distance = candidate.map_or(i64::MAX, |c| self.nodes[c].distance(target));
        let mut closest = candidate;
        for &child in &n.children {
            let child_distance = self.nodes[child].distance(target);
            if min_distance > child_distance {
                let leaf = self.search(child, target, closest);
                let leaf_distance = if child == leaf {
                    child_distance
                } else {
                    self.nodes[leaf].distance(target)
                };
                if min_distance > leaf_distance {
                    min_distance = leaf_distance;
                    closest = Some(leaf);
                }
            }
        }
        closest.expect("R-tree search found no leaf")
    }
}

fn center(p: Parameter) -> i64 {
    (p.min + p.max) / 2
}

fn build(nodes: &mut Vec<Node>, mut children: Vec<usize>, per_node: usize) -> usize {
    assert!(!children.is_empty(), "Need at least one child to build a node");
    if children.len() == 1 {
        return children[0];
    }
    if children.len() <= per_node {
        children.sort_by_key(|&c| nodes[c].space.iter().map(|p| center(*p).abs()).sum::<i64>());
        return subtree(nodes, children);
    }
    let mut min_cost = i64::MAX;
    let mut min_dimension = 0;
    let mut min_buckets: Vec<usize> = Vec::new();
    for d in 0..7 {
        sort_nodes(nodes, &mut children, d, false);
        let buckets = bucketize(nodes, &children, per_node);
        let total: i64 = buckets.iter().map(|&b| cost(&nodes[b].space)).sum();
        if min_cost > total {
            min_cost = total;
            min_dimension = d;
            min_buckets = buckets;
        } else {
            // Discard the bucket nodes we created for this dimension (they are unreferenced).
        }
    }
    sort_nodes(nodes, &mut min_buckets, min_dimension, true);
    let built: Vec<usize> = min_buckets
        .iter()
        .map(|&b| {
            let kids = nodes[b].children.clone();
            build(nodes, kids, per_node)
        })
        .collect();
    subtree(nodes, built)
}

fn sort_nodes(nodes: &[Node], list: &mut [usize], dimension: usize, absolute: bool) {
    list.sort_by(|&a, &b| {
        for i in 0..7 {
            let d = (dimension + i) % 7;
            let (mut ca, mut cb) = (center(nodes[a].space[d]), center(nodes[b].space[d]));
            if absolute {
                ca = ca.abs();
                cb = cb.abs();
            }
            match ca.cmp(&cb) {
                std::cmp::Ordering::Equal => continue,
                o => return o,
            }
        }
        std::cmp::Ordering::Equal
    });
}

fn bucketize(nodes: &mut Vec<Node>, list: &[usize], per_node: usize) -> Vec<usize> {
    let expected =
        (per_node as f64).powf(((list.len() as f64 - 0.01).ln() / (per_node as f64).ln()).floor()) as usize;
    let mut buckets = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for &child in list {
        current.push(child);
        if current.len() >= expected {
            buckets.push(subtree(nodes, std::mem::take(&mut current)));
        }
    }
    if !current.is_empty() {
        buckets.push(subtree(nodes, current));
    }
    buckets
}

fn cost(space: &[Parameter; 7]) -> i64 {
    space.iter().map(|p| (p.max - p.min).abs()).sum()
}

fn subtree(nodes: &mut Vec<Node>, children: Vec<usize>) -> usize {
    let mut bounds: [Option<Parameter>; 7] = [None; 7];
    for &c in &children {
        for d in 0..7 {
            bounds[d] = Some(nodes[c].space[d].union(bounds[d]));
        }
    }
    let space = bounds.map(|b| b.unwrap());
    nodes.push(Node {
        space,
        leaf: None,
        children,
    });
    nodes.len() - 1
}
