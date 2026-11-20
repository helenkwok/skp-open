//! RVT to SKP, over public models (fetched by tools/ci/fetch-fixtures.sh): the file written
//! reads back through skpwright with the counts and the box of what was written.

mod common;

use skp_open::{SkpTarget, counts, rvt_to_skp, write_skp};

#[test]
fn every_public_model_writes_a_skp_that_reads_back() {
    let Some(dir) = common::fixtures("rvt") else {
        return;
    };
    let files = common::files(&dir, "rvt");
    assert!(!files.is_empty(), "no .rvt in {}", dir.display());
    let out = common::scratch("rvt");
    let (mut with_faces, mut total_faces) = (0usize, 0usize);
    for f in &files {
        let rel = f.strip_prefix(&dir).unwrap().display().to_string();
        let (model, st, notes) = rvt_to_skp(f).unwrap_or_else(|e| panic!("{rel}: {e:#}"));
        let written = counts(&model);
        assert_eq!(written.faces, st.faces, "{rel}: {st:?}");
        assert!(
            st.triangles_in >= st.faces + st.two_sided_duplicates,
            "{rel}: {st:?}"
        );
        for (label, target) in [
            ("2026", SkpTarget::Container2026),
            ("2017", SkpTarget::parse("2017").unwrap()),
        ] {
            let p = out.join(format!(
                "{}-{label}.skp",
                f.file_stem().unwrap().to_string_lossy()
            ));
            let wn =
                write_skp(&model, &p, target).unwrap_or_else(|e| panic!("{rel} as {label}: {e:#}"));
            let read =
                skpwright::Model::read(&p).unwrap_or_else(|e| panic!("{rel} as {label}: {}", e.0));
            assert!(
                read.diagnostics.iter().all(|d| !d.is_desync()),
                "{rel} as {label}: {:?}",
                read.diagnostics
            );
            let c = counts(&read);
            assert_eq!(c.faces, written.faces, "{rel} as {label}: faces");
            assert_eq!(
                c.definitions, written.definitions,
                "{rel} as {label}: definitions"
            );
            assert_eq!(
                c.materials, written.materials,
                "{rel} as {label}: materials"
            );
            assert_eq!(c.layers, written.layers, "{rel} as {label}: layers");
            assert_eq!(c.vertices, written.vertices, "{rel} as {label}: vertices");
            let (shown, bb) = common::shown(&read);
            assert_eq!(shown, st.faces, "{rel} as {label}: faces shown");
            if let (Some(a), Some(b)) = (st.source_bbox, bb) {
                assert!(
                    common::close(a, b, 1e-5),
                    "{rel} as {label}: graph {a:?}, skp {b:?}"
                );
            }
            println!("RVT {rel} -> {label}: {c:?} writer notes {}", wn.len());
        }
        println!("RVT {rel}: {st:?} export notes {}", notes.len());
        if st.faces > 0 {
            with_faces += 1;
            total_faces += st.faces;
        }
    }
    println!(
        "RVT total: {} files, {with_faces} with faces, {total_faces} faces",
        files.len()
    );
    std::fs::remove_dir_all(&out).unwrap();
}
