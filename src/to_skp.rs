//! rvt-open's model, as the graph every one of its exporters reads, mapped to a
//! skpwright [`Model`] that writes as a `.skp`.
//!
//! The graph is the tree IFCX has: project, site, building, storeys, elements,
//! each element with the bodies it draws. Every node that holds anything
//! becomes a group whose definition carries the faces of the node's own bodies
//! (triangles in metres, world coordinates, welded) and whose placements are
//! its children. The IFC class is the layer (tag), the node's colour a solid
//! material, and its properties an attribute dictionary on the definition.

use rvt::ifc::graph::IfcxGraph;
use serde_json::Value as Json;
use skpwright::{
    Attributes, Definition, DefinitionKind, Dictionary, GeometryRun, Instance, Layer, Material,
    Mesh, MeshEdge, MeshFace, Model, Owner, PlacedInstance, Value,
};
use std::collections::HashMap;

const MAX_DEPTH: usize = 64;
const WELD: f64 = 1e6;

/// What the mapping did.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ToSkpStats {
    /// Groups written (one definition each).
    pub groups: usize,
    /// Nodes left out for holding no body and no child.
    pub empty_skipped: usize,
    /// Triangles read from the graph.
    pub triangles_in: usize,
    /// Triangles that repeat another with its sides swapped (rvt-open draws an open mesh from both
    /// sides; a SketchUp face has two) and are not written again.
    pub two_sided_duplicates: usize,
    /// Triangles written as faces (the rest were degenerate).
    pub faces: usize,
    /// Attribute dictionaries written.
    pub dictionaries: usize,
    /// Bounding box (metres) of every point of every mesh in the graph.
    pub source_bbox: Option<([f64; 3], [f64; 3])>,
}

/// Map `graph` to a SketchUp model.
pub fn graph_to_skp(graph: &IfcxGraph) -> (Model, ToSkpStats) {
    let mut b = Build {
        g: graph,
        m: Model::default(),
        st: ToSkpStats::default(),
        colours: HashMap::new(),
        layers: HashMap::new(),
        pid: 0,
    };
    b.m.layers.push(Layer {
        name: "Layer0".into(),
        visible: true,
        rgba: [255, 84, 84, 255],
    });
    b.m.layer_links.push((0, 0));
    for p in graph
        .nodes
        .values()
        .filter_map(|n| n.mesh.as_ref())
        .flat_map(|m| m.finite_points())
    {
        let bb = b.st.source_bbox.get_or_insert((p, p));
        for (k, c) in p.iter().enumerate() {
            bb.0[k] = bb.0[k].min(*c);
            bb.1[k] = bb.1[k].max(*c);
        }
    }
    let placed = b.placements(&graph.root, 0);
    b.m.geometry.push(GeometryRun {
        def_index: None,
        placed,
        ..Default::default()
    });
    (b.m, b.st)
}

struct Build<'a> {
    g: &'a IfcxGraph,
    m: Model,
    st: ToSkpStats,
    colours: HashMap<[u8; 4], u16>,
    layers: HashMap<String, u16>,
    pid: u32,
}

fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    (l > 1e-12).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn colour_of(n: &rvt::ifc::graph::Node) -> Option<[u8; 4]> {
    let rgb = n
        .attributes
        .get("bsi::ifc::presentation::diffuseColor")?
        .as_array()?;
    let c = |i: usize| {
        (rgb.get(i)
            .and_then(Json::as_f64)
            .unwrap_or(0.0)
            .clamp(0.0, 1.0)
            * 255.0)
            .round() as u8
    };
    let a = n
        .attributes
        .get("bsi::ifc::presentation::opacity")
        .and_then(Json::as_f64)
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    Some([c(0), c(1), c(2), (a * 255.0).round() as u8])
}

fn text(n: &rvt::ifc::graph::Node, key: &str) -> Option<String> {
    n.attributes
        .get(key)
        .and_then(Json::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn class_of(n: &rvt::ifc::graph::Node) -> Option<String> {
    n.attributes
        .get("bsi::ifc::class")
        .and_then(|c| c.get("code"))
        .and_then(Json::as_str)
        .map(str::to_string)
}

fn value_of(v: &Json) -> Option<Value> {
    match v {
        Json::Null => None,
        Json::Bool(b) => Some(Value::Bool(*b)),
        Json::String(s) => Some(Value::Text(s.clone())),
        Json::Number(n) => Some(match n.as_i64().and_then(|i| i32::try_from(i).ok()) {
            Some(i) => Value::Int(i),
            None => Value::Double(n.as_f64()?),
        }),
        other => Some(Value::Text(other.to_string())),
    }
}

impl Build<'_> {
    fn next_pid(&mut self) -> u32 {
        self.pid += 1;
        self.pid
    }

    /// The archive slot of a solid material of this colour, made on first use.
    fn material(&mut self, rgba: [u8; 4]) -> u16 {
        if let Some(s) = self.colours.get(&rgba) {
            return *s;
        }
        let index = self.m.materials.len();
        let slot = (index + 1) as u16;
        let name = if rgba[3] == 255 {
            format!("Colour {:02x}{:02x}{:02x}", rgba[0], rgba[1], rgba[2])
        } else {
            format!(
                "Colour {:02x}{:02x}{:02x}, {}% opaque",
                rgba[0],
                rgba[1],
                rgba[2],
                (f64::from(rgba[3]) / 2.55).round()
            )
        };
        self.m.materials.push(Material::Solid {
            name,
            rgba,
            opacity: f64::from(rgba[3]) / 255.0,
            colorize_type: 0,
        });
        self.m.material_links.push((slot, index));
        self.colours.insert(rgba, slot);
        slot
    }

    /// The archive slot of the layer named `name` (the default layer for none).
    fn layer(&mut self, name: Option<&str>) -> u16 {
        let Some(name) = name else { return 0 };
        if let Some(s) = self.layers.get(name) {
            return *s;
        }
        let slot = self.m.layers.len() as u16;
        self.m.layers.push(Layer {
            name: name.to_string(),
            visible: true,
            rgba: [90, 120, 200, 255],
        });
        self.m.layer_links.push((slot, slot as usize));
        self.layers.insert(name.to_string(), slot);
        slot
    }

    /// Placements, in the owning run, of the children of the node at `path`
    /// that hold something.
    fn placements(&mut self, path: &str, depth: usize) -> Vec<PlacedInstance> {
        let g = self.g;
        let mut out = Vec::new();
        let Some(node) = g.nodes.get(path) else {
            return out;
        };
        for child in node.children.values() {
            let Some(c) = g.nodes.get(child) else {
                continue;
            };
            if c.mesh.is_some() || !c.ifcx || g.type_nodes.contains(child) {
                continue;
            }
            if let Some(p) = self.group(child, depth + 1) {
                out.push(p);
            }
        }
        out
    }

    /// The node at `path` as a group: its definition and run written, its placement returned.
    fn group(&mut self, path: &str, depth: usize) -> Option<PlacedInstance> {
        let g = self.g;
        let node = g.nodes.get(path)?;
        if depth > MAX_DEPTH {
            return None;
        }
        let own = colour_of(node);
        let mut bodies: Vec<(&rvt::ifc::graph::UsdMesh, Option<[u8; 4]>)> = Vec::new();
        if let Some(m) = &node.mesh {
            bodies.push((m, own));
        }
        for child in node.children.values() {
            if let Some(c) = g.nodes.get(child)
                && let Some(m) = &c.mesh
            {
                bodies.push((m, colour_of(c).or(own)));
            }
        }
        let placed = self.placements(path, depth);
        let class = class_of(node);
        let name = text(node, "bsi::ifc::prop::Name")
            .or_else(|| class.clone())
            .unwrap_or_else(|| "Group".to_string());
        let mesh = self.mesh(&bodies);
        if mesh.faces.is_empty() && placed.is_empty() {
            self.st.empty_skipped += 1;
            return None;
        }
        let spatial = matches!(
            class.as_deref(),
            None | Some("IFCPROJECT" | "IFCSITE" | "IFCBUILDING" | "IFCBUILDINGSTOREY")
        );
        let layer = if spatial {
            0
        } else {
            self.layer(class.as_deref())
        };

        let index = self.m.definitions.len();
        let defref = (index + 1) as u32;
        self.m.definitions.push(Definition {
            name: name.clone(),
            kind: DefinitionKind::Group,
            map_index: Some(defref as usize),
            ..Default::default()
        });
        self.m.definition_links.push((defref, index));
        self.m.geometry.push(GeometryRun {
            def_index: Some(defref as usize),
            mesh,
            placed,
            ..Default::default()
        });
        let entries: Vec<(String, Value)> = node
            .attributes
            .iter()
            .filter(|(k, _)| !k.starts_with("usd::") && !k.starts_with("bsi::ifc::presentation::"))
            .filter_map(|(k, v)| {
                let v = if k == "bsi::ifc::class" {
                    v.get("code").and_then(value_of)
                } else {
                    value_of(v)
                };
                v.map(|v| (k.clone(), v))
            })
            .collect();
        if !entries.is_empty() {
            self.st.dictionaries += 1;
            self.m.dictionaries.push(Attributes {
                owner: Owner::Definition(index),
                dictionaries: vec![Dictionary {
                    name: "rvt-open".into(),
                    entries,
                    flag: 0,
                }],
            });
        }
        self.st.groups += 1;

        let pid = self.next_pid();
        let transform = [
            1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0,
        ];
        self.m.instances.push(Instance {
            pid,
            definition: Some(name.clone()),
            defref,
            offset: 0,
            translation_m: [0.0; 3],
            transform,
            material: None,
            name: Some(name),
            is_group: Some(true),
            hidden: Some(false),
            layer: Some(layer),
            slot: None,
            guid: String::new(),
            casts_shadows: Some(true),
            receives_shadows: Some(true),
            locked: Some(false),
        });
        Some(PlacedInstance {
            pid,
            defref,
            transform,
            is_group: true,
            layer,
            ..Default::default()
        })
    }

    /// Triangles as faces, with one vertex per distinct point and one edge per
    /// distinct pair; an edge between two flat neighbours of one material is
    /// hidden, as the diagonal of a quad is.
    fn mesh(&mut self, bodies: &[(&rvt::ifc::graph::UsdMesh, Option<[u8; 4]>)]) -> Mesh {
        let mut mesh = Mesh::default();
        let mut weld: HashMap<[i64; 3], u32> = HashMap::new();
        let mut edge_of: HashMap<(u32, u32), usize> = HashMap::new();
        let mut faces_on: Vec<Vec<usize>> = Vec::new();
        let mut seen: std::collections::HashSet<[u32; 3]> = std::collections::HashSet::new();
        for (um, colour) in bodies {
            let slot = colour.map(|c| self.material(c));
            for t in um.face_vertex_indices.chunks_exact(3) {
                self.st.triangles_in += 1;
                let p: Vec<[f64; 3]> = t
                    .iter()
                    .filter_map(|&i| um.points.get(i as usize).copied())
                    .collect();
                if p.len() != 3 || p.iter().flatten().any(|c| !c.is_finite()) {
                    continue;
                }
                let ids: Vec<u32> = p
                    .iter()
                    .map(|q| {
                        let key = q.map(|c| (c * WELD).round() as i64);
                        *weld.entry(key).or_insert_with(|| {
                            mesh.vertices.push(*q);
                            (mesh.vertices.len() - 1) as u32
                        })
                    })
                    .collect();
                if ids[0] == ids[1] || ids[1] == ids[2] || ids[0] == ids[2] {
                    continue;
                }
                let mut key = [ids[0], ids[1], ids[2]];
                key.sort_unstable();
                if !seen.insert(key) {
                    self.st.two_sided_duplicates += 1;
                    continue;
                }
                let d1 = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
                let d2 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
                let Some(normal) = unit(cross(d1, d2)) else {
                    continue;
                };
                let face = mesh.faces.len();
                mesh.faces.push(MeshFace {
                    outer: ids.clone(),
                    normal,
                    front_material: slot,
                    ..Default::default()
                });
                for k in 0..3 {
                    let (a, b) = (ids[k], ids[(k + 1) % 3]);
                    let key = (a.min(b), a.max(b));
                    let e = *edge_of.entry(key).or_insert_with(|| {
                        mesh.edges.push(MeshEdge {
                            v0: key.0,
                            v1: key.1,
                            ..Default::default()
                        });
                        faces_on.push(Vec::new());
                        mesh.edges.len() - 1
                    });
                    faces_on[e].push(face);
                }
            }
        }
        for (e, on) in faces_on.iter().enumerate() {
            if let [a, b] = on[..] {
                let (fa, fb) = (&mesh.faces[a], &mesh.faces[b]);
                let flat = fa
                    .normal
                    .iter()
                    .zip(&fb.normal)
                    .map(|(x, y)| x * y)
                    .sum::<f64>()
                    > 1.0 - 1e-9;
                if flat && fa.front_material == fb.front_material {
                    mesh.edges[e].soft = true;
                    mesh.edges[e].hidden = true;
                }
            }
        }
        self.st.faces += mesh.faces.len();
        mesh
    }
}
