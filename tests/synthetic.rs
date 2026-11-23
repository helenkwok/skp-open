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
