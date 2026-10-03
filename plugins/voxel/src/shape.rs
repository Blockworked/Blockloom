//! What part of its cell a solid cell fills, and the faces that draws.
//!
//! A shape is a few boxes (and a wedge for ramps) in a cell of unit size.
//! `faces` lists what to draw: a face lying on the cell's edge says which
//! direction a whole-cube neighbour would hide it from, and faces that two
//! boxes of one shape press together are dropped. Facing turns the shape a
//! quarter turn at a time about the cell's vertical axis.

/// Which side of the cell a stair or ramp rises towards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Facing {
    #[default]
    South,
    West,
    North,
    East,
}

impl Facing {
    const ALL: [Facing; 4] = [Facing::South, Facing::West, Facing::North, Facing::East];

    fn name(self) -> &'static str {
        match self {
            Facing::South => "south",
            Facing::West => "west",
            Facing::North => "north",
            Facing::East => "east",
        }
    }

    fn turns(self) -> usize {
        Facing::ALL.iter().position(|&f| f == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    #[default]
    Cube,
    /// The lower half.
    Slab,
    /// The upper half.
    TopSlab,
    /// A half-width column through the middle.
    Post,
    /// A low step with the tall half on one side.
    Stair(Facing),
    /// A wedge rising towards one side.
    Ramp(Facing),
}

/// One flat piece of a shape's surface, in cell units.
#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    pub points: Vec<[f32; 3]>,
    pub normal: [f32; 3],
    /// Hidden when the neighbour in this direction is a whole cube.
    pub edge: Option<[i32; 3]>,
}

const EPS: f32 = 1e-4;

impl Shape {
    pub fn from_name(name: &str) -> Result<Shape, String> {
        let lowered = name.trim().to_ascii_lowercase().replace(['_', '-'], " ");
        let mut words = lowered.split_whitespace();
        let first = words.next().unwrap_or("");
        let (first, second) = match first {
            "top" if words.next() == Some("slab") => ("top slab", words.next()),
            _ => (first, words.next()),
        };
        let facing = || match second {
            None => Ok(Facing::South),
            Some(w) => Facing::ALL
                .into_iter()
                .find(|f| f.name() == w)
                .ok_or_else(|| format!("no side called {w} (south, west, north or east)")),
        };
        match (first, second) {
            ("cube" | "full", None) => Ok(Shape::Cube),
            ("slab", None) => Ok(Shape::Slab),
            ("top slab", None) => Ok(Shape::TopSlab),
            ("post", None) => Ok(Shape::Post),
            ("stair" | "stairs", _) => Ok(Shape::Stair(facing()?)),
            ("ramp", _) => Ok(Shape::Ramp(facing()?)),
            _ => Err(format!(
                "no shape called {}; try cube, slab, top slab, post, stair or ramp (with south, west, north or east)",
                lowered.trim()
            )),
        }
    }

    pub fn name(self) -> String {
        match self {
            Shape::Cube => "cube".to_string(),
            Shape::Slab => "slab".to_string(),
            Shape::TopSlab => "top slab".to_string(),
            Shape::Post => "post".to_string(),
            Shape::Stair(f) => format!("stair {}", f.name()),
            Shape::Ramp(f) => format!("ramp {}", f.name()),
        }
    }

    pub(crate) fn solids(self) -> Vec<Vec<Face>> {
        let (parts, facing) = match self {
            Shape::Cube => (vec![boxes_faces(&[([0.0; 3], [1.0; 3])])], Facing::South),
            Shape::Slab => (
                vec![boxes_faces(&[([0.0; 3], [1.0, 0.5, 1.0])])],
                Facing::South,
            ),
            Shape::TopSlab => (
                vec![boxes_faces(&[([0.0, 0.5, 0.0], [1.0; 3])])],
                Facing::South,
            ),
            Shape::Post => (
                vec![boxes_faces(&[([0.25, 0.0, 0.25], [0.75, 1.0, 0.75])])],
                Facing::South,
            ),
            Shape::Stair(f) => (
                vec![
                    boxes_faces(&[([0.0; 3], [1.0, 0.5, 0.5])]),
                    boxes_faces(&[([0.0, 0.0, 0.5], [1.0; 3])]),
                ],
                f,
            ),
            Shape::Ramp(f) => (vec![wedge_faces()], f),
        };
        parts
            .into_iter()
            .map(|faces| {
                faces
                    .into_iter()
                    .map(|face| turned(face, facing.turns()))
                    .collect()
            })
            .collect()
    }

    /// The faces to draw, before any neighbour hides one.
    pub fn faces(self) -> Vec<Face> {
        let (faces, facing) = match self {
            Shape::Cube => (boxes_faces(&[([0.0; 3], [1.0; 3])]), Facing::South),
            Shape::Slab => (boxes_faces(&[([0.0; 3], [1.0, 0.5, 1.0])]), Facing::South),
            Shape::TopSlab => (boxes_faces(&[([0.0, 0.5, 0.0], [1.0; 3])]), Facing::South),
            Shape::Post => (
                boxes_faces(&[([0.25, 0.0, 0.25], [0.75, 1.0, 0.75])]),
                Facing::South,
            ),
            Shape::Stair(f) => (
                boxes_faces(&[([0.0; 3], [1.0, 0.5, 0.5]), ([0.0, 0.0, 0.5], [1.0; 3])]),
                f,
            ),
            Shape::Ramp(f) => (wedge_faces(), f),
        };
        faces
            .into_iter()
            .map(|f| turned(f, facing.turns()))
            .collect()
    }
}

/// A quarter turn about the cell's vertical axis, `n` times: (x, z) goes to
/// (1 - z, x), and a direction to (-z, x).
fn turned(mut face: Face, n: usize) -> Face {
    for _ in 0..n {
        for p in &mut face.points {
            *p = [1.0 - p[2], p[1], p[0]];
        }
        face.normal = [-face.normal[2], face.normal[1], face.normal[0]];
        face.edge = face.edge.map(|e| [-e[2], e[1], e[0]]);
    }
    face
}

type Rect = ([f32; 2], [f32; 2]);

/// What is left of `a` once `b`, lying on the same plane, is taken away -
/// when that is still one rectangle.
fn without(a: Rect, b: Rect) -> Option<Rect> {
    let covers = |d: usize| b.0[d] <= a.0[d] + EPS && b.1[d] >= a.1[d] - EPS;
    let inside = |d: usize| b.0[d] >= a.0[d] - EPS && b.1[d] <= a.1[d] + EPS;
    if covers(0) && covers(1) {
        return None;
    }
    for d in 0..2 {
        let other = 1 - d;
        if covers(other) && inside(d) {
            let mut r = a;
            if b.0[d] <= a.0[d] + EPS {
                r.0[d] = b.1[d];
                return Some(r);
            }
            if b.1[d] >= a.1[d] - EPS {
                r.1[d] = b.0[d];
                return Some(r);
            }
        }
    }
    Some(a)
}

/// Every outward face of a set of boxes that do not overlap, with the faces
/// they press together taken out.
fn boxes_faces(boxes: &[([f32; 3], [f32; 3])]) -> Vec<Face> {
    let mut out = Vec::new();
    for (i, (lo, hi)) in boxes.iter().enumerate() {
        for axis in 0..3 {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            for sign in [1i32, -1] {
                let plane = if sign > 0 { hi[axis] } else { lo[axis] };
                let mut rect: Rect = ([lo[u], lo[v]], [hi[u], hi[v]]);
                let mut gone = false;
                for (j, (olo, ohi)) in boxes.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    let other_plane = if sign > 0 { olo[axis] } else { ohi[axis] };
                    if (other_plane - plane).abs() > EPS {
                        continue;
                    }
                    let theirs: Rect = ([olo[u], olo[v]], [ohi[u], ohi[v]]);
                    match without(rect, theirs) {
                        Some(left) => rect = left,
                        None => gone = true,
                    }
                }
                if gone {
                    continue;
                }
                let corner = |a: f32, b: f32| {
                    let mut p = [0.0; 3];
                    p[axis] = plane;
                    p[u] = a;
                    p[v] = b;
                    p
                };
                let mut normal = [0.0; 3];
                normal[axis] = sign as f32;
                let on_edge = if sign > 0 {
                    plane >= 1.0 - EPS
                } else {
                    plane <= EPS
                };
                let mut edge = [0; 3];
                edge[axis] = sign;
                out.push(Face {
                    points: vec![
                        corner(rect.0[0], rect.0[1]),
                        corner(rect.1[0], rect.0[1]),
                        corner(rect.1[0], rect.1[1]),
                        corner(rect.0[0], rect.1[1]),
                    ],
                    normal,
                    edge: on_edge.then_some(edge),
                });
            }
        }
    }
    out
}

/// The wedge that rises towards +z: low at z = 0, a full wall at z = 1.
fn wedge_faces() -> Vec<Face> {
    let s = std::f32::consts::FRAC_1_SQRT_2;
    vec![
        Face {
            points: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
            ],
            normal: [0.0, -1.0, 0.0],
            edge: Some([0, -1, 0]),
        },
        Face {
            points: vec![
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
            normal: [0.0, 0.0, 1.0],
            edge: Some([0, 0, 1]),
        },
        Face {
            points: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 1.0]],
            normal: [-1.0, 0.0, 0.0],
            edge: Some([-1, 0, 0]),
        },
        Face {
            points: vec![[1.0, 0.0, 0.0], [1.0, 1.0, 1.0], [1.0, 0.0, 1.0]],
            normal: [1.0, 0.0, 0.0],
            edge: Some([1, 0, 0]),
        },
        Face {
            points: vec![
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
            normal: [0.0, s, -s],
            edge: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(face: &Face) -> f32 {
        // Fan area of a convex polygon.
        let p = &face.points;
        (1..p.len() - 1)
            .map(|i| {
                let a = [p[i][0] - p[0][0], p[i][1] - p[0][1], p[i][2] - p[0][2]];
                let b = [
                    p[i + 1][0] - p[0][0],
                    p[i + 1][1] - p[0][1],
                    p[i + 1][2] - p[0][2],
                ];
                let c = [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ];
                (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt() / 2.0
            })
            .sum()
    }

    #[test]
    fn names_round_trip_and_bad_ones_are_refused() {
        for shape in [
            Shape::Cube,
            Shape::Slab,
            Shape::TopSlab,
            Shape::Post,
            Shape::Stair(Facing::West),
            Shape::Ramp(Facing::East),
            Shape::Ramp(Facing::South),
        ] {
            assert_eq!(Shape::from_name(&shape.name()), Ok(shape), "{shape:?}");
        }
        assert_eq!(Shape::from_name("Top_Slab"), Ok(Shape::TopSlab));
        assert_eq!(Shape::from_name("stair"), Ok(Shape::Stair(Facing::South)));
        assert!(Shape::from_name("ramp up").is_err());
        assert!(Shape::from_name("dome").is_err());
        assert!(Shape::from_name("slab north").is_err());
    }

    #[test]
    fn a_stair_drops_the_faces_its_two_boxes_press_together() {
        let faces = Shape::Stair(Facing::South).faces();
        // Front step 6 - 1 hidden, back block 6 with its front cut to the upper half.
        assert_eq!(faces.len(), 11);
        let front_of_back = faces
            .iter()
            .find(|f| f.normal == [0.0, 0.0, -1.0] && f.points.iter().all(|p| p[2] == 0.5))
            .unwrap();
        assert!((area(front_of_back) - 0.5).abs() < 1e-5);
        // Step 2.0 plus back block 3.5 square cells of surface.
        let total: f32 = faces.iter().map(area).sum();
        assert!((total - 5.5).abs() < 1e-4, "{total}");
    }

    #[test]
    fn a_ramp_has_a_slope_and_triangular_sides() {
        let faces = Shape::Ramp(Facing::South).faces();
        assert_eq!(faces.len(), 5);
        assert_eq!(faces.iter().filter(|f| f.points.len() == 3).count(), 2);
        let slope = faces.iter().find(|f| f.edge.is_none()).unwrap();
        assert!((area(slope) - 2f32.sqrt()).abs() < 1e-5);
    }

    #[test]
    fn facing_turns_the_shape_and_its_edges() {
        for turns in 0..4 {
            let f = Facing::ALL[turns];
            for face in Shape::Ramp(f).faces() {
                // Every point stays inside the cell, and a face on the edge
                // really lies on the plane its edge names.
                assert!(
                    face.points
                        .iter()
                        .flatten()
                        .all(|&c| (-EPS..=1.0 + EPS).contains(&c))
                );
                if let Some(e) = face.edge {
                    let axis = (0..3).find(|&a| e[a] != 0).unwrap();
                    let want = if e[axis] > 0 { 1.0 } else { 0.0 };
                    assert!(
                        face.points.iter().all(|p| (p[axis] - want).abs() < EPS),
                        "{f:?}"
                    );
                }
            }
        }
        // South rises towards +z, east towards +x.
        let high = |s: Shape| {
            let wall = s
                .faces()
                .into_iter()
                .find(|f| f.normal[1] == 0.0 && f.points.len() == 4)
                .unwrap();
            wall.normal
        };
        assert_eq!(high(Shape::Ramp(Facing::South)), [0.0, 0.0, 1.0]);
        assert_eq!(high(Shape::Ramp(Facing::East)), [1.0, 0.0, 0.0]);
    }
}
