//! Collider shapes with the actor's scale applied: the concrete dimensions a
//! physics backend builds.
//!
//! Scale follows Unity's rules instead of stretching every shape the same way: a
//! box scales per axis, a sphere by its largest component, a capsule's radius by
//! the larger of the two axes across it and its length by the axis along it. A
//! shape is therefore never silently turned into a hull because the scale is not
//! uniform. Negative scale mirrors a shape, which does not change it, so only the
//! magnitude counts.

use std::f32::consts::PI;

use std::sync::Arc;

use serde::{Serialize, Serializer};

use super::spec::{Axis, ColliderShape};

/// Cooked points a mesh shape shares with its cache; plan output shows the count.
#[derive(Debug, Clone, PartialEq)]
pub struct Points(pub Arc<Vec<[f32; 3]>>);

impl Serialize for Points {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0.len() as u64)
    }
}

/// A cooked triangle list, shown in plan output as a count.
#[derive(Debug, Clone, PartialEq)]
pub struct Indices(pub Arc<Vec<u32>>);

impl Serialize for Indices {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0.len() as u64 / 3)
    }
}

/// A cooked mesh shape under the actor's scale (3D only).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum MeshShape {
    /// One convex hull.
    Hull { points: Points, volume: f32 },
    /// Several convex hulls standing for a concave mesh.
    Compound { hulls: Vec<Points>, volume: f32 },
    /// The triangles as they are (the winding already follows any mirroring).
    Triangles { vertices: Points, indices: Indices },
}

impl MeshShape {
    /// Cooked data under `scale`, which may be negative to mirror.
    pub fn from_cooked(cooked: &crate::physics::cook::Cooked, scale: [f32; 3]) -> MeshShape {
        use crate::physics::cook::Cooked;
        let at = |p: &[f32; 3]| [p[0] * scale[0], p[1] * scale[1], p[2] * scale[2]];
        let factor = (scale[0] * scale[1] * scale[2]).abs();
        match cooked {
            Cooked::Hull { points, volume, .. } => MeshShape::Hull {
                points: Points(Arc::new(points.iter().map(at).collect())),
                volume: volume * factor,
            },
            Cooked::Decomposed { hulls, volume, .. } => MeshShape::Compound {
                hulls: hulls
                    .iter()
                    .map(|h| Points(Arc::new(h.iter().map(at).collect())))
                    .collect(),
                volume: volume * factor,
            },
            Cooked::Triangles {
                vertices, indices, ..
            } => {
                let mirrored = scale[0] * scale[1] * scale[2] < 0.0;
                let indices = if mirrored {
                    indices
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .flat_map(|t| [t[0], t[2], t[1]])
                        .collect()
                } else {
                    indices.clone()
                };
                MeshShape::Triangles {
                    vertices: Points(Arc::new(vertices.iter().map(at).collect())),
                    indices: Indices(Arc::new(indices)),
                }
            }
        }
    }

    /// Cubic metres; a triangle mesh has no inside.
    pub fn volume(&self) -> f32 {
        match self {
            MeshShape::Hull { volume, .. } | MeshShape::Compound { volume, .. } => *volume,
            MeshShape::Triangles { .. } => 0.0,
        }
    }
}

/// A 3D solid primitive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Solid3 {
    Cuboid {
        half: [f32; 3],
    },
    Ball {
        radius: f32,
    },
    /// A segment of `2 * half_segment` along `axis`, swept by `radius`.
    Capsule {
        axis: Axis,
        half_segment: f32,
        radius: f32,
    },
}

/// A 2D primitive. A polygon is convex; edges and chains have no inside.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Solid2 {
    Cuboid {
        half: [f32; 2],
    },
    Ball {
        radius: f32,
    },
    Capsule {
        vertical: bool,
        half_segment: f32,
        radius: f32,
    },
    Convex {
        points: Vec<[f32; 2]>,
    },
    Segment {
        a: [f32; 2],
        b: [f32; 2],
    },
    Polyline {
        points: Vec<[f32; 2]>,
        closed: bool,
    },
}

fn magnitude(scale: [f32; 3]) -> [f32; 3] {
    scale.map(f32::abs)
}

/// A capsule's backend dimensions from its end-to-end `length`: the segment
/// shrinks to nothing (a ball) when the cap would not fit.
fn capsule_parts(radius: f32, length: f32) -> (f32, f32) {
    (((length / 2.0) - radius).max(0.0), radius)
}

/// The 3D primitive for `shape` under `scale`, or `None` for a shape that is not a
/// primitive (a mesh, terrain, tilemap) or belongs to 2D.
pub fn solid3(shape: &ColliderShape, scale: [f32; 3]) -> Option<Solid3> {
    let s = magnitude(scale);
    Some(match shape {
        ColliderShape::Box { size } => Solid3::Cuboid {
            half: [
                size[0] * s[0] / 2.0,
                size[1] * s[1] / 2.0,
                size[2] * s[2] / 2.0,
            ],
        },
        ColliderShape::Sphere { radius } => Solid3::Ball {
            radius: radius * s[0].max(s[1]).max(s[2]),
        },
        ColliderShape::Capsule {
            radius,
            height,
            axis,
        } => {
            let (along, across) = match axis {
                Axis::X => (s[0], s[1].max(s[2])),
                Axis::Y => (s[1], s[0].max(s[2])),
                Axis::Z => (s[2], s[0].max(s[1])),
            };
            let (half_segment, radius) = capsule_parts(radius * across, height * along);
            Solid3::Capsule {
                axis: *axis,
                half_segment,
                radius,
            }
        }
        _ => return None,
    })
}

/// The 2D primitive for `shape` under `scale` (x, y).
pub fn solid2(shape: &ColliderShape, scale: [f32; 2]) -> Option<Solid2> {
    let s = scale.map(f32::abs);
    Some(match shape {
        ColliderShape::Rect { size } => Solid2::Cuboid {
            half: [size[0] * s[0] / 2.0, size[1] * s[1] / 2.0],
        },
        ColliderShape::Circle { radius } => Solid2::Ball {
            radius: radius * s[0].max(s[1]),
        },
        ColliderShape::Capsule2d { size, horizontal } => {
            let (w, h) = (size[0] * s[0], size[1] * s[1]);
            if *horizontal {
                let (half_segment, radius) = capsule_parts(h / 2.0, w);
                Solid2::Capsule {
                    vertical: false,
                    half_segment,
                    radius,
                }
            } else {
                let (half_segment, radius) = capsule_parts(w / 2.0, h);
                Solid2::Capsule {
                    vertical: true,
                    half_segment,
                    radius,
                }
            }
        }
        ColliderShape::Polygon { points } => Solid2::Convex {
            points: points
                .iter()
                .map(|p| [p[0] * scale[0], p[1] * scale[1]])
                .collect(),
        },
        ColliderShape::Edge { a, b } => Solid2::Segment {
            a: [a[0] * scale[0], a[1] * scale[1]],
            b: [b[0] * scale[0], b[1] * scale[1]],
        },
        ColliderShape::Chain { points, closed } => Solid2::Polyline {
            points: points
                .iter()
                .map(|p| [p[0] * scale[0], p[1] * scale[1]])
                .collect(),
            closed: *closed,
        },
        _ => return None,
    })
}

impl Solid3 {
    /// Volume in cubic metres.
    pub fn volume(&self) -> f32 {
        match *self {
            Solid3::Cuboid { half } => 8.0 * half[0] * half[1] * half[2],
            Solid3::Ball { radius } => 4.0 / 3.0 * PI * radius.powi(3),
            Solid3::Capsule {
                half_segment,
                radius,
                ..
            } => PI * radius * radius * (2.0 * half_segment) + 4.0 / 3.0 * PI * radius.powi(3),
        }
    }
}

impl Solid2 {
    /// Area in square world units; shapes with no inside have none.
    pub fn area(&self) -> f32 {
        match self {
            Solid2::Cuboid { half } => 4.0 * half[0] * half[1],
            Solid2::Ball { radius } => PI * radius * radius,
            Solid2::Capsule {
                half_segment,
                radius,
                ..
            } => 4.0 * half_segment * radius + PI * radius * radius,
            Solid2::Convex { points } => shoelace(points).abs() / 2.0,
            Solid2::Segment { .. } | Solid2::Polyline { .. } => 0.0,
        }
    }
}

fn shoelace(points: &[[f32; 2]]) -> f32 {
    let mut sum = 0.0;
    for (i, p) in points.iter().enumerate() {
        let q = points[(i + 1) % points.len()];
        sum += p[0] * q[1] - q[0] * p[1];
    }
    sum
}

/// Whether the points form a convex polygon (collinear points allowed), whichever
/// way they wind.
pub fn is_convex(points: &[[f32; 2]]) -> bool {
    let n = points.len();
    if n < 3 {
        return false;
    }
    let mut sign = 0.0f32;
    for i in 0..n {
        let (a, b, c) = (points[i], points[(i + 1) % n], points[(i + 2) % n]);
        let cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if cross.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_scales_per_axis() {
        let shape = ColliderShape::Box {
            size: [1.0, 2.0, 3.0],
        };
        assert_eq!(
            solid3(&shape, [2.0, 1.0, -0.5]),
            Some(Solid3::Cuboid {
                half: [1.0, 1.0, 0.75]
            })
        );
    }

    #[test]
    fn a_sphere_follows_the_largest_axis() {
        let shape = ColliderShape::Sphere { radius: 0.5 };
        assert_eq!(
            solid3(&shape, [1.0, 3.0, 2.0]),
            Some(Solid3::Ball { radius: 1.5 })
        );
        assert_eq!(
            solid3(&shape, [-2.0, 1.0, 1.0]),
            Some(Solid3::Ball { radius: 1.0 })
        );
    }

    #[test]
    fn a_capsule_takes_its_length_from_its_axis_and_its_radius_from_the_other_two() {
        let shape = ColliderShape::Capsule {
            radius: 0.5,
            height: 2.0,
            axis: Axis::Y,
        };
        let capsule = solid3(&shape, [1.0, 2.0, 1.0]).unwrap();
        assert_eq!(
            capsule,
            Solid3::Capsule {
                axis: Axis::Y,
                half_segment: 1.5,
                radius: 0.5
            }
        );
        // Widening grows the radius, not the length.
        let wide = solid3(&shape, [2.0, 1.0, 1.0]).unwrap();
        assert_eq!(
            wide,
            Solid3::Capsule {
                axis: Axis::Y,
                half_segment: 0.0,
                radius: 1.0
            },
            "a squashed capsule is a ball rather than a hull"
        );
        // A sideways capsule reads the X scale as its length.
        let sideways = ColliderShape::Capsule {
            radius: 0.5,
            height: 2.0,
            axis: Axis::X,
        };
        assert_eq!(
            solid3(&sideways, [3.0, 1.0, 1.0]),
            Some(Solid3::Capsule {
                axis: Axis::X,
                half_segment: 2.5,
                radius: 0.5
            })
        );
    }

    #[test]
    fn other_shapes_are_not_primitives() {
        assert_eq!(solid3(&ColliderShape::Terrain, [1.0; 3]), None);
        assert_eq!(
            solid3(&ColliderShape::Circle { radius: 1.0 }, [1.0; 3]),
            None
        );
        assert_eq!(
            solid2(&ColliderShape::Sphere { radius: 1.0 }, [1.0; 2]),
            None
        );
    }

    #[test]
    fn two_d_shapes_scale_by_the_xy_scale() {
        let rect = ColliderShape::Rect { size: [10.0, 20.0] };
        assert_eq!(
            solid2(&rect, [2.0, 0.5]),
            Some(Solid2::Cuboid { half: [10.0, 5.0] })
        );
        let capsule = ColliderShape::Capsule2d {
            size: [10.0, 30.0],
            horizontal: false,
        };
        assert_eq!(
            solid2(&capsule, [1.0, 1.0]),
            Some(Solid2::Capsule {
                vertical: true,
                half_segment: 10.0,
                radius: 5.0
            })
        );
        let flat = ColliderShape::Capsule2d {
            size: [30.0, 10.0],
            horizontal: true,
        };
        assert_eq!(
            solid2(&flat, [1.0, 1.0]),
            Some(Solid2::Capsule {
                vertical: false,
                half_segment: 10.0,
                radius: 5.0
            })
        );
        let polygon = ColliderShape::Polygon {
            points: vec![[0.0, 0.0], [2.0, 0.0], [0.0, 2.0]],
        };
        match solid2(&polygon, [3.0, 1.0]) {
            Some(Solid2::Convex { points }) => assert_eq!(points[1], [6.0, 0.0]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn volumes_and_areas_match_the_formulas() {
        let cube = Solid3::Cuboid { half: [0.5; 3] };
        assert!((cube.volume() - 1.0).abs() < 1e-6);
        let ball = Solid3::Ball { radius: 1.0 };
        assert!((ball.volume() - 4.18879).abs() < 1e-4);
        let capsule = Solid3::Capsule {
            axis: Axis::Y,
            half_segment: 1.0,
            radius: 1.0,
        };
        assert!((capsule.volume() - (2.0 * PI + 4.18879)).abs() < 1e-4);
        assert!((Solid2::Cuboid { half: [1.0, 2.0] }.area() - 8.0).abs() < 1e-6);
        let triangle = Solid2::Convex {
            points: vec![[0.0, 0.0], [4.0, 0.0], [0.0, 3.0]],
        };
        assert!((triangle.area() - 6.0).abs() < 1e-6);
    }

    #[test]
    fn convexity_ignores_winding() {
        let square = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        assert!(is_convex(&square));
        let mut reversed = square;
        reversed.reverse();
        assert!(is_convex(&reversed));
        let arrow = [[0.0, 0.0], [2.0, 1.0], [0.0, 2.0], [0.5, 1.0]];
        assert!(!is_convex(&arrow));
        assert!(!is_convex(&square[..2]));
    }

    #[test]
    fn a_mirrored_mesh_keeps_its_triangles_facing_out() {
        use crate::physics::cook::{CookControl, CookSettings, MeshKind};
        let cube = crate::physics::cook::tests::cube(2.0);
        let cooked = cube
            .cook(
                MeshKind::Triangles,
                &CookSettings::default(),
                None,
                &CookControl::new(),
            )
            .unwrap();
        let plain = MeshShape::from_cooked(&cooked, [1.0, 1.0, 1.0]);
        let mirror = MeshShape::from_cooked(&cooked, [-1.0, 1.0, 1.0]);
        let (
            MeshShape::Triangles {
                indices: a,
                vertices: va,
            },
            MeshShape::Triangles {
                indices: b,
                vertices: vb,
            },
        ) = (&plain, &mirror)
        else {
            panic!()
        };
        assert_eq!(a.0[..3], [b.0[0], b.0[2], b.0[1]]);
        assert_eq!(vb.0[0][0], -va.0[0][0]);
        assert_eq!(plain.volume(), 0.0);
        let hull = cube
            .cook(
                MeshKind::Hull,
                &CookSettings::default(),
                None,
                &CookControl::new(),
            )
            .unwrap();
        assert!((MeshShape::from_cooked(&hull, [2.0, 3.0, -1.0]).volume() - 48.0).abs() < 1e-3);
    }
}
