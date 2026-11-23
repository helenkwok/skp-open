//! A model built in code, through every writer, and back: no fixtures needed.

mod common;

use rvt::ifc::{GraphOptions, IfcxGraph};
use skp_open::{Format, SkpTarget, counts, from_skp, to_skp, write_format, write_skp};

#[test]
fn a_built_model_converts_to_every_format() {
    let skp = common::built_read_back();
    let (faces, bb) = common::shown(&skp);
    assert_eq!(faces, 6 + 2 * (6 + 1), "faces of the source");
    let (model, st) = from_skp::skp_to_model(&skp, "built");
    assert_eq!(st.faces, faces);
    assert_eq!(st.triangles, 52, "{st:?}");
    assert_eq!(st.elements, 5, "root box, two parts, two plates: {st:?}");
    assert_eq!(st.instances_skipped, 0);
    // The parts' cubes and the plates have no material: SketchUp draws them in the
    // style's front colour, not the writers' category grey.
    assert_eq!(st.faces_default, 2 * (6 + 1), "{st:?}");
    let front = skp
        .rendering
        .as_ref()
        .map_or([255, 255, 255, 255], |r| r.front_color);
    let default = model.materials.last().expect("a default material");
    assert_eq!(default.name, "Default");
    assert_eq!(
        default.color_packed,
        Some(u32::from(front[0]) | u32::from(front[1]) << 8 | u32::from(front[2]) << 16)
    );
    assert_eq!(model.materials.len(), skp.materials.len() + 1);

    let dir = common::scratch("synthetic");
    let mut sizes = Vec::new();
    for (ext, f) in [
        ("ifc", Format::Ifc),
        ("ifcx", Format::Ifcx),
        ("glb", Format::Glb),
        ("obj", Format::Obj),
        ("usda", Format::Usda),
        ("usdc", Format::Usdc),
        ("frag", Format::Frag),
        ("dxf", Format::Dxf),
    ] {
        let out = dir.join(format!("built.{ext}"));
        write_format(&model, f, &out).unwrap_or_else(|e| panic!("{ext}: {e:#}"));
        let n = std::fs::metadata(&out).unwrap().len();
        assert!(n > 0, "{ext} is empty");
        sizes.push((ext, n));
    }
    println!("sizes {sizes:?}");

    let step = std::fs::read_to_string(dir.join("built.ifc")).unwrap();
    assert_eq!(
        step.matches("IFCTRIANGULATEDFACESET(").count(),
        5,
        "one face set per element"
    );
    assert!(
        step.matches("IFCRELAGGREGATES(").count() >= 2,
        "the parts aggregate their plates"
    );
    assert!(step.contains("IFCELEMENTASSEMBLY("));

    let ifcx: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("built.ifcx")).unwrap()).unwrap();
    assert!(
        ifcx["data"].as_array().is_some_and(|d| !d.is_empty()),
        "ifcx has nodes"
    );

    let glb = std::fs::read(dir.join("built.glb")).unwrap();
    assert_eq!(&glb[..4], b"glTF");
    assert_eq!(
        &std::fs::read(dir.join("built.usdc")).unwrap()[..8],
        b"PXR-USDC"
    );

    let obj = std::fs::read_to_string(dir.join("built.obj")).unwrap();
    assert_eq!(
        obj.lines().filter(|l| l.starts_with("o ")).count(),
        5,
        "one OBJ object per element"
    );
    let obj_faces = obj.lines().filter(|l| l.starts_with("f ")).count();
    // An open mesh (the plate) is drawn from both sides, so its triangles are written twice.
    assert!(obj_faces >= 52, "OBJ triangles {obj_faces}");
    let dxf = std::fs::read_to_string(dir.join("built.dxf")).unwrap();
    assert_eq!(dxf.matches("\n3DFACE\n").count(), 52);
    assert!(
        dxf.contains("\nParts\n"),
        "the SketchUp layer names the DXF layer"
    );
    std::fs::remove_dir_all(&dir).unwrap();

    // Back to SKP through rvt-open's graph: every triangle a face, the box the same.
    let graph = IfcxGraph::from_model(
        &model,
        GraphOptions {
            meshes: true,
            shared_shapes: false,
        },
    );
    let (back, st2) = to_skp::graph_to_skp(&graph);
    assert_eq!(
        st2.triangles_in, 68,
        "the plates are drawn from both sides: {st2:?}"
    );
    assert_eq!(st2.two_sided_duplicates, 16, "{st2:?}");
    assert_eq!(st2.faces, 52, "{st2:?}");
    for target in [SkpTarget::Container2026, SkpTarget::parse("2017").unwrap()] {
        let dir = common::scratch("synthetic-back");
        let out = dir.join("back.skp");
        write_skp(&back, &out, target).unwrap();
        let read = skpwright::Model::read(&out).unwrap();
        assert!(
            read.diagnostics.iter().all(|d| !d.is_desync()),
            "{:?}",
            read.diagnostics
        );
        let c = counts(&read);
        assert_eq!(c.faces, 52, "{target:?}");
        assert_eq!(c.definitions, st2.groups, "{target:?}");
        assert!(
            read.definitions.iter().any(|d| d.name == "Part"),
            "names survive"
        );
        let (f, bb2) = common::shown(&read);
        assert_eq!(f, 52);
        assert!(
            common::close(bb.unwrap(), bb2.unwrap(), 1e-5),
            "{bb:?} {bb2:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// A definition SketchUp classified (its `AppliedSchemaTypes` dictionary) is written
/// as that IFC class; one the writer cannot write stays what it was. Both containers.
#[test]
fn classified_definitions_keep_their_ifc_class() {
    use skpwright::{Attributes, Dictionary, Owner, Value};
    let mut built = common::built();
    let applied = |class: &str| Dictionary {
        name: "AppliedSchemaTypes".into(),
        entries: vec![("IFC 2x3".into(), Value::Text(class.into()))],
        flag: 0,
    };
    // "Plate" is a door; "Part", which places the plate, a storey (no element).
    built.dictionaries = vec![
        Attributes {
            owner: Owner::Definition(1),
            dictionaries: vec![applied("IfcDoor")],
        },
        Attributes {
            owner: Owner::Definition(0),
            dictionaries: vec![applied("IfcBuildingStorey")],
        },
    ];
    for target in [SkpTarget::Container2026, SkpTarget::parse("2017").unwrap()] {
        let dir = common::scratch("classified");
        let path = dir.join("classified.skp");
        write_skp(&built, &path, target).unwrap();
        let skp = skpwright::Model::read(&path).unwrap();
        let (model, st) = from_skp::skp_to_model(&skp, "classified");
        assert_eq!(st.classified, 2, "the two plates, {target:?}: {st:?}");
        assert_eq!(
            st.classified_unwritten, 2,
            "the two parts, {target:?}: {st:?}"
        );
        let out = dir.join("classified.ifc");
        write_format(&model, Format::Ifc, &out).unwrap();
        let step = std::fs::read_to_string(&out).unwrap();
        assert_eq!(step.matches("IFCDOOR(").count(), 2, "{target:?}");
        assert_eq!(step.matches("IFCELEMENTASSEMBLY(").count(), 2, "{target:?}");
        assert_eq!(
            step.matches("IFCBUILDINGELEMENTPROXY(").count(),
            1,
            "the root box, {target:?}"
        );
        assert!(step.contains("'SKP_AppliedSchemaTypes'"), "{target:?}");
        assert!(
            step.contains("'IfcBuildingStorey'"),
            "the classification is kept, {target:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// A 2 x 2 RGB PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xfd, 0xd4, 0x9a,
    0x73, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0x00, 0x04,
    0x20, 0xe2, 0x3f, 0x10, 0x00, 0x00, 0x1e, 0xef, 0x05, 0xfb, 0x35, 0x81, 0xa0, 0xc0, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn textured_faces_carry_their_image_and_coordinates() {
    use rvt::ifc::entities::{IfcEntity, SolidShape};
    let mut skp = common::built();
    // The red becomes an image 1 m wide and 2 m tall per repeat; the plate is
    // painted on its back only.
    skp.materials[0] = skpwright::Material::Textured {
        name: "Checker".into(),
        texture: Some("C:\\maps\\checker.png".into()),
        applied_size_in: Some((1.0 / 0.0254, 2.0 / 0.0254)),
        jpeg_quality: None,
        image_bytes: Some(PNG.to_vec()),
        avg_rgba: Some([128, 128, 128, 255]),
        colorized: false,
        texture_avg_rgba: None,
        colorize_type: 0,
        opacity: 1.0,
    };
    skp.geometry[0].mesh.faces[0].back_material = Some(1);
    let (model, st) = from_skp::skp_to_model(&skp, "textured");
    assert_eq!(
        st.textured_faces,
        6 + 2,
        "the box, and each plate by its back: {st:?}"
    );
    assert_eq!(st.faces_back_material, 2, "{st:?}");
    assert_eq!(st.faces_default, 2 * 6, "{st:?}");
    let t = model.materials[0].texture.as_ref().expect("the image");
    assert_eq!(t.name.as_deref(), Some("checker.png"));
    assert_eq!(t.mime_type, "image/png");
    let size = t.size_feet.expect("size");
    assert!((size[0] - 1.0 / 0.3048).abs() < 1e-9 && (size[1] - 2.0 / 0.3048).abs() < 1e-9);

    let body = model.entities.iter().find_map(|e| match e {
        IfcEntity::BuildingElement {
            name,
            solid_shape:
                Some(SolidShape::TriangulatedFaceSet {
                    vertices_feet, uvs, ..
                }),
            ..
        } if name == "Model geometry" => Some((vertices_feet, uvs)),
        _ => None,
    });
    let (points, uvs) = body.expect("the box");
    assert_eq!(points.len(), 6 * 4, "a textured face keeps its own points");
    assert_eq!(uvs.len(), points.len());
    // The top face (z = 1 m) maps x across 1 m and y across 2 m: its corner
    // (1, 1, 1) is at (1, 0.5). The side faces meeting there keep their own.
    let ft = 1.0 / 0.3048;
    let corner: Vec<[f64; 2]> = points
        .iter()
        .zip(uvs)
        .filter(|(p, _)| p.iter().all(|c| (c - ft).abs() < 1e-9))
        .map(|(_, uv)| *uv)
        .collect();
    assert_eq!(corner.len(), 3, "one point per face at the corner");
    assert!(
        corner
            .iter()
            .any(|uv| (uv[0] - 1.0).abs() < 1e-9 && (uv[1] - 0.5).abs() < 1e-9),
        "{corner:?}"
    );

    let dir = common::scratch("textured");
    write_format(&model, Format::Glb, &dir.join("t.glb")).unwrap();
    let glb = std::fs::read(dir.join("t.glb")).unwrap();
    let len = u32::from_le_bytes([glb[12], glb[13], glb[14], glb[15]]) as usize;
    let doc: serde_json::Value = serde_json::from_slice(&glb[20..20 + len]).unwrap();
    assert_eq!(doc["images"].as_array().map(Vec::len), Some(1));
    let notes = write_format(&model, Format::Obj, &dir.join("t.obj")).unwrap();
    assert!(
        notes.iter().any(|n| n.contains("1 texture images")),
        "{notes:?}"
    );
    let mtl = std::fs::read_to_string(dir.join("t.mtl")).unwrap();
    let image = mtl
        .lines()
        .find_map(|l| l.strip_prefix("map_Kd "))
        .expect("map_Kd");
    assert_eq!(std::fs::read(dir.join(image)).unwrap(), PNG);
    write_format(&model, Format::Usda, &dir.join("t.usda")).unwrap();
    let usda = std::fs::read_to_string(dir.join("t.usda")).unwrap();
    assert!(usda.contains("UsdUVTexture") && usda.contains("primvars:st"));
    assert_eq!(
        std::fs::read(dir.join("textures").join(image)).unwrap(),
        PNG
    );
    let step = {
        write_format(&model, Format::Ifc, &dir.join("t.ifc")).unwrap();
        std::fs::read_to_string(dir.join("t.ifc")).unwrap()
    };
    assert!(step.contains("IFCBLOBTEXTURE(") && step.contains("IFCINDEXEDTRIANGLETEXTUREMAP("));

    // IFCX: the main file keeps the colours; a layer beside it carries the
    // coordinates, one [s, t] per point of each textured mesh, and the image.
    let x = dir.join("ifcx");
    std::fs::create_dir_all(&x).unwrap();
    let notes = write_format(&model, Format::Ifcx, &x.join("t.ifcx")).unwrap();
    assert!(
        notes.iter().any(|n| n.contains("1 texture images")),
        "{notes:?}"
    );
    let read = |p: &std::path::Path| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
    };
    let main = read(&x.join("t.ifcx"));
    let layer = read(&x.join("t.textures.ifcx"));
    assert_eq!(
        layer["header"]["id"],
        format!("{}/textures", main["header"]["id"].as_str().unwrap())
    );
    let data = layer["data"].as_array().expect("layer data");
    assert!(!data.is_empty(), "the textured meshes");
    for n in data {
        let path = &n["path"];
        let points = main["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| &m["path"] == path)
            .and_then(|m| m["attributes"]["usd::usdgeom::mesh"]["points"].as_array())
            .map(Vec::len);
        let st = n["attributes"][rvt::ifc::TEXTURE_COORDINATES_KEY]
            .as_array()
            .map(Vec::len);
        assert!(st.is_some() && st == points, "{path}: {st:?} vs {points:?}");
        let file = n["attributes"][rvt::ifc::GLTF_MATERIAL_KEY]["pbrMetallicRoughness"]
            ["baseColorTexture"]
            .as_str()
            .expect("image");
        assert_eq!(std::fs::read(x.join(file)).unwrap(), PNG);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
