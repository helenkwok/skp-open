//! A 3D DXF (release 12, ASCII) of an [`IfcModel`]: one `3DFACE` per triangle,
//! on a layer named after the SketchUp layer the element was on, in metres.

use rvt::ifc::IfcModel;
use rvt::ifc::entities::{IfcEntity, PropertyValue, SolidShape};
use std::collections::BTreeSet;
use std::fmt::Write;

const FT: f64 = 0.3048;

fn layer_name(s: &str) -> String {
    let n: String = s
        .chars()
        .map(|c| {
            if "<>/\\\":;?*|,=`".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    if n.trim().is_empty() { "0".into() } else { n }
}

/// DXF text, and the number of faces written.
pub fn write_dxf(model: &IfcModel) -> (String, usize) {
    let mut layers = BTreeSet::new();
    let mut body = String::new();
    let mut faces = 0;
    for e in &model.entities {
        let IfcEntity::BuildingElement {
            solid_shape:
                Some(SolidShape::TriangulatedFaceSet {
                    vertices_feet,
                    triangles,
                    ..
                }),
            property_set,
            ..
        } = e
        else {
            continue;
        };
        let layer = property_set
            .as_ref()
            .and_then(|s| s.properties.iter().find(|p| p.name == "Layer"))
            .and_then(|p| match &p.value {
                PropertyValue::Text(t) => Some(layer_name(t)),
                _ => None,
            })
            .unwrap_or_else(|| "0".into());
        for t in triangles {
            let p = |i: u32| vertices_feet.get(i as usize).map(|v| v.map(|c| c * FT));
            let (Some(a), Some(b), Some(c)) = (p(t.0), p(t.1), p(t.2)) else {
                continue;
            };
            let _ = write!(body, "0\n3DFACE\n8\n{layer}\n");
            for (k, v) in [a, b, c, c].iter().enumerate() {
                let _ = write!(
                    body,
                    "{}\n{}\n{}\n{}\n{}\n{}\n",
                    10 + k,
                    v[0],
                    20 + k,
                    v[1],
                    30 + k,
                    v[2]
                );
            }
            faces += 1;
        }
        layers.insert(layer);
    }
    layers.insert("0".into());
    let mut out = String::from("0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009\n0\nENDSEC\n");
    let _ = write!(
        out,
        "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n{}\n",
        layers.len()
    );
    for l in &layers {
        let _ = write!(out, "0\nLAYER\n2\n{l}\n70\n0\n62\n7\n6\nCONTINUOUS\n");
    }
    out.push_str("0\nENDTAB\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n");
    out.push_str(&body);
    out.push_str("0\nENDSEC\n0\nEOF\n");
    (out, faces)
}
