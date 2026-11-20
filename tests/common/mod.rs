//! Shared by the integration tests: fixture lookup, a bounding box read through
//! skpwright's own scene walk (not skp-open's), and a model built in code.
#![allow(dead_code)]

use skpwright::{
    Definition, DefinitionKind, GeometryRun, Layer, Material, Mesh, MeshEdge, MeshFace, Model,
    PlacedInstance,
};
use std::path::{Path, PathBuf};

pub type Bbox = ([f64; 3], [f64; 3]);

/// `$SKP_OPEN_FIXTURES/<sub>`. A missing folder fails where
/// `SKP_OPEN_REQUIRE_FIXTURES` is set (CI) and says so and skips elsewhere.
pub fn fixtures(sub: &str) -> Option<PathBuf> {
    let dir = std::env::var_os("SKP_OPEN_FIXTURES").map(|d| PathBuf::from(d).join(sub));
    match dir {
        Some(d) if d.is_dir() => Some(d),
        _ => {
            assert!(
                std::env::var_os("SKP_OPEN_REQUIRE_FIXTURES").is_none(),
                "fixtures {sub} are required here: run tools/ci/fetch-fixtures.sh and set SKP_OPEN_FIXTURES"
            );
            eprintln!(
                "skipped: no fixtures {sub} (set SKP_OPEN_FIXTURES, see tools/ci/fetch-fixtures.sh)"
            );
            None
        }
    }
}

pub fn files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == ext) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// A fresh folder under the system temp dir, removed by the caller.
pub fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("skp-open-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn visible(m: &Model, slot: u16) -> bool {
    slot == 0 || m.layer_of(slot).is_none_or(|l| l.visible)
}

fn apply(w: &[f64; 16], p: [f64; 3]) -> [f64; 3] {
    [
        w[0] * p[0] + w[1] * p[1] + w[2] * p[2] + w[3],
        w[4] * p[0] + w[5] * p[1] + w[6] * p[2] + w[7],
        w[8] * p[0] + w[9] * p[1] + w[10] * p[2] + w[11],
    ]
}

/// The visible faces of a run, as (vertex positions of the outer ring and holes).
fn run_points(m: &Model, run: usize, world: &[f64; 16], faces: &mut usize, bb: &mut Option<Bbox>) {
    let mesh = &m.geometry[run].mesh;
    for f in &mesh.faces {
        if f.hidden || !visible(m, f.layer) {
            continue;
        }
        *faces += 1;
        for &i in f.outer.iter().chain(f.holes.iter().flatten()) {
            let p = apply(world, mesh.vertices[i as usize]);
            let b = bb.get_or_insert((p, p));
            for k in 0..3 {
                b.0[k] = b.0[k].min(p[k]);
                b.1[k] = b.1[k].max(p[k]);
            }
        }
    }
}

fn walk(m: &Model, n: &skpwright::Node, faces: &mut usize, bb: &mut Option<Bbox>) {
    if n.hidden || !visible(m, n.layer) {
        return;
    }
    if let Some(r) = n.run {
        run_points(m, r, &n.world, faces, bb);
    }
    for c in &n.children {
        walk(m, c, faces, bb);
    }
}

/// Faces shown and their bounding box (metres), by skpwright's own scene walk.
/// Only for a model read from a file or bytes (its scene needs the reader's offsets).
pub fn shown(m: &Model) -> (usize, Option<Bbox>) {
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let (mut faces, mut bb) = (0, None);
    if let Some(root) = m.geometry.iter().position(|r| r.def_index.is_none()) {
        run_points(m, root, &identity, &mut faces, &mut bb);
    }
    for n in m.scene() {
        walk(m, &n, &mut faces, &mut bb);
    }
    (faces, bb)
}

pub fn close(a: Bbox, b: Bbox, tol: f64) -> bool {
    (0..3).all(|k| (a.0[k] - b.0[k]).abs() <= tol && (a.1[k] - b.1[k]).abs() <= tol)
}

/// A box of `s` metres at the origin, wound counter-clockwise seen from outside.
pub fn cube(s: f64, material: Option<u16>) -> Mesh {
    let vertices = vec![
        [0.0, 0.0, 0.0],
        [s, 0.0, 0.0],
        [s, s, 0.0],
        [0.0, s, 0.0],
        [0.0, 0.0, s],
        [s, 0.0, s],
        [s, s, s],
        [0.0, s, s],
    ];
    let pairs = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    let edges = pairs
        .iter()
        .map(|&(v0, v1)| MeshEdge {
            v0,
            v1,
            ..Default::default()
        })
        .collect();
    let rings: [([u32; 4], [f64; 3]); 6] = [
        ([0, 3, 2, 1], [0.0, 0.0, -1.0]),
        ([4, 5, 6, 7], [0.0, 0.0, 1.0]),
        ([0, 1, 5, 4], [0.0, -1.0, 0.0]),
        ([1, 2, 6, 5], [1.0, 0.0, 0.0]),
        ([2, 3, 7, 6], [0.0, 1.0, 0.0]),
        ([3, 0, 4, 7], [-1.0, 0.0, 0.0]),
    ];
    let faces = rings
        .iter()
        .map(|(r, n)| MeshFace {
            outer: r.to_vec(),
            normal: *n,
            front_material: material,
            ..Default::default()
        })
        .collect();
    Mesh {
        vertices,
        faces,
        edges,
        ..Default::default()
    }
}

/// A 2 m square plate with a 1 m square hole: one face with an inner ring.
pub fn plate() -> Mesh {
    let vertices = vec![
        [0.0, 0.0, 0.0],
        [2.0, 0.0, 0.0],
        [2.0, 2.0, 0.0],
        [0.0, 2.0, 0.0],
        [0.5, 0.5, 0.0],
        [0.5, 1.5, 0.0],
        [1.5, 1.5, 0.0],
        [1.5, 0.5, 0.0],
    ];
    let ring = |a: u32| [(a, a + 1), (a + 1, a + 2), (a + 2, a + 3), (a + 3, a)];
    let edges = ring(0)
        .into_iter()
        .chain(ring(4))
        .map(|(v0, v1)| MeshEdge {
            v0,
            v1,
            ..Default::default()
        })
        .collect();
    let face = MeshFace {
        outer: vec![0, 1, 2, 3],
        holes: vec![vec![4, 5, 6, 7]],
        normal: [0.0, 0.0, 1.0],
        ..Default::default()
    };
    Mesh {
        vertices,
        faces: vec![face],
        edges,
        ..Default::default()
    }
}

fn place(defref: u32, x: f64) -> PlacedInstance {
    PlacedInstance {
        defref,
        transform: [
            1.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            0.0,
            1.0,
            x * skpwright::INCH,
            0.0,
            0.0,
            1.0,
        ],
        is_group: false,
        layer: 1,
        ..Default::default()
    }
}

/// A red box among the model's own entities, a component "Part" (a half-metre box
/// that places a "Plate" inside it) placed twice. 12 + 2 * (12 + 8) = 52 triangles.
pub fn built() -> Model {
    let mut m = Model {
        materials: vec![Material::Solid {
            name: "Red".into(),
            rgba: [200, 30, 30, 255],
            opacity: 1.0,
            colorize_type: 0,
        }],
        material_links: vec![(1, 0)],
        ..Default::default()
    };
    m.layers = vec![
        Layer {
            name: "Layer0".into(),
            visible: true,
            rgba: [255, 84, 84, 255],
        },
        Layer {
            name: "Parts".into(),
            visible: true,
            rgba: [60, 120, 200, 255],
        },
    ];
    m.layer_links = vec![(0, 0), (1, 1)];
    m.definitions = vec![
        Definition {
            name: "Part".into(),
            kind: DefinitionKind::Component,
            map_index: Some(1),
            ..Default::default()
        },
        Definition {
            name: "Plate".into(),
            kind: DefinitionKind::Component,
            map_index: Some(2),
            ..Default::default()
        },
    ];
    m.definition_links = vec![(1, 0), (2, 1)];
    m.geometry = vec![
        GeometryRun {
            def_index: Some(2),
            mesh: plate(),
            ..Default::default()
        },
        GeometryRun {
            def_index: Some(1),
            mesh: cube(0.5, None),
            placed: vec![place(2, 0.0)],
            ..Default::default()
        },
        GeometryRun {
            def_index: None,
            mesh: cube(1.0, Some(1)),
            placed: vec![place(1, 2.0), place(1, 4.0)],
            ..Default::default()
        },
    ];
    m
}

/// `built()` written as the 2026 container and read back, so it has a scene.
pub fn built_read_back() -> Model {
    let bytes = skpwright::Document::from_model(&built())
        .unwrap()
        .write()
        .unwrap();
    Model::parse(&bytes).unwrap()
}
