# Voxel worldgen graphs (`.blockgen`)

A `.blockgen` asset describes a whole voxel world as node trees, in the
shape of Hytale's Worldgen V2: a world structure picks biomes from a
density map, and every biome is a terrain density, a material provider
and a list of props. Set the voxel `world` resource's `preset` to `graph`
and `worldgen_file` to the asset. Finite and streamed worlds give the same
cells, since a cell depends only on its coordinates, the seed and the graph.

```json
{
  "version": 1,
  "constants": {"base": 40, "sea": 36},
  "exports": {"hills": {"type": "noise2d", "scale": 60, "octaves": 3, "seed": "hills"}},
  "biome_map": {"type": "noise2d", "scale": 400, "seed": "biomes"},
  "default_biome": "plains",
  "transition": 8,
  "biomes": [
    {"name": "plains", "min": -1, "max": 0.3,
     "terrain": {"type": "sum", "inputs": [
       {"type": "multiply", "inputs": [{"type": "import", "name": "hills"}, 6]},
       {"type": "curve", "points": [[-16, 16], [16, -16]],
        "input": {"type": "base_height", "name": "base"}}]},
     "materials": {"type": "solidity",
       "solid": {"type": "queue", "queue": [
         {"type": "depth", "layers": [
           {"thickness": 1, "material": "grass"},
           {"thickness": 3, "material": "dirt"}]},
         "stone"]},
       "empty": {"type": "horizontal", "base": "sea", "top": 0, "material": "water"}},
     "props": [
       {"positions": {"type": "triangular", "spacing": 9, "jitter": 0.6, "seed": "trees"},
        "on": ["grass"], "chance": 0.7,
        "prop": {"type": "tree", "trunk": "wood", "leaves": "leaves", "height": [4, 6]}}]},
    {"name": "desert", "min": 0.3, "max": 1,
     "terrain": {"type": "curve", "points": [[-16, 16], [16, -16]],
       "input": {"type": "base_height", "name": "base"}},
     "materials": {"type": "solidity", "solid": "sand"}}
  ]
}
```

## World structure

- `constants`: named numbers (`base_height` and `horizontal` read them).
- `exports`: named density trees; `{"type": "import", "name": ...}` copies one in.
- `biome_map`: a density read at y 0 for each column.
- `biomes`: each has `min`/`max` over the biome map, a `terrain`, `materials`
  and `props`. `default_biome` answers where no range does.
- `transition`: cells over which neighbouring biomes' terrain densities blend.

## Density nodes

A density is a number or `{"type": ..., "inputs": [...]}` (or one `input`).
Positive means solid.

| Type | Fields |
| --- | --- |
| `constant`, `x`, `y`, `z` | `value` |
| `base_height` | `name`, `distance` (default true: `y - base`) |
| `noise2d`, `noise3d` | `scale`, `octaves`, `persistence`, `lacunarity`, `seed` |
| `cell2d` | `scale`, `jitter`, `seed`, `returns` (`distance` or `value`) |
| `sum`, `multiply`, `min`, `max` | inputs |
| `abs`, `pow` | `exponent` |
| `clamp` | `min`, `max` |
| `normalize` | `from_min`, `from_max`, `to_min`, `to_max` |
| `curve` | `points` as `[in, out]` pairs |
| `select` | inputs: condition (not 0), then, else |
| `less`, `greater` | two inputs, answers 1 or 0 |
| `mix` | inputs: a, b, t |
| `scale`, `offset` | `scale` / `offset` as a number or `[x, y, z]` |
| `y_override` | `value` |
| `warp` | noise fields plus `strength`, moves x and z |

Seeds are text (or numbers) mixed with the world seed. Nodes that don't
depend on y are worked out once per column.

## Material providers

A provider is a block name (`air` and `empty` are air) or typed:

| Type | Fields |
| --- | --- |
| `constant` | `material` |
| `queue` | `queue`: the first provider that answers |
| `solidity` | `solid`, `empty` |
| `depth` | `layers` of `{thickness, material}` counted down from the surface; `min_space` (open cells above); `from: "ceiling"` counts up |
| `range` | `density`, `ranges` of `{from, to, material}` |
| `horizontal` | `bottom`, `top` (from `base`, a constant), `material` |
| `weighted` | `seed`, `entries` of `{weight, material}` |

## Props

A prop rule is `{positions, prop, on, chance, where}`. `positions` is a
`grid` or `triangular` lattice (`spacing`, `jitter`, `seed`) or a `list` of
`[x, z]` points. A prop stands on the column's surface, only where the
surface block is in `on` (any when empty) and `where` (a density) is
positive, and belongs to the biome at its anchor. Props fill air only.

| Type | Fields |
| --- | --- |
| `cuboid` | `from`, `to` (relative to the anchor), `material` |
| `sphere` | `radius`, `offset`, `material` |
| `tree` | `trunk`, `leaves`, `height` (`[min, max]`), `radius` |
| `column` | `material`, `height` |
| `weighted` | `entries` of `{weight, prop}` |

A prop reaches at most 16 cells from its anchor, and a graph holds at most
4096 nodes after imports are expanded.

## Compared with Hytale's Worldgen V2

The structure follows V2: a noise-range world structure, biomes as terrain
density plus material provider plus props, `Exported`/`Imported` densities,
`BaseHeight` with named constants, `SpaceAndDepth` layering, `Queue` and
`Solidity` providers, and props placed by position providers on a floor
pattern. Not here yet: the node editor, environments and tints, framework
positions, scanners and patterns beyond the floor, prefab props, caves made
from carving densities, and cell noise in 3D.
