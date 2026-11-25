//! A `.skp` read by skpwright, mapped to rvt-open's model, from which every
//! exporter of rvt-open writes.
//!
//! Each group or component instance becomes one building element: the
//! faces of its own definition, cut into triangles and placed in world
//! coordinates (an `IfcTriangulatedFaceSet`, one material per triangle).
//! An instance that places others becomes an element assembly that
//! aggregates them. Names, layers and attribute dictionaries ride along as
//! property sets. Hidden instances, hidden faces and anything on a hidden
//! layer are left out, as an export of what SketchUp shows would. A textured
//! material carries its image, and the faces drawn in it their texture
//! coordinates, placed as SketchUp places them (the material's size, the
//! face's own texture position).

use crate::nested::Dict;
use rvt::ifc::entities::FillerSize;
use rvt::ifc::entities::{
    BrepTriangle, IfcEntity, Property, PropertySet, PropertyValue, SolidShape,
};
use rvt::ifc::{ExportConvention, IfcModel, MaterialInfo, MaterialTexture, RevitGlobalIds};
use skpwright::{Material, Model, Owner, Side, Value};
use std::collections::{HashMap, HashSet};

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
    /// Faces drawn in a textured material's image, with texture coordinates.
    pub textured_faces: usize,
    /// Faces of a textured material without an image (or a size) to place,
    /// drawn in the material's colour.
    pub textured_faces_flat: usize,
    /// Faces with no material on their front, of their own or inherited, drawn in their back's.
    pub faces_back_material: usize,
    /// Faces that could not be cut into triangles.
    pub faces_unfilled: usize,
    /// Attribute dictionaries written as property sets.
    pub attribute_sets: usize,
    /// Faces with no material of their own or inherited, drawn in the style's front colour.
    pub faces_default: usize,
    /// Elements written as the IFC class SketchUp classified their definition as.
    pub classified: usize,
    /// Elements classified as an IFC class the writer cannot write, written as a proxy
    /// (the classification stays in the `SKP_AppliedSchemaTypes` property set).
    pub classified_unwritten: usize,
    /// Classification attribute values written, read from the dictionaries a
    /// classification keeps inside its own.
    pub classification_values: usize,
}

/// Map `model` to an [`IfcModel`] named `name`.
pub fn skp_to_model(model: &Model, name: &str) -> (IfcModel, FromSkpStats) {
    skp_to_model_nested(model, name, &HashMap::new())
}

/// [`skp_to_model`] with the dictionaries inside dictionaries that skpwright's
/// model leaves out ([`crate::nested::read`]): a classification's attribute
/// values become properties, and its overall sizes and predefined type the
/// IFC attributes.
pub fn skp_to_model_nested(
    model: &Model,
    name: &str,
    nested: &HashMap<String, Vec<Dict>>,
) -> (IfcModel, FromSkpStats) {
    let mut w = Walk::new(model, nested);
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
            w.push_element("Model geometry".into(), &[], Some(mesh), 0, &[], &[], None);
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
    // A colourised texture is drawn as its image, without the tint.
    let texture = match m {
        Material::Textured {
            texture,
            applied_size_in,
            image_bytes: Some(bytes),
            ..
        } => MaterialTexture::new(
            bytes.clone(),
            texture.as_deref().map(file_name),
            applied_size_in.map(|(w, h)| [w / 12.0, h / 12.0]),
        ),
        _ => None,
    };
    MaterialInfo {
        name: name.clone(),
        color_packed: Some(u32::from(rgba[0]) | u32::from(rgba[1]) << 8 | u32::from(rgba[2]) << 16),
        transparency: (opacity < 1.0).then(|| (1.0 - opacity).clamp(0.0, 1.0)),
        texture,
    }
}

/// The last part of a texture's path, as SketchUp stores it (either slash).
fn file_name(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
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

/// One element's triangles: points in feet, triangles, a material per
/// triangle, and per point its texture coordinates (empty when no face is
/// textured).
struct Body {
    points: Vec<[f64; 3]>,
    triangles: Vec<BrepTriangle>,
    materials: Vec<usize>,
    uvs: Vec<[f64; 2]>,
}

struct Walk<'a> {
    m: &'a Model,
    out: IfcModel,
    st: FromSkpStats,
    slot_material: HashMap<u16, usize>,
    instance_name: HashMap<u32, &'a str>,
    definition_of: HashMap<u32, usize>,
    sets: HashMap<usize, Vec<PropertySet>>,
    /// The attribute dictionaries of placements, by run and index into its `placed`.
    instance_sets: HashMap<(usize, usize), Vec<PropertySet>>,
    /// GlobalIds given out, so that a placement seen twice (inside a definition
    /// placed twice) keeps its GUID once and the rest are derived.
    used_ids: HashSet<String>,
    /// The IFC class each classified definition names (`AppliedSchemaTypes`).
    classes: HashMap<usize, String>,
    /// What a classified definition's attribute values set on its IFC entity.
    class_attrs: HashMap<usize, ClassAttrs>,
    /// The material for unpainted faces, added on first use.
    default_material: Option<usize>,
}

impl<'a> Walk<'a> {
    fn new(m: &'a Model, nested: &HashMap<String, Vec<Dict>>) -> Self {
        let mut sets: HashMap<usize, Vec<PropertySet>> = HashMap::new();
        let mut instance_sets: HashMap<(usize, usize), Vec<PropertySet>> = HashMap::new();
        let mut classes: HashMap<usize, String> = HashMap::new();
        let mut class_attrs: HashMap<usize, ClassAttrs> = HashMap::new();
        let mut values_written = 0;
        let to_feet = length_to_feet(m.units.length_unit);
        // A flat dictionary as a property set, with the attribute values of the
        // dictionaries it holds, if any (`inner`, those of the same owner).
        let mut as_sets = |a: &skpwright::Attributes, inner: &[Dict]| -> Vec<PropertySet> {
            a.dictionaries
                .iter()
                .filter_map(|dict| {
                    let mut properties: Vec<Property> = dict
                        .entries
                        .iter()
                        .map(|(k, v)| Property {
                            name: k.clone(),
                            value: property_value(v),
                        })
                        .collect();
                    if let Some(tree) = inner.iter().find(|t| t.name == dict.name) {
                        let values = attribute_values(tree, to_feet);
                        values_written += values.len();
                        properties.extend(values);
                    }
                    (!properties.is_empty()).then(|| PropertySet {
                        name: format!("SKP_{}", dict.name),
                        properties,
                    })
                })
                .collect()
        };
        let inner_of = |guid: &str| nested.get(guid).map_or(&[][..], Vec::as_slice);
        for a in &m.dictionaries {
            match a.owner {
                Owner::Definition(d) => {
                    let inner = m.definitions.get(d).map_or(&[][..], |x| inner_of(&x.guid));
                    let applied = a.dictionaries.iter().find_map(applied_ifc_class);
                    if let Some((schema, class)) = applied {
                        if let Some(tree) = inner.iter().find(|t| t.name == schema) {
                            class_attrs.insert(d, ClassAttrs::of(tree, to_feet));
                        }
                        classes.insert(d, class);
                    }
                    let built = as_sets(a, inner);
                    sets.entry(d).or_default().extend(built);
                }
                Owner::Instance(run, k) => {
                    let inner = m
                        .geometry
                        .get(run)
                        .and_then(|r| r.placed.get(k))
                        .map_or(&[][..], |p| inner_of(&p.guid));
                    let built = as_sets(a, inner);
                    instance_sets.entry((run, k)).or_default().extend(built);
                }
                _ => {}
            }
        }
        Walk {
            m,
            out: IfcModel::default(),
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
            instance_sets,
            used_ids: HashSet::new(),
            classes,
            class_attrs,
            default_material: None,
            st: FromSkpStats {
                classification_values: values_written,
                ..Default::default()
            },
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
                texture: None,
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
        // A point of an untextured face is shared (`None`); a textured face
        // has its own, its coordinates being its own.
        let mut remap: HashMap<(u32, Option<usize>), u32> = HashMap::new();
        let mut body = Body {
            points: Vec::new(),
            triangles: Vec::new(),
            materials: Vec::new(),
            uvs: Vec::new(),
        };
        for (face_index, f) in mesh.faces.iter().enumerate() {
            if f.hidden || !self.layer_visible(f.layer) {
                self.st.faces_hidden += 1;
                continue;
            }
            let Some(tris) = triangulate(mesh, f) else {
                self.st.faces_unfilled += 1;
                continue;
            };
            self.st.faces += 1;
            // The front's own material, else the enclosing instance's (which
            // paints a front with none), else the back's own. One side is
            // written; the other side's own paint is not.
            let own = |s: Option<u16>| s.filter(|s| *s != 0 && self.slot_material.contains_key(s));
            let (slot, side) = match (
                own(f.front_material),
                own(Some(inherited)),
                own(f.back_material),
            ) {
                (Some(s), _, _) | (None, Some(s), _) => (s, Side::Front),
                (None, None, Some(s)) => {
                    self.st.faces_back_material += 1;
                    (s, Side::Back)
                }
                (None, None, None) => (0, Side::Front),
            };
            let material = match self.slot_material.get(&slot).copied().filter(|_| slot != 0) {
                Some(i) => i,
                None => {
                    self.st.faces_default += 1;
                    self.default_material()
                }
            };
            let textured = self
                .out
                .materials
                .get(material)
                .is_some_and(|i| i.texture.is_some());
            let uv = if textured {
                m.applied_size_of(slot)
                    .and_then(|size| f.uv_xform(side, size))
            } else {
                None
            };
            if uv.is_some() {
                self.st.textured_faces += 1;
            } else if matches!(m.materials.get(material), Some(Material::Textured { .. })) {
                self.st.textured_faces_flat += 1;
            }
            let key = uv.as_ref().map(|_| face_index);
            for t in tris {
                let mut ix = t.map(|v| {
                    *remap.entry((v, key)).or_insert_with(|| {
                        let local = mesh.vertices[v as usize];
                        let p = apply(world, local);
                        body.points.push([p[0] / FT, p[1] / FT, p[2] / FT]);
                        if let Some(x) = &uv {
                            body.uvs.resize(body.points.len() - 1, [0.0; 2]);
                            let [u, w] = x.apply(local);
                            body.uvs.push([u, w]);
                        }
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
        if !body.uvs.is_empty() {
            body.uvs.resize(body.points.len(), [0.0; 2]);
        }
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
        for (k, p) in self.m.geometry[run].placed.iter().enumerate() {
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
            let mut sets = def
                .and_then(|d| self.sets.get(&d).cloned())
                .unwrap_or_default();
            sets.extend(
                self.instance_sets
                    .get(&(run, k))
                    .cloned()
                    .unwrap_or_default(),
            );
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
            let class = def.and_then(|d| self.classes.get(&d)).cloned();
            let attrs = def.and_then(|d| self.class_attrs.get(&d)).cloned();
            if let Some(i) = self.push_element(name, &facts, own, p.pid, &kids, &sets, class) {
                if let Some(a) = attrs {
                    self.apply_class_attrs(i, &a);
                }
                // SketchUp's own IFC export names a component by its placement's
                // GUID; it gives a classified group none, so a group's is taken too.
                if let Some(id) =
                    placement_global_id(&p.guid).filter(|id| self.used_ids.insert(id.clone()))
                {
                    self.out.global_ids.elements.insert(i, id);
                }
                out.push(i);
            }
        }
        out
    }

    /// The overall sizes and predefined type a classification gives element `i`,
    /// where its written entity has those attributes.
    fn apply_class_attrs(&mut self, i: usize, a: &ClassAttrs) {
        let Some(IfcEntity::BuildingElement {
            ifc_type,
            predefined_type,
            ..
        }) = self.out.entities.get_mut(i)
        else {
            return;
        };
        if let Some(p) = a
            .predefined
            .as_deref()
            .filter(|p| ifc4_predefined(ifc_type, p))
        {
            *predefined_type = Some(p.to_string());
        }
        if matches!(ifc_type.as_str(), "IFCDOOR" | "IFCWINDOW")
            && let (Some(height_feet), Some(width_feet)) = (a.height_feet, a.width_feet)
        {
            self.out.filler_sizes.insert(
                i,
                FillerSize {
                    width_feet,
                    height_feet,
                    cut_loop: false,
                },
            );
        }
    }

    /// The element for a mesh, a set of children or both; `None` when it holds neither.
    #[allow(clippy::too_many_arguments)]
    fn push_element(
        &mut self,
        name: String,
        facts: &[(&str, PropertyValue)],
        body: Option<Body>,
        pid: u32,
        kids: &[usize],
        sets: &[PropertySet],
        class: Option<String>,
    ) -> Option<usize> {
        if body.is_none() && kids.is_empty() {
            return None;
        }
        let written = class.as_deref().and_then(writable_class);
        match (&class, written) {
            (Some(_), Some(_)) => self.st.classified += 1,
            (Some(_), None) => self.st.classified_unwritten += 1,
            _ => {}
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
                    uvs: b.uvs,
                };
                (Some(shape), uniform)
            }
            None => (None, None),
        };
        let index = self.out.entities.len();
        self.out.entities.push(IfcEntity::BuildingElement {
            ifc_type: written
                .unwrap_or(if kids.is_empty() {
                    "IFCBUILDINGELEMENTPROXY"
                } else {
                    "IFCELEMENTASSEMBLY"
                })
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

/// The IFC class an `AppliedSchemaTypes` dictionary names, as SketchUp records a
/// classification on a definition: the schema's name ("IFC 2x3", "IFC 4") to the
/// class ("IfcDoor"). Other schemas (national ones) are left to the property set.
fn applied_ifc_class(d: &skpwright::Dictionary) -> Option<(String, String)> {
    if d.name != "AppliedSchemaTypes" {
        return None;
    }
    d.entries.iter().find_map(|(schema, v)| match v {
        Value::Text(class)
            if schema.to_ascii_uppercase().starts_with("IFC")
                && class.to_ascii_uppercase().starts_with("IFC") =>
        {
            Some((schema.clone(), class.trim().to_string()))
        }
        _ => None,
    })
}

/// What a classification's attribute values give its IFC entity's own attributes.
#[derive(Debug, Default, Clone, PartialEq)]
struct ClassAttrs {
    height_feet: Option<f64>,
    width_feet: Option<f64>,
    /// Upper case, as IFC writes an enumerator.
    predefined: Option<String>,
}

impl ClassAttrs {
    fn of(tree: &Dict, to_feet: f64) -> ClassAttrs {
        let mut a = ClassAttrs::default();
        for (path, _, v) in leaves(tree) {
            let number = match v {
                Value::Double(x) => Some(*x),
                Value::Float(x) => Some(f64::from(*x)),
                Value::Int(x) => Some(f64::from(*x)),
                _ => None,
            };
            match (path.first().map(String::as_str), number, v) {
                (Some("OverallHeight"), Some(x), _) if x > 0.0 => a.height_feet = Some(x * to_feet),
                (Some("OverallWidth"), Some(x), _) if x > 0.0 => a.width_feet = Some(x * to_feet),
                (Some("PredefinedType"), _, Value::Text(t)) if !t.trim().is_empty() => {
                    a.predefined = Some(t.trim().to_ascii_uppercase());
                }
                _ => {}
            }
        }
        a
    }
}

/// Feet per unit of `length_unit` (skpwright's `Units`: 0 inches, 1 feet,
/// 2 millimetres, 3 centimetres, 4 metres). A classification's lengths are taken
/// to be in the model's unit, which no export of SketchUp's confirms (its own IFC
/// export leaves them out).
fn length_to_feet(length_unit: u32) -> f64 {
    match length_unit {
        0 => 1.0 / 12.0,
        1 => 1.0,
        2 => 1.0 / 304.8,
        3 => 1.0 / 30.48,
        _ => 1.0 / FT,
    }
}

/// The values set in a classification's tree: each leaf (a dictionary with a
/// `value`) under its path of dictionary names, with its `attribute_type`. The
/// `instanceAttributes` bookkeeping SketchUp keeps beside each is left out, as
/// are values left empty.
fn leaves(tree: &Dict) -> Vec<(Vec<String>, String, &Value)> {
    fn walk<'t>(
        d: &'t Dict,
        path: &mut Vec<String>,
        out: &mut Vec<(Vec<String>, String, &'t Value)>,
    ) {
        if let Some((_, v)) = d.entries.iter().find(|(k, _)| k == "value") {
            let set = match v {
                Value::Text(t) => !t.is_empty(),
                Value::Nil => false,
                _ => true,
            };
            if set {
                let kind = d
                    .entries
                    .iter()
                    .find_map(|(k, v)| match (k.as_str(), v) {
                        ("attribute_type", Value::Text(t)) => Some(t.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                out.push((path.clone(), kind, v));
            }
        }
        for c in d.children.iter().filter(|c| c.name != "instanceAttributes") {
            path.push(c.name.clone());
            walk(c, path, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    walk(tree, &mut Vec::new(), &mut out);
    out
}

/// A classification's set values as properties: named by the attribute (and
/// property set) names on the path, typed by its outermost IFC type
/// (`OverallHeight > IfcPositiveLengthMeasure > IfcLengthMeasure` is a positive
/// length). SketchUp's own IFC export writes them as labels, or not at all.
fn attribute_values(tree: &Dict, to_feet: f64) -> Vec<Property> {
    leaves(tree)
        .into_iter()
        .filter_map(|(path, kind, v)| {
            let names: Vec<&str> = path
                .iter()
                .map(String::as_str)
                .filter(|n| !n.starts_with("Ifc"))
                .collect();
            let name = if names.is_empty() {
                path.first()?.clone()
            } else {
                names.join(".")
            };
            let ifc_type = path
                .iter()
                .find(|n| n.starts_with("Ifc"))
                .map_or("", String::as_str);
            let number = match v {
                Value::Double(x) => Some(*x),
                Value::Float(x) => Some(f64::from(*x)),
                Value::Int(x) => Some(f64::from(*x)),
                _ => None,
            };
            let value = match (number, v) {
                (Some(x), _) if ifc_type.contains("PositiveLengthMeasure") => {
                    PropertyValue::PositiveLengthFeet(x * to_feet)
                }
                (Some(x), _) if ifc_type.contains("LengthMeasure") => {
                    PropertyValue::LengthFeet(x * to_feet)
                }
                (Some(_), Value::Int(i)) => PropertyValue::Integer(i64::from(*i)),
                (Some(x), _) => PropertyValue::Real(x),
                (None, Value::Text(t)) if ifc_type == "IfcText" => PropertyValue::Text(t.clone()),
                (None, Value::Text(t)) if ifc_type == "IfcIdentifier" => {
                    PropertyValue::Identifier(t.clone())
                }
                (None, Value::Text(t)) if kind == "string" && ifc_type != "IfcLabel" => {
                    PropertyValue::Text(t.clone())
                }
                (None, Value::Text(t)) => PropertyValue::Label(t.clone()),
                (None, other) => property_value(other),
            };
            Some(Property { name, value })
        })
        .collect()
}

/// Whether `token` is an enumerator of the IFC4 predefined type of `ifc_type`
/// (as written), for the classes SketchUp's classifier gives one; an IFC 2x3
/// enumerator IFC4 does not have, or any other class, keeps `NOTDEFINED`.
fn ifc4_predefined(ifc_type: &str, token: &str) -> bool {
    let list: &[&str] = match ifc_type {
        "IFCSLAB" => &["FLOOR", "ROOF", "LANDING", "BASESLAB"],
        "IFCWALL" => &[
            "MOVABLE",
            "PARAPET",
            "PARTITIONING",
            "PLUMBINGWALL",
            "SHEAR",
            "SOLIDWALL",
            "STANDARD",
            "POLYGONAL",
            "ELEMENTEDWALL",
        ],
        "IFCCOLUMN" => &["COLUMN", "PILASTER"],
        "IFCBEAM" => &[
            "BEAM",
            "JOIST",
            "HOLLOWCORE",
            "LINTEL",
            "SPANDREL",
            "T_BEAM",
        ],
        "IFCMEMBER" => &[
            "BRACE", "CHORD", "COLLAR", "MEMBER", "MULLION", "PLATE", "POST", "PURLIN", "RAFTER",
            "STRINGER", "STRUT", "STUD",
        ],
        "IFCPLATE" => &["CURTAIN_PANEL", "SHEET"],
        "IFCCOVERING" => &[
            "CEILING",
            "FLOORING",
            "CLADDING",
            "ROOFING",
            "MOLDING",
            "SKIRTINGBOARD",
            "INSULATION",
            "MEMBRANE",
            "SLEEVING",
            "WRAPPING",
        ],
        "IFCRAILING" => &["HANDRAIL", "GUARDRAIL", "BALUSTRADE"],
        "IFCSPACE" => &["SPACE", "PARKING", "GFA", "INTERNAL", "EXTERNAL"],
        "IFCDOOR" => &["DOOR", "GATE", "TRAPDOOR"],
        "IFCWINDOW" => &["WINDOW", "SKYLIGHT", "LIGHTDOME"],
        _ => &[],
    };
    list.contains(&token)
}

/// The IFC4 entity a classification is written as, where rvt-open's STEP writer
/// writes that entity's attributes in full; `None` for any other class (spatial
/// elements, openings, distribution classes the writer has no attribute list for,
/// IFC 2x3 classes IFC4 dropped), which stays a proxy.
fn writable_class(class: &str) -> Option<&'static str> {
    Some(match class.to_ascii_uppercase().as_str() {
        // IFC 2x3's standard and elemented cases are their base class in IFC4, which
        // holds no constraint on how the geometry is made.
        "IFCWALL" | "IFCWALLSTANDARDCASE" | "IFCWALLELEMENTEDCASE" => "IFCWALL",
        "IFCSLAB" | "IFCSLABSTANDARDCASE" | "IFCSLABELEMENTEDCASE" => "IFCSLAB",
        "IFCBEAM" | "IFCBEAMSTANDARDCASE" => "IFCBEAM",
        "IFCCOLUMN" | "IFCCOLUMNSTANDARDCASE" => "IFCCOLUMN",
        "IFCMEMBER" | "IFCMEMBERSTANDARDCASE" => "IFCMEMBER",
        "IFCPLATE" | "IFCPLATESTANDARDCASE" => "IFCPLATE",
        "IFCDOOR" | "IFCDOORSTANDARDCASE" => "IFCDOOR",
        "IFCWINDOW" | "IFCWINDOWSTANDARDCASE" => "IFCWINDOW",
        // Tag and PredefinedType, as the writer writes any class it does not list.
        "IFCBUILDINGELEMENTPROXY" => "IFCBUILDINGELEMENTPROXY",
        "IFCBUILDINGELEMENTPART" => "IFCBUILDINGELEMENTPART",
        "IFCCHIMNEY" => "IFCCHIMNEY",
        "IFCCOVERING" => "IFCCOVERING",
        "IFCCURTAINWALL" => "IFCCURTAINWALL",
        "IFCFOOTING" => "IFCFOOTING",
        "IFCRAILING" => "IFCRAILING",
        "IFCRAMP" => "IFCRAMP",
        "IFCRAMPFLIGHT" => "IFCRAMPFLIGHT",
        "IFCROOF" => "IFCROOF",
        "IFCSHADINGDEVICE" => "IFCSHADINGDEVICE",
        "IFCSTAIR" => "IFCSTAIR",
        "IFCDISCRETEACCESSORY" => "IFCDISCRETEACCESSORY",
        "IFCFASTENER" => "IFCFASTENER",
        "IFCTRANSPORTELEMENT" => "IFCTRANSPORTELEMENT",
        "IFCFURNITURE" => "IFCFURNITURE",
        "IFCSYSTEMFURNITUREELEMENT" => "IFCSYSTEMFURNITUREELEMENT",
        "IFCGEOGRAPHICELEMENT" => "IFCGEOGRAPHICELEMENT",
        "IFCDISTRIBUTIONCHAMBERELEMENT" => "IFCDISTRIBUTIONCHAMBERELEMENT",
        // Classes the writer lists with their own attributes.
        "IFCFURNISHINGELEMENT" => "IFCFURNISHINGELEMENT",
        "IFCFLOWCONTROLLER" => "IFCFLOWCONTROLLER",
        "IFCFLOWTERMINAL" => "IFCFLOWTERMINAL",
        "IFCFLOWSEGMENT" => "IFCFLOWSEGMENT",
        "IFCSTAIRFLIGHT" => "IFCSTAIRFLIGHT",
        "IFCELEMENTASSEMBLY" => "IFCELEMENTASSEMBLY",
        // A space, as SketchUp's own IFC export writes it.
        "IFCSPACE" => "IFCSPACE",
        _ => return None,
    })
}

/// A placement's GUID (hex of its 16 stored bytes, Windows byte order) as the
/// 22-character GlobalId SketchUp's IFC export writes for it; `None` for none.
fn placement_global_id(hex: &str) -> Option<String> {
    if hex.len() != 32 {
        return None;
    }
    let mut b = [0u8; 16];
    for (i, slot) in b.iter_mut().enumerate() {
        *slot = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    if b == [0; 16] {
        return None;
    }
    // The first three fields are stored little-endian.
    b[0..4].reverse();
    b[4..6].reverse();
    b[6..8].reverse();
    const DIGITS: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";
    let n = u128::from_be_bytes(b);
    Some(
        (0..22)
            .rev()
            .map(|k| char::from(DIGITS[((n >> (6 * k)) & 63) as usize]))
            .collect(),
    )
}

fn property_value(v: &Value) -> PropertyValue {
    match v {
        Value::Text(s) => PropertyValue::Text(s.clone()),
        Value::Int(i) => PropertyValue::Integer(i64::from(*i)),
        Value::Float(f) => PropertyValue::Real(f64::from(*f)),
        Value::Double(f) => PropertyValue::Real(*f),
        Value::Bool(b) => PropertyValue::Boolean(*b),
        Value::Time(t) => PropertyValue::Integer(i64::from(*t)),
        Value::Array(items) => {
            PropertyValue::Text(items.iter().map(value_text).collect::<Vec<_>>().join(", "))
        }
        other => PropertyValue::Text(value_text(other)),
    }
}

/// A value as text: an array's items one after another, a point its coordinates.
fn value_text(v: &Value) -> String {
    match v {
        Value::Nil => String::new(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Time(t) => t.to_string(),
        Value::Text(s) => s.clone(),
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(value_text).collect::<Vec<_>>().join(", ")
        ),
        Value::Bytes8(b) => b.iter().map(|x| format!("{x:02x}")).collect(),
        Value::Vector(p) | Value::Direction(p) => format!("{}, {}, {}", p[0], p[1], p[2]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied(entries: &[(&str, &str)]) -> skpwright::Dictionary {
        skpwright::Dictionary {
            name: "AppliedSchemaTypes".into(),
            entries: entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), Value::Text((*v).to_string())))
                .collect(),
            flag: 0,
        }
    }

    #[test]
    fn classifications_map_to_ifc4_entities() {
        let class = |e: &[(&str, &str)]| applied_ifc_class(&applied(e)).map(|(_, c)| c);
        assert_eq!(class(&[("IFC 2x3", "IfcDoor")]).as_deref(), Some("IfcDoor"));
        assert_eq!(
            class(&[("Uniclass", "Pr_30"), ("IFC 4", "IfcWall")]).as_deref(),
            Some("IfcWall")
        );
        assert_eq!(class(&[("Uniclass", "Pr_30")]), None);
        let mut other = applied(&[("IFC 2x3", "IfcDoor")]);
        other.name = "IFC 2x3".into();
        assert_eq!(applied_ifc_class(&other), None);

        assert_eq!(writable_class("IfcDoor"), Some("IFCDOOR"));
        assert_eq!(writable_class("IfcWallStandardCase"), Some("IFCWALL"));
        assert_eq!(writable_class("IFCSLAB"), Some("IFCSLAB"));
        assert_eq!(writable_class("IfcSpace"), Some("IFCSPACE"));
        // Spatial structure, dropped in IFC4, or attributes the writer does not write.
        for c in [
            "IfcBuildingStorey",
            "IfcElectricalElement",
            "IfcPile",
            "IfcFlowFitting",
        ] {
            assert_eq!(writable_class(c), None, "{c}");
        }
    }

    #[test]
    fn a_placement_guid_is_sketchups_globalid() {
        // As SketchUp 2020 stores them and writes them in its IFC export.
        assert_eq!(
            placement_global_id("f11657414e974e4ea53a77e1a1c5ed6e").as_deref(),
            Some("11LnRnbqvEJgKwT_6XnUrk")
        );
        assert_eq!(
            placement_global_id("d4a394cccc23a1488fcc93b9441f7e16").as_deref(),
            Some("3CbAFK8yn8eO$Caxb47tuM")
        );
        assert_eq!(placement_global_id(""), None);
        assert_eq!(placement_global_id(&"0".repeat(32)), None);
        assert_eq!(placement_global_id(&"g".repeat(32)), None);
    }

    fn leaf(name: &str, kind: &str, value: Value) -> Dict {
        Dict {
            name: name.into(),
            entries: vec![
                ("attribute_type".into(), Value::Text(kind.into())),
                ("value".into(), value),
            ],
            children: vec![Dict {
                name: "instanceAttributes".into(),
                children: vec![Dict {
                    name: "pos".into(),
                    entries: vec![("value".into(), Value::Text("1".into()))],
                    ..Default::default()
                }],
                ..Default::default()
            }],
        }
    }

    fn node(name: &str, children: Vec<Dict>) -> Dict {
        Dict {
            name: name.into(),
            children,
            ..Default::default()
        }
    }

    #[test]
    fn classification_values_become_typed_properties_and_attributes() {
        let length = |name: &str, x: f64| {
            node(
                name,
                vec![node(
                    "IfcPositiveLengthMeasure",
                    vec![leaf("IfcLengthMeasure", "double", Value::Double(x))],
                )],
            )
        };
        let tree = node(
            "IFC 2x3",
            vec![
                length("OverallHeight", 2.1),
                length("OverallWidth", 0.9),
                node(
                    "Name",
                    vec![leaf("IfcLabel", "string", Value::Text("D-01".into()))],
                ),
                node(
                    "Tag",
                    vec![leaf("IfcIdentifier", "string", Value::Text(String::new()))],
                ),
                node(
                    "PredefinedType",
                    vec![leaf(
                        "IfcSlabTypeEnum",
                        "enumeration",
                        Value::Text("floor".into()),
                    )],
                ),
            ],
        );
        let to_feet = length_to_feet(4);
        let props = attribute_values(&tree, to_feet);
        let names: Vec<&str> = props.iter().map(|p| p.name.as_str()).collect();
        // The empty Tag and the bookkeeping are left out.
        assert_eq!(
            names,
            ["OverallHeight", "OverallWidth", "Name", "PredefinedType"]
        );
        match &props[0].value {
            PropertyValue::PositiveLengthFeet(f) => assert!((f * FT - 2.1).abs() < 1e-9),
            other => panic!("{other:?}"),
        }
        assert!(matches!(&props[2].value, PropertyValue::Label(t) if t == "D-01"));
        let a = ClassAttrs::of(&tree, to_feet);
        assert!((a.height_feet.unwrap() * FT - 2.1).abs() < 1e-9);
        assert!((a.width_feet.unwrap() * FT - 0.9).abs() < 1e-9);
        assert_eq!(a.predefined.as_deref(), Some("FLOOR"));
        assert!(ifc4_predefined("IFCSLAB", "FLOOR"));
        assert!(!ifc4_predefined("IFCWALL", "FLOOR"));
        assert!(!ifc4_predefined("IFCFURNITURE", "CHAIR"));
        assert!((length_to_feet(0) - 1.0 / 12.0).abs() < 1e-12);
        let list = Value::Array(vec![Value::Text("SketchUp".into()), Value::Int(2)]);
        assert!(matches!(property_value(&list), PropertyValue::Text(t) if t == "SketchUp, 2"));
    }
}
