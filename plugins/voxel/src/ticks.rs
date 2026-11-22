//! Block ticks: scheduled updates (fluids, falling blocks) and random ones
//! (growth, spreading, decay), stepped once per fixed step.
//!
//! Every choice is a hash of the seed, the step and the cell, so a run
//! replays the same. Only cells near an edit are scheduled; generated
//! water stays still until something disturbs it.

use crate::grid::{Block, Grid, SECTION};
use crate::registry::{Condition, Registry, RuleAct};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Steps a falling block waits before dropping a cell.
const FALL_RATE: u32 = 2;
/// Scheduled ticks a world may hold; past it new ones are dropped.
pub const MAX_QUEUED: usize = 1 << 18;
/// The most cells one `touch` scans for ticks to schedule.
const MAX_TOUCH: i64 = 1 << 20;

const SIDES: [[i32; 3]; 4] = [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]];
const AROUND: [[i32; 3]; 7] = [
    [0, 0, 0],
    [1, 0, 0],
    [-1, 0, 0],
    [0, 1, 0],
    [0, -1, 0],
    [0, 0, 1],
    [0, 0, -1],
];

fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

struct Fluid {
    /// State id per level, 0 the source.
    levels: Vec<Block>,
    rate: u32,
    infinite: bool,
}

enum Act {
    Becomes(Block),
    /// The next state per state of the type.
    Grow(BTreeMap<Block, Block>),
    SpreadsTo(Block),
}

struct Rule {
    act: Act,
    /// Out of `u32::MAX`.
    chance: u32,
    when: Condition,
    near: Option<(BTreeSet<Block>, i32)>,
}

/// What the registry says about ticking, resolved to ids.
#[derive(Default)]
pub struct TickRules {
    fluids: Vec<Fluid>,
    fluid_of: BTreeMap<Block, (usize, u8)>,
    falls: BTreeSet<Block>,
    random: BTreeMap<Block, Vec<Rule>>,
    replaceable: BTreeSet<Block>,
    /// Solid opaque blocks that cover what is under them.
    covers: BTreeSet<Block>,
}

impl TickRules {
    /// Resolves every rule's names; a name that doesn't resolve is an error.
    pub fn new(registry: &Registry) -> Result<TickRules, String> {
        let mut rules = TickRules::default();
        let mut families: BTreeMap<String, usize> = BTreeMap::new();
        for def in registry.defs() {
            if def.replaceable {
                rules.replaceable.insert(def.id);
            }
            if def.collide && !def.replaceable && def.layer == crate::registry::Layer::Opaque {
                rules.covers.insert(def.id);
            }
            if def.falls {
                rules.falls.insert(def.id);
            }
            if let Some(fluid) = &def.fluid {
                let ty = registry
                    .type_of(def.id)
                    .ok_or_else(|| format!("fluid {} has no level property", def.type_name))?;
                let level = def
                    .state
                    .iter()
                    .find(|(p, _)| p == "level")
                    .and_then(|(_, v)| v.parse::<u8>().ok())
                    .ok_or_else(|| format!("fluid {} needs a numeric level", def.type_name))?;
                let family = *families.entry(def.type_name.clone()).or_insert_with(|| {
                    rules.fluids.push(Fluid {
                        levels: Vec::new(),
                        rate: fluid.rate,
                        infinite: fluid.infinite,
                    });
                    rules.fluids.len() - 1
                });
                // Other properties keep the type's default value.
                let mut values = registry
                    .get(ty.default)
                    .map_or(Vec::new(), |d| d.state.clone());
                if def
                    .state
                    .iter()
                    .zip(&values)
                    .any(|(a, b)| a.0 != "level" && a != b)
                {
                    continue;
                }
                values.retain(|(p, _)| p != "level");
                let levels = &mut rules.fluids[family].levels;
                let at = usize::from(level);
                if levels.len() <= at {
                    levels.resize(at + 1, 0);
                }
                levels[at] = def.id;
                rules.fluid_of.insert(def.id, (family, level));
            }
            if def.random.is_empty() {
                continue;
            }
            let mut list = Vec::new();
            for rule in &def.random {
                let act = match &rule.act {
                    RuleAct::Becomes(name) => Act::Becomes(if name == "air" {
                        0
                    } else {
                        registry.state_id(name)?
                    }),
                    RuleAct::SpreadsTo(name) => Act::SpreadsTo(registry.state_id(name)?),
                    RuleAct::Grow(prop) => {
                        let ty = registry
                            .type_of(def.id)
                            .ok_or_else(|| format!("{} has no {prop} to grow", def.type_name))?;
                        let options = &ty
                            .properties
                            .iter()
                            .find(|(p, _)| p == prop)
                            .ok_or_else(|| format!("{} has no property {prop}", def.type_name))?
                            .1;
                        let mut next = BTreeMap::new();
                        for id in ty.base..ty.base + ty.states() as Block {
                            let mut values = ty.values_of(id);
                            let (_, value) = values.iter_mut().find(|(p, _)| p == prop).unwrap();
                            let at = options.iter().position(|o| o == value).unwrap();
                            if let Some(after) = options.get(at + 1) {
                                *value = after.clone();
                                next.insert(id, ty.id_for(&values)?);
                            }
                        }
                        Act::Grow(next)
                    }
                };
                let near = rule.unless_near.as_ref().map(|(name, distance)| {
                    let set = registry
                        .defs()
                        .filter(|d| d.type_name.eq_ignore_ascii_case(name) || d.tags.contains(name))
                        .map(|d| d.id)
                        .collect::<BTreeSet<_>>();
                    (set, *distance)
                });
                if near.as_ref().is_some_and(|(set, _)| set.is_empty()) {
                    return Err(format!(
                        "{}: no block is called or tagged {}",
                        def.type_name,
                        rule.unless_near.as_ref().unwrap().0
                    ));
                }
                list.push(Rule {
                    act,
                    chance: (rule.chance * f64::from(u32::MAX)) as u32,
                    when: rule.when,
                    near,
                });
            }
            rules.random.insert(def.id, list);
        }
        for (fluid, family) in rules.fluids.iter().zip(families.keys()) {
            if fluid.levels.is_empty() || fluid.levels.contains(&0) {
                return Err(format!("fluid {family} needs every level from 0"));
            }
        }
        Ok(rules)
    }

    /// Air, or a block placement and fluids may write over.
    pub fn replaceable(&self, block: Block) -> bool {
        block == 0 || self.replaceable.contains(&block)
    }

    pub fn is_idle(&self) -> bool {
        self.fluids.is_empty() && self.falls.is_empty() && self.random.is_empty()
    }

    fn scheduled(&self, block: Block) -> Option<u32> {
        if let Some((family, _)) = self.fluid_of.get(&block) {
            Some(self.fluids[*family].rate)
        } else {
            self.falls.contains(&block).then_some(FALL_RATE)
        }
    }

    fn covered(&self, grid: &Grid, cell: [i32; 3]) -> bool {
        self.covers.contains(&grid.get(add(cell, [0, 1, 0])))
    }

    /// Whether a fluid at `level` may flow into `cell`.
    fn flows_into(&self, grid: &Grid, cell: [i32; 3], family: usize, level: u8) -> bool {
        if !grid.contains(cell) {
            return false;
        }
        let block = grid.get(cell);
        match self.fluid_of.get(&block) {
            Some((f, l)) => *f == family && *l != 0 && *l > level,
            None => block == 0 || self.replaceable.contains(&block),
        }
    }
}

/// Pending scheduled ticks and the step counter.
#[derive(Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Ticks {
    pub now: u64,
    queue: BTreeSet<(u64, [i32; 3])>,
    #[serde(skip)]
    due: BTreeMap<[i32; 3], u64>,
}

/// What one step changed.
#[derive(Default)]
pub struct Stepped {
    pub cells: Vec<[i32; 3]>,
    /// What each changed cell held before, and its shape then.
    pub before: Vec<(Block, crate::shape::Shape)>,
    pub dropped: usize,
}

fn hash(seed: u32, now: u64, a: [i32; 3], i: u32) -> u64 {
    let mut x = u64::from(seed)
        ^ now.wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (a[0] as u32 as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ (a[1] as u32 as u64).wrapping_mul(0x94d0_49bb_1331_11eb)
        ^ (a[2] as u32 as u64).wrapping_mul(0x2545_f491_4f6c_dd1d)
        ^ u64::from(i).wrapping_mul(0xd6e8_feb8_6659_fd93);
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

impl Ticks {
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Rebuilds the per-cell index after a restore.
    pub fn reindex(&mut self) {
        self.due = self.queue.iter().map(|(at, cell)| (*cell, *at)).collect();
    }

    fn schedule(&mut self, cell: [i32; 3], delay: u32) -> bool {
        let at = self.now + u64::from(delay.max(1));
        if let Some(old) = self.due.get(&cell) {
            if *old <= at {
                return true;
            }
            self.queue.remove(&(*old, cell));
        } else if self.queue.len() >= MAX_QUEUED {
            return false;
        }
        self.queue.insert((at, cell));
        self.due.insert(cell, at);
        true
    }

    fn around(&mut self, rules: &TickRules, grid: &Grid, cell: [i32; 3]) {
        for offset in AROUND {
            let n = add(cell, offset);
            if let Some(delay) = rules.scheduled(grid.get(n)) {
                self.schedule(n, delay);
            }
        }
    }

    /// Schedules what an edit of the box `lo..=hi` may have woken.
    pub fn touch(&mut self, rules: &TickRules, grid: &Grid, lo: [i32; 3], hi: [i32; 3]) {
        if rules.fluid_of.is_empty() && rules.falls.is_empty() {
            return;
        }
        let lo = lo.map(|c| c - 1);
        let hi = hi.map(|c| c + 1);
        let cells: i64 = (0..3)
            .map(|a| i64::from(hi[a] - lo[a] + 1).max(0))
            .product();
        if cells > MAX_TOUCH {
            return;
        }
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    if let Some(delay) = rules.scheduled(grid.get([x, y, z])) {
                        self.schedule([x, y, z], delay);
                    }
                }
            }
        }
    }

    /// One fixed step: due scheduled ticks (at most `budget`), then
    /// `speed` random ticks per 16-cell cube of each listed section.
    pub fn step(
        &mut self,
        rules: &TickRules,
        grid: &mut Grid,
        seed: u32,
        sections: &[[i32; 3]],
        speed: u32,
        budget: usize,
    ) -> Stepped {
        self.now += 1;
        let mut out = Stepped::default();
        let mut set = |ticks: &mut Ticks, grid: &mut Grid, cell, block, out: &mut Stepped| {
            let old = (grid.get(cell), grid.shape_at(cell));
            if grid.set(cell, block) {
                out.cells.push(cell);
                out.before.push(old);
                ticks.around(rules, grid, cell);
            }
        };
        let mut done = 0;
        while done < budget {
            let Some(&(at, cell)) = self.queue.first() else {
                break;
            };
            if at > self.now {
                break;
            }
            self.queue.pop_first();
            self.due.remove(&cell);
            done += 1;
            let block = grid.get(cell);
            if let Some(&(family, level)) = rules.fluid_of.get(&block) {
                self.flow(rules, grid, cell, family, level, &mut set, &mut out);
            } else if rules.falls.contains(&block) {
                let below = add(cell, [0, -1, 0]);
                let open = grid.contains(below) && {
                    let under = grid.get(below);
                    under == 0 || rules.replaceable.contains(&under)
                };
                if open {
                    set(self, grid, below, block, &mut out);
                    set(self, grid, cell, 0, &mut out);
                }
            }
        }
        out.dropped = self.queue.range(..(self.now + 1, [i32::MIN; 3])).count();
        if speed == 0 || rules.random.is_empty() {
            return out;
        }
        // 16-cell cubes, as Minecraft counts them, inside 32-cell sections.
        let per = speed * (SECTION as u32 / 16).pow(3);
        for &section in sections {
            for i in 0..per {
                let h = hash(seed, self.now, section, i);
                let local =
                    [h as i32, (h >> 16) as i32, (h >> 32) as i32].map(|v| v.rem_euclid(SECTION));
                let cell = [0, 1, 2].map(|a| section[a] * SECTION + local[a]);
                let block = grid.get(cell);
                let Some(list) = rules.random.get(&block) else {
                    continue;
                };
                for (r, rule) in list.iter().enumerate() {
                    let roll = hash(seed ^ 0x5eed, self.now, cell, r as u32) as u32;
                    if roll > rule.chance {
                        continue;
                    }
                    let covered = rules.covered(grid, cell);
                    if (rule.when == Condition::Covered && !covered)
                        || (rule.when == Condition::Open && covered)
                    {
                        continue;
                    }
                    if let Some((set_of, d)) = &rule.near
                        && near(grid, cell, set_of, *d)
                    {
                        continue;
                    }
                    match &rule.act {
                        Act::Becomes(to) => set(self, grid, cell, *to, &mut out),
                        Act::Grow(next) => {
                            if let Some(to) = next.get(&block) {
                                set(self, grid, cell, *to, &mut out);
                            }
                        }
                        Act::SpreadsTo(target) => {
                            let off = [h >> 40, h >> 44, h >> 48].map(|v| (v & 15) as i32);
                            let at = add(cell, [off[0] % 3 - 1, off[1] % 5 - 3, off[2] % 3 - 1]);
                            if grid.get(at) == *target && !rules.covered(grid, at) {
                                set(self, grid, at, block, &mut out);
                            }
                        }
                    }
                    break;
                }
            }
        }
        out
    }

    fn flow(
        &mut self,
        rules: &TickRules,
        grid: &mut Grid,
        cell: [i32; 3],
        family: usize,
        mut level: u8,
        set: &mut impl FnMut(&mut Ticks, &mut Grid, [i32; 3], Block, &mut Stepped),
        out: &mut Stepped,
    ) {
        let fluid = &rules.fluids[family];
        let last = (fluid.levels.len() - 1) as u8;
        let of = |grid: &Grid, at| {
            rules
                .fluid_of
                .get(&grid.get(at))
                .filter(|(f, _)| *f == family)
                .map(|(_, l)| *l)
        };
        let below = add(cell, [0, -1, 0]);
        if level != 0 {
            let fed = if of(grid, add(cell, [0, 1, 0])).is_some() {
                Some(1)
            } else {
                let mut best: Option<u8> = None;
                let mut sources = 0;
                for side in SIDES {
                    if let Some(l) = of(grid, add(cell, side)) {
                        sources += u8::from(l == 0);
                        best = Some(best.map_or(l + 1, |b| b.min(l + 1)));
                    }
                }
                let grounded = grid.contains(below)
                    && (rules.covers.contains(&grid.get(below)) || of(grid, below) == Some(0));
                if fluid.infinite && sources >= 2 && grounded {
                    Some(0)
                } else {
                    best
                }
            };
            match fed.filter(|l| *l <= last) {
                None => {
                    set(self, grid, cell, 0, out);
                    return;
                }
                Some(l) if l != level => {
                    level = l;
                    set(self, grid, cell, fluid.levels[usize::from(l)], out);
                }
                Some(_) => {}
            }
        }
        if rules.flows_into(grid, below, family, 1) {
            set(
                self,
                grid,
                below,
                fluid.levels[1.min(usize::from(last))],
                out,
            );
            return;
        }
        // Resting on ground, or on itself: spread sideways.
        if level < last && of(grid, below).is_none_or(|l| l == 0) {
            for side in SIDES {
                let at = add(cell, side);
                if rules.flows_into(grid, at, family, level + 1) {
                    set(self, grid, at, fluid.levels[usize::from(level + 1)], out);
                }
            }
        }
    }
}

fn near(grid: &Grid, cell: [i32; 3], set: &BTreeSet<Block>, distance: i32) -> bool {
    for x in -distance..=distance {
        for y in -distance..=distance {
            for z in -distance..=distance {
                if set.contains(&grid.get(add(cell, [x, y, z]))) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(lines: &[&str]) -> Registry {
        Registry::parse(&lines.iter().map(|l| l.to_string()).collect::<Vec<_>>()).unwrap()
    }

    const WATER: &str = r#"{"name":"water","id":20,"layer":"translucent","fluid":{"rate":1}}"#;

    fn run(ticks: &mut Ticks, rules: &TickRules, grid: &mut Grid, steps: usize) {
        for _ in 0..steps {
            ticks.step(rules, grid, 7, &[], 0, 4096);
        }
    }

    fn floor(grid: &mut Grid) {
        for x in 0..32 {
            for z in 0..32 {
                grid.set([x, 0, z], 1);
            }
        }
    }

    #[test]
    fn fluids_get_a_level_per_spread_step() {
        let reg = registry(&[WATER]);
        assert_eq!(reg.state_id("water").unwrap(), 20);
        assert_eq!(reg.state_id("water[level=7]").unwrap(), 27);
        let def = reg.get(23).unwrap();
        assert!(!def.collide && def.replaceable);
        assert!(TickRules::new(&reg).is_ok());
    }

    #[test]
    fn a_source_spreads_seven_cells_and_drains_when_removed() {
        let reg = registry(&[WATER]);
        let rules = TickRules::new(&reg).unwrap();
        let mut grid = Grid::new([32, 8, 32]);
        floor(&mut grid);
        let mut ticks = Ticks::default();
        grid.set([16, 1, 16], 20);
        ticks.touch(&rules, &grid, [16, 1, 16], [16, 1, 16]);
        run(&mut ticks, &rules, &mut grid, 40);
        assert_eq!(grid.get([17, 1, 16]), 21);
        assert_eq!(grid.get([23, 1, 16]), 27);
        assert_eq!(grid.get([24, 1, 16]), 0);
        assert_eq!(
            grid.get([19, 1, 18]),
            25,
            "diamond-shaped, by taxicab distance"
        );
        assert_eq!(ticks.len(), 0, "the flow settles");
        grid.set([16, 1, 16], 0);
        ticks.touch(&rules, &grid, [16, 1, 16], [16, 1, 16]);
        run(&mut ticks, &rules, &mut grid, 200);
        assert!(
            (0..32).all(|x| (0..32).all(|z| grid.get([x, 1, z]) == 0)),
            "flowing water with no source dries up"
        );
    }

    #[test]
    fn water_falls_before_it_spreads_and_two_sources_make_a_third() {
        let reg = registry(&[WATER]);
        let rules = TickRules::new(&reg).unwrap();
        let mut grid = Grid::new([32, 8, 32]);
        floor(&mut grid);
        let mut ticks = Ticks::default();
        grid.set([4, 5, 4], 20);
        ticks.touch(&rules, &grid, [4, 5, 4], [4, 5, 4]);
        run(&mut ticks, &rules, &mut grid, 6);
        assert_eq!(grid.get([4, 4, 4]), 21);
        assert_eq!(grid.get([5, 5, 4]), 0, "a source over air only falls");
        run(&mut ticks, &rules, &mut grid, 30);
        assert_eq!(grid.get([4, 1, 4]), 21, "the column lands");
        assert_eq!(grid.get([5, 1, 4]), 22);

        let mut grid = Grid::new([32, 8, 32]);
        floor(&mut grid);
        let mut ticks = Ticks::default();
        grid.set([10, 1, 10], 20);
        grid.set([12, 1, 10], 20);
        ticks.touch(&rules, &grid, [10, 1, 10], [12, 1, 10]);
        run(&mut ticks, &rules, &mut grid, 10);
        assert_eq!(grid.get([11, 1, 10]), 20, "an infinite pool");
    }

    #[test]
    fn sand_falls_and_random_ticks_grow_spread_and_decay_the_same_every_run() {
        let reg = registry(&[
            r#"{"name":"sand","falls":true}"#,
            r#"{"name":"grass","random":[{"becomes":"dirt","when":"covered"},{"spreads_to":"dirt"}]}"#,
            r#"{"name":"wheat","id":30,"layer":"cutout","collide":false,"properties":{"age":[0,1,2,3]},"random":[{"grow":"age"}]}"#,
            r#"{"name":"leaves","random":[{"becomes":"air","unless_near":"wood","distance":2}]}"#,
        ]);
        let rules = TickRules::new(&reg).unwrap();
        let mut grid = Grid::new([32, 8, 32]);
        floor(&mut grid);
        let mut ticks = Ticks::default();
        grid.set([3, 6, 3], 4);
        ticks.touch(&rules, &grid, [3, 6, 3], [3, 6, 3]);
        run(&mut ticks, &rules, &mut grid, 20);
        assert_eq!(grid.get([3, 1, 3]), 4);
        assert_eq!(grid.get([3, 2, 3]), 0);

        let world = |grid: &mut Grid| {
            for x in 0..32 {
                for z in 0..32 {
                    grid.set([x, 1, z], 2);
                }
            }
            grid.set([8, 1, 8], 3);
            grid.set([20, 2, 20], 30);
            grid.set([25, 4, 25], 5);
            grid.set([25, 4, 26], 6);
            grid.set([10, 4, 25], 6);
        };
        let mut a = Grid::new([32, 8, 32]);
        world(&mut a);
        let mut ticks = Ticks::default();
        for _ in 0..3000 {
            ticks.step(&rules, &mut a, 9, &[[0, 0, 0]], 16, 64);
        }
        assert_eq!(a.get([20, 2, 20]), 33, "wheat grew to its last age");
        assert_eq!(a.get([25, 4, 26]), 6, "leaves by their wood stay");
        assert_eq!(a.get([10, 4, 25]), 0, "lone leaves decay");
        let grassy = (0..32)
            .flat_map(|x| (0..32).map(move |z| (x, z)))
            .filter(|(x, z)| a.get([*x, 1, *z]) == 3)
            .count();
        assert!(grassy > 1, "grass spread onto dirt");
        let mut b = Grid::new([32, 8, 32]);
        world(&mut b);
        let mut ticks = Ticks::default();
        for _ in 0..3000 {
            ticks.step(&rules, &mut b, 9, &[[0, 0, 0]], 16, 64);
        }
        assert!(
            (0..32).all(|x| (0..8).all(|y| (0..32).all(|z| a.get([x, y, z]) == b.get([x, y, z]))))
        );
    }

    #[test]
    fn a_bad_rule_names_what_is_missing() {
        let reg = registry(&[r#"{"name":"grass","random":[{"becomes":"mud"}]}"#]);
        assert!(TickRules::new(&reg).err().unwrap().contains("mud"));
    }
}
