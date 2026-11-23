//! SKP to every format, over skpwright's corpus (fetched by tools/ci/fetch-fixtures.sh).
//! Counts and bounding boxes are held against skpwright's own scene walk of the same file.

mod common;

use skp_open::{Format, from_skp, write_format};

#[test]
fn every_corpus_file_converts_with_its_counts_and_box() {
    let Some(dir) = common::fixtures("skp") else {
        return;
    };
    let files = common::files(&dir, "skp");
    assert!(!files.is_empty(), "no .skp in {}", dir.display());
    let out = common::scratch("corpus");
    let (mut with_faces, mut triangles, mut elements) = (0usize, 0usize, 0usize);
    for f in &files {
        let rel = f.strip_prefix(&dir).unwrap().display().to_string();
        let skp = skpwright::Model::read(f).unwrap_or_else(|e| panic!("{rel}: {}", e.0));
        let (faces, bb) = common::shown(&skp);
        let (model, st) = from_skp::skp_to_model(&skp, "corpus");
        assert_eq!(
            st.faces, faces,
            "{rel}: faces drawn against faces shown {st:?}"
        );
        assert!(st.triangles >= st.faces, "{rel}: {st:?}");
        assert_eq!(st.faces_unfilled, 0, "{rel}: {st:?}");
        // The box of the points the face sets hold, in metres.
        let mut got: Option<common::Bbox> = None;
        for e in &model.entities {
            if let rvt::ifc::entities::IfcEntity::BuildingElement {
                solid_shape:
                    Some(rvt::ifc::entities::SolidShape::TriangulatedFaceSet { vertices_feet, .. }),
                ..
            } = e
            {
                for v in vertices_feet {
                    let p = v.map(|c| c * 0.3048);
                    let b = got.get_or_insert((p, p));
                    for k in 0..3 {
                        b.0[k] = b.0[k].min(p[k]);
                        b.1[k] = b.1[k].max(p[k]);
                    }
                }
            }
        }
        match (bb, got) {
            (Some(a), Some(b)) => assert!(
                common::close(a, b, 1e-6),
                "{rel}: skpwright {a:?}, converted {b:?}"
            ),
            (None, None) => {}
            (a, b) => panic!("{rel}: skpwright {a:?}, converted {b:?}"),
        }
        if st.triangles > 0 {
            with_faces += 1;
            // Every format writes; the text ones are counted.
            let name = f.file_stem().unwrap().to_string_lossy().replace(' ', "_");
            for (ext, fmt) in [
                ("ifc", Format::Ifc),
                ("ifcx", Format::Ifcx),
                ("glb", Format::Glb),
                ("obj", Format::Obj),
                ("usdc", Format::Usdc),
                ("dxf", Format::Dxf),
            ] {
                let p = out.join(format!("{name}.{ext}"));
                write_format(&model, fmt, &p).unwrap_or_else(|e| panic!("{rel} as {ext}: {e:#}"));
                assert!(std::fs::metadata(&p).unwrap().len() > 0, "{rel} as {ext}");
            }
            let obj = std::fs::read_to_string(out.join(format!("{name}.obj"))).unwrap();
            assert!(
                obj.lines().filter(|l| l.starts_with("f ")).count() >= st.triangles,
                "{rel}: OBJ triangles"
            );
            let dxf = std::fs::read_to_string(out.join(format!("{name}.dxf"))).unwrap();
            assert_eq!(
                dxf.matches("\n3DFACE\n").count(),
                st.triangles,
                "{rel}: DXF faces"
            );
            let step = std::fs::read_to_string(out.join(format!("{name}.ifc"))).unwrap();
            // One face set per element, except that a textured body drawing several
            // materials is one face set per material (an IFC4 texture map lays one
            // image on a whole face set).
            let textured = |m: &usize| model.materials.get(*m).is_some_and(|i| i.texture.is_some());
            let face_sets: usize = model
                .entities
                .iter()
                .map(|e| match e {
                    rvt::ifc::entities::IfcEntity::BuildingElement {
                        solid_shape:
                            Some(rvt::ifc::entities::SolidShape::TriangulatedFaceSet {
                                vertices_feet,
                                face_materials,
                                uvs,
                                ..
                            }),
                        ..
                    } => {
                        let mut drawn = face_materials.clone();
                        drawn.sort_unstable();
                        drawn.dedup();
                        let split = !uvs.is_empty()
                            && uvs.len() == vertices_feet.len()
                            && drawn.len() >= 2
                            && drawn.iter().any(textured);
                        if split { drawn.len() } else { 1 }
                    }
                    rvt::ifc::entities::IfcEntity::BuildingElement {
                        solid_shape: Some(_),
                        ..
                    } => 1,
                    _ => 0,
                })
                .sum();
            assert_eq!(
                step.matches("IFCTRIANGULATEDFACESET(").count(),
                face_sets,
                "{rel}: IFC face sets"
            );
            serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(out.join(format!("{name}.ifcx"))).unwrap(),
            )
            .unwrap_or_else(|e| panic!("{rel}: IFCX is not JSON: {e}"));
            assert_eq!(
                &std::fs::read(out.join(format!("{name}.glb"))).unwrap()[..4],
                b"glTF"
            );
            triangles += st.triangles;
            elements += st.elements;
        }
        println!(
            "SKP {rel}: faces {} triangles {} elements {} hidden faces {} skipped instances {} textured faces {} attribute sets {}",
            st.faces,
            st.triangles,
            st.elements,
            st.faces_hidden,
            st.instances_skipped,
            st.textured_faces,
            st.attribute_sets
        );
    }
    println!(
        "SKP total: {} files, {with_faces} with faces, {elements} elements, {triangles} triangles",
        files.len()
    );
    assert!(with_faces * 2 > files.len(), "most corpus files hold faces");
    std::fs::remove_dir_all(&out).unwrap();
}
