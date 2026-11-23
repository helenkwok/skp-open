//! A `.skp` read by skpwright, mapped to rvt-open's model, from which every
//! exporter of rvt-open writes.
//!
//! Each group or component instance becomes one building element: the
//! faces of its own definition, cut into triangles and placed in world
//! coordinates (an `IfcTriangulatedFaceSet`, one material per triangle).
//! An instance that places others becomes an element assembly that
//! aggregates them. Names, layers and attribute dictionaries ride along as
//! property sets. Hidden instances, hidden faces and anything on a hidden
//! layer are left out, as an export of what SketchUp shows would.

use rvt::ifc::entities::{
    BrepTriangle, IfcEntity, Property, PropertySet, PropertyValue, SolidShape,
};
use rvt::ifc::{ExportConvention, IfcModel, MaterialInfo, RevitGlobalIds};
use skpwright::{Material, Model, Owner, Value};
use std::collections::HashMap;

const FT: f64 = 0.3048;
const MAX_DEPTH: usize = 64;

type M4 = [f64; 16];

/// What the mapping did, for the notes a conversion prints and the tests count.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FromSkpStats {
    /// Building elements written (assemblies included).
    pub elements: usize,
    /// Faces of the source drawn.
    pub faces: usize,
    /// Triangles written.
    pub triangles: usize,
    /// Faces left out for being hidden or on a hidden layer.
    pub faces_hidden: usize,
    /// Instances left out for being hidden, on a hidden layer or without a definition.
    pub instances_skipped: usize,
    /// Faces of a textured material, drawn in the material's colour only.
    pub textured_faces: usize,
    /// Faces that could not be cut into triangles.
    pub faces_unfilled: usize,
    /// Attribute dictionaries written as property sets.
    pub attribute_sets: usize,
    /// Faces with no material of their own or inherited, drawn in the style's front colour.
    pub faces_default: usize,
}

/// Map `model` to an [`IfcModel`] named `name`.
pub fn skp_to_model(model: &Model, name: &str) -> (IfcModel, FromSkpStats) {
    let mut w = Walk::new(model);
    w.out.project_name = Some(name.to_string());
    w.out.convention = ExportConvention::Open;
    w.out.global_ids = RevitGlobalIds {
        history: model.model_guid.clone(),
        guid_namespace: Some("skp-open ifc GlobalId v1".to_string()),
        ..Default::default()
    };
    w.out.materials = model.materials.iter().map(material_info).collect();
    if let Some(root) = model.geometry.iter().position(|r| r.def_index.is_none()) {
        let identity = identity();
        if let Some(mesh) = w.mesh(root, &identity, 0) {
            w.push_element("Model geometry".into(), &[], Some(mesh), 0, &[], &[]);
        }
        w.children(root, &identity, 0, "", 0);
    }
    (w.out, w.st)
}

fn identity() -> M4 {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}

/// skpwright's 13-value transform as a row-major 4x4, translation in metres.
fn mat_of(t: &[f64; 13]) -> M4 {
    const M_PER_IN: f64 = 0.0254;
    [
        t[0],
        t[1],
        t[2],
        t[9] * M_PER_IN,
        t[3],
        t[4],
        t[5],
        t[10] * M_PER_IN,
        t[6],
        t[7],
        t[8],
        t[11] * M_PER_IN,
        0.0,
        0.0,
        0.0,
        1.0,
    ]
}

fn mul(a: &M4, b: &M4) -> M4 {
    let mut o = [0.0; 16];
    for r in 0..4 {
        for c in 0..4 {
            o[r * 4 + c] = (0..4).map(|k| a[r * 4 + k] * b[k * 4 + c]).sum();
        }
    }
    o
}

fn apply(m: &M4, p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
    ]
}

fn det3(m: &M4) -> f64 {
    m[0] * (m[5] * m[10] - m[6] * m[9]) - m[1] * (m[4] * m[10] - m[6] * m[8])
        + m[2] * (m[4] * m[9] - m[5] * m[8])
}

fn material_info(m: &Material) -> MaterialInfo {
    let (name, rgba, opacity) = match m {
        Material::Solid {
            name,
            rgba,
            opacity,
            ..
        } => (name, *rgba, *opacity),
        Material::Textured {
            name,
            avg_rgba,
            opacity,
            ..
        } => (name, avg_rgba.unwrap_or([160, 160, 160, 255]), *opacity),
    };
    MaterialInfo {
        name: name.clone(),
        color_packed: Some(u32::from(rgba[0]) | u32::from(rgba[1]) << 8 | u32::from(rgba[2]) << 16),
        transparency: (opacity < 1.0).then(|| (1.0 - opacity).clamp(0.0, 1.0)),
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = dot(a, a).sqrt();
    (l > 1e-12).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

/// A face cut into triangles (indices into `mesh.vertices`), wound as the
/// face's normal says. `None` when it cannot be cut.
fn triangulate(mesh: &skpwright::Mesh, f: &skpwright::MeshFace) -> Option<Vec<[u32; 3]>> {
    let n = mesh.vertices.len();
    let ok = |ring: &[u32]| ring.len() >= 3 && ring.iter().all(|&i| (i as usize) < n);
    if !ok(&f.outer) || f.holes.iter().any(|h| !ok(h)) {
        return None;
    }
    let p = |i: u32| mesh.vertices[i as usize];
    if f.outer.len() == 3 && f.holes.is_empty() {
        return Some(vec![[f.outer[0], f.outer[1], f.outer[2]]]);
    }
    // The face's own normal, else the outer ring's (Newell).
    let normal = unit(f.normal).or_else(|| {
        let mut s = [0.0; 3];
        for k in 0..f.outer.len() {
            let (a, b) = (p(f.outer[k]), p(f.outer[(k + 1) % f.outer.len()]));
            s[0] += (a[1] - b[1]) * (a[2] + b[2]);
            s[1] += (a[2] - b[2]) * (a[0] + b[0]);
            s[2] += (a[0] - b[0]) * (a[1] + b[1]);
        }
        unit(s)
    })?;
    let axis = if normal[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let u = unit(cross(normal, axis))?;
    let v = cross(normal, u);
    let mut ids: Vec<u32> = Vec::new();
    let mut flat: Vec<[f64; 2]> = Vec::new();
    let mut holes: Vec<u32> = Vec::new();
    for (k, ring) in std::iter::once(&f.outer).chain(f.holes.iter()).enumerate() {
        if k > 0 {
            holes.push(ids.len() as u32);
        }
        for &i in ring {
            ids.push(i);
            flat.push([dot(p(i), u), dot(p(i), v)]);
        }
    }
    let mut out: Vec<u32> = Vec::new();
    earcut::Earcut::<f64>::new().earcut(flat.iter().copied(), &holes, &mut out);
    if out.is_empty() || !out.len().is_multiple_of(3) {
        return None;
    }
    Some(
        out.chunks_exact(3)
            .map(|t| {
                let tri = [ids[t[0] as usize], ids[t[1] as usize], ids[t[2] as usize]];
                let g = cross(sub(p(tri[1]), p(tri[0])), sub(p(tri[2]), p(tri[0])));
                if dot(g, normal) < 0.0 {
                    [tri[0], tri[2], tri[1]]
                } else {
                    tri
                }
            })
            .collect(),
    )
}

/// One element's triangles: points in feet, triangles, a material per triangle.
struct Body {
    points: Vec<[f64; 3]>,
    triangles: Vec<BrepTriangle>,
    materials: Vec<usize>,
}

struct Walk<'a> {
    m: &'a Model,
    out: IfcModel,
    st: FromSkpStats,
    slot_material: HashMap<u16, usize>,
    instance_name: HashMap<u32, &'a str>,
    definition_of: HashMap<u32, usize>,
    sets: HashMap<usize, Vec<PropertySet>>,
    /// The material for unpainted faces, added on first use.
    default_material: Option<usize>,
}

impl<'a> Walk<'a> {
    fn new(m: &'a Model) -> Self {
        let mut sets: HashMap<usize, Vec<PropertySet>> = HashMap::new();
        for a in &m.dictionaries {
            if let Owner::Definition(d) = a.owner {
                for dict in a.dictionaries.iter().filter(|d| !d.entries.is_empty()) {
                    sets.entry(d).or_default().push(PropertySet {
                        name: format!("SKP_{}", dict.name),
                        properties: dict
                            .entries
                            .iter()
                            .map(|(k, v)| Property {
                                name: k.clone(),
                                value: property_value(v),
                            })
                            .collect(),
                    });
                }
            }
        }
        Walk {
            m,
            out: IfcModel::default(),
            st: FromSkpStats::default(),
            slot_material: m.material_links.iter().copied().collect(),
            instance_name: m
                .instances
                .iter()
                .filter(|i| i.pid != 0)
                .filter_map(|i| i.name.as_deref().map(|n| (i.pid, n)))
                .filter(|(_, n)| !n.is_empty())
                .collect(),
            definition_of: m.definition_links.iter().copied().collect(),
            sets,
            default_material: None,
        }
    }

    /// SketchUp draws a face with no material in the style's front colour.
    fn default_material(&mut self) -> usize {
        *self.default_material.get_or_insert_with(|| {
            let [r, g, b, _] = self
                .m
                .rendering
                .as_ref()
                .map_or([255, 255, 255, 255], |o| o.front_color);
            self.out.materials.push(MaterialInfo {
                name: "Default".to_string(),
                color_packed: Some(u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16),
                transparency: None,
            });
            self.out.materials.len() - 1
        })
    }

    fn layer_visible(&self, slot: u16) -> bool {
        slot == 0 || self.m.layer_of(slot).is_none_or(|l| l.visible)
    }

    fn layer_name(&self, slot: u16) -> String {
        self.m
            .layer_of(slot)
            .map_or_else(|| "Layer0".to_string(), |l| l.name.clone())
    }

    /// The triangles of the faces `run` itself holds, placed by `world`.
    fn mesh(&mut self, run: usize, world: &M4, inherited: u16) -> Option<Body> {
        let m = self.m;
        let mesh = &m.geometry[run].mesh;
        let flip = det3(world) < 0.0;
        let mut remap: HashMap<u32, u32> = HashMap::new();
        let mut body = Body {
            points: Vec::new(),
            triangles: Vec::new(),
            materials: Vec::new(),
        };
        for f in &mesh.faces {
            if f.hidden || !self.layer_visible(f.layer) {
                self.st.faces_hidden += 1;
                continue;
            }
            let Some(tris) = triangulate(mesh, f) else {
                self.st.faces_unfilled += 1;
                continue;
            };
            self.st.faces += 1;
            let slot = f.front_material.filter(|s| *s != 0).unwrap_or(inherited);
            let material = self.slot_material.get(&slot).copied().filter(|_| slot != 0);
            let material = match material {
                Some(i) => i,
                None => {
                    self.st.faces_default += 1;
                    self.default_material()
                }
            };
            if matches!(
                self.m.materials.get(material),
                Some(Material::Textured { .. })
            ) {
                self.st.textured_faces += 1;
            }
            for t in tris {
                let mut ix = t.map(|v| {
                    *remap.entry(v).or_insert_with(|| {
                        let p = apply(world, mesh.vertices[v as usize]);
                        body.points.push([p[0] / FT, p[1] / FT, p[2] / FT]);
                        (body.points.len() - 1) as u32
                    })
                });
                if flip {
                    ix.swap(1, 2);
                }
                body.triangles.push(BrepTriangle(ix[0], ix[1], ix[2]));
                body.materials.push(material);
            }
        }
        self.st.triangles += body.triangles.len();
        (!body.triangles.is_empty()).then_some(body)
    }

    /// The placements of `run`, as elements; the indices of those written.
    fn children(
        &mut self,
        run: usize,
        world: &M4,
        inherited: u16,
        path: &str,
        depth: usize,
    ) -> Vec<usize> {
        let mut out = Vec::new();
        if depth >= MAX_DEPTH {
            return out;
        }
        for p in &self.m.geometry[run].placed {
            let child = self
                .m
                .geometry
                .iter()
                .position(|r| r.def_index == Some(p.defref as usize));
            let Some(child) = child.filter(|_| !p.hidden && self.layer_visible(p.layer)) else {
                self.st.instances_skipped += 1;
                continue;
            };
            let def = self.definition_of.get(&p.defref).copied();
            let def_name = def.map(|d| self.m.definitions[d].name.as_str());
            let name = self
                .instance_name
                .get(&p.pid)
                .copied()
                .or(def_name)
                .filter(|n| !n.is_empty())
                .unwrap_or(if p.is_group { "Group" } else { "Component" })
                .to_string();
            let world = mul(world, &mat_of(&p.transform));
            let inherited = if p.material != 0 {
                p.material
            } else {
                inherited
            };
            let own = self.mesh(child, &world, inherited);
            let here = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            let kids = self.children(child, &world, inherited, &here, depth + 1);
            let sets = def
                .and_then(|d| self.sets.get(&d).cloned())
                .unwrap_or_default();
            let facts = vec![
                ("Path", PropertyValue::Text(here)),
                ("Layer", PropertyValue::Text(self.layer_name(p.layer))),
                (
                    "Kind",
                    PropertyValue::Label(if p.is_group { "Group" } else { "Component" }.into()),
                ),
                (
                    "Definition",
                    PropertyValue::Text(def_name.unwrap_or_default().to_string()),
                ),
            ];
            if let Some(i) = self.push_element(name, &facts, own, p.pid, &kids, &sets) {
                out.push(i);
            }
        }
        out
    }

    /// The element for a mesh, a set of children or both; `None` when it holds neither.
    fn push_element(
        &mut self,
        name: String,
        facts: &[(&str, PropertyValue)],
        body: Option<Body>,
        pid: u32,
        kids: &[usize],
        sets: &[PropertySet],
    ) -> Option<usize> {
        if body.is_none() && kids.is_empty() {
            return None;
        }
        let (shape, material_index) = match body {
            Some(b) => {
                let first = b.materials.first().copied();
                let uniform = first.filter(|f| b.materials.iter().all(|m| m == f));
                let shape = SolidShape::TriangulatedFaceSet {
                    vertices_feet: b.points,
                    triangles: b.triangles,
                    face_materials: if uniform.is_some() {
                        Vec::new()
                    } else {
                        b.materials
                    },
                };
                (Some(shape), uniform)
            }
            None => (None, None),
        };
        let index = self.out.entities.len();
        self.out.entities.push(IfcEntity::BuildingElement {
            ifc_type: if kids.is_empty() {
                "IFCBUILDINGELEMENTPROXY"
            } else {
                "IFCELEMENTASSEMBLY"
            }
            .to_string(),
            name,
            type_guid: (pid != 0).then(|| pid.to_string()),
            predefined_type: Some("NOTDEFINED".to_string()),
            storey_index: None,
            material_index,
            property_set: Some(PropertySet {
                name: "SketchUp".to_string(),
                properties: facts
                    .iter()
                    .map(|(k, v)| Property {
                        name: (*k).to_string(),
                        value: v.clone(),
                    })
                    .collect(),
            }),
            location_feet: None,
            rotation_radians: None,
            extrusion: None,
            host_element_index: None,
            material_layer_set_index: None,
            material_profile_set_index: None,
            solid_shape: shape,
            representation_map_index: None,
        });
        self.st.elements += 1;
        for set in sets {
            self.st.attribute_sets += 1;
            self.out.entities.push(IfcEntity::ElementPropertySet {
                element: index,
                set: set.clone(),
            });
        }
        if !kids.is_empty() {
            self.out.entities.push(IfcEntity::Aggregate {
                whole: index,
                parts: kids.to_vec(),
            });
        }
        Some(index)
    }
}

fn property_value(v: &Value) -> PropertyValue {
    match v {
        Value::Text(s) => PropertyValue::Text(s.clone()),
        Value::Int(i) => PropertyValue::Integer(i64::from(*i)),
        Value::Float(f) => PropertyValue::Real(f64::from(*f)),
        Value::Double(f) => PropertyValue::Real(*f),
        Value::Bool(b) => PropertyValue::Boolean(*b),
        Value::Time(t) => PropertyValue::Integer(i64::from(*t)),
        other => PropertyValue::Text(format!("{other:?}")),
    }
}
