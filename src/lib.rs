//! skp-open: `.skp` to IFC, IFCX, glTF, OBJ, USD, Fragments and DXF, and `.rvt` to `.skp`.
//!
//! `.skp` is read and written by skpwright, `.rvt` read and every other format
//! written by rvt-open. This crate maps one model to the other in both directions
//! ([`from_skp`], [`to_skp`]) and drives the writers.

pub mod dxf;
pub mod from_skp;
pub mod to_skp;

use anyhow::{Context, Result, bail};
use rvt::ifc::IfcModel;
use skpwright::{Model, Release};
use std::path::Path;

/// What a `.skp` holds, summed over every definition and the model's own entities.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SkpCounts {
    pub definitions: usize,
    pub placements: usize,
    pub vertices: usize,
    pub edges: usize,
    pub faces: usize,
    pub materials: usize,
    pub layers: usize,
}

/// The counts of `m`, as skpwright reads it.
pub fn counts(m: &Model) -> SkpCounts {
    SkpCounts {
        definitions: m.definitions.len(),
        placements: m.geometry.iter().map(|r| r.placed.len()).sum(),
        vertices: m.geometry.iter().map(|r| r.mesh.vertices.len()).sum(),
        edges: m.geometry.iter().map(|r| r.mesh.edges.len()).sum(),
        faces: m.geometry.iter().map(|r| r.mesh.faces.len()).sum(),
        materials: m.materials.len(),
        layers: m.layers.len(),
    }
}

/// The formats a `.skp` converts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Ifc,
    Ifcx,
    Glb,
    Obj,
    Usda,
    Usdc,
    Frag,
    Dxf,
}

impl Format {
    pub fn from_path(p: &Path) -> Option<Format> {
        Some(
            match p.extension()?.to_str()?.to_ascii_lowercase().as_str() {
                "ifc" => Format::Ifc,
                "ifcx" => Format::Ifcx,
                "glb" => Format::Glb,
                "obj" => Format::Obj,
                "usd" | "usda" => Format::Usda,
                "usdc" => Format::Usdc,
                "frag" => Format::Frag,
                "dxf" => Format::Dxf,
                _ => return None,
            },
        )
    }
}

/// Write `model` as `format` to `out` (an OBJ also writes its `.mtl` beside it).
/// Returns notes on what the format left out.
pub fn write_format(model: &IfcModel, format: Format, out: &Path) -> Result<Vec<String>> {
    let mut notes = Vec::new();
    match format {
        Format::Ifc => std::fs::write(out, rvt::ifc::write_step(model))?,
        Format::Ifcx => std::fs::write(out, rvt::ifc::ifcx_writer::write_ifcx(model)?)?,
        Format::Glb => std::fs::write(out, rvt::ifc::gltf::model_to_glb(model))?,
        Format::Obj => {
            let mtl = out.with_extension("mtl");
            let mtl_name = mtl
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("model.mtl");
            let (obj, mtl_text) = rvt::ifc::obj::model_to_obj(model, mtl_name);
            std::fs::write(out, obj)?;
            std::fs::write(&mtl, mtl_text)?;
        }
        Format::Usda => std::fs::write(out, rvt::ifc::write_usda(model)?)?,
        Format::Usdc => std::fs::write(out, rvt::ifc::write_usdc(model)?)?,
        Format::Frag => {
            let graph = rvt::ifc::IfcxGraph::from_model(model, rvt::ifc::GraphOptions::default());
            let (bytes, _) = rvt::ifc::frag_writer::write_frag_graph(&graph)?;
            std::fs::write(out, bytes)?;
        }
        Format::Dxf => {
            let (text, faces) = dxf::write_dxf(model);
            notes.push(format!("{faces} 3DFACE entities, metres"));
            std::fs::write(out, text)?;
        }
    }
    Ok(notes)
}

/// Read a `.skp` and map it to rvt-open's model.
pub fn skp_to_model(path: &Path) -> Result<(IfcModel, from_skp::FromSkpStats)> {
    let m = Model::read(path).map_err(|e| anyhow::anyhow!("{}: {}", path.display(), e.0))?;
    let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
    Ok(from_skp::skp_to_model(&m, name))
}

/// Which `.skp` to write: the 2026 container (SketchUp 2026 and 2027, which 2021 to 2025
/// also open) or a release of the stream container (2013 to 2020, 3 to 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkpTarget {
    Container2026,
    Release(Release),
}

impl SkpTarget {
    /// `2026` or `2027` for the 2026 container, a year 2013 to 2020 or a number 3 to 8 for a release.
    pub fn parse(s: &str) -> Result<SkpTarget> {
        let n: u32 = s
            .parse()
            .with_context(|| format!("{s:?} is not a release"))?;
        match n {
            2026 | 2027 => Ok(SkpTarget::Container2026),
            n => match Release::of_name(n) {
                Some(r) => Ok(SkpTarget::Release(r)),
                None => bail!("{s:?} is no release: use 2026, or 2013 to 2020, or 3 to 8"),
            },
        }
    }
}

/// Write a model as a `.skp`; the notes name what the target could not hold.
pub fn write_skp(m: &Model, out: &Path, target: SkpTarget) -> Result<Vec<String>> {
    match target {
        SkpTarget::Container2026 => {
            let (bytes, notes) = m
                .to_archive(skpwright::ArchiveTarget::ByContent)
                .map_err(|e| anyhow::anyhow!("{}", e.0))?;
            std::fs::write(out, bytes)?;
            Ok(notes)
        }
        SkpTarget::Release(r) => m.write_as(out, r).map_err(|e| anyhow::anyhow!("{}", e.0)),
    }
}

/// Read a `.rvt` (or `.rfa`, `.rte`, `.rft`) and map it to a `.skp` model. The notes are
/// the ones rvt-open's export left.
pub fn rvt_to_skp(path: &Path) -> Result<(Model, to_skp::ToSkpStats, Vec<String>)> {
    let mut rf = rvt::RevitFile::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut opts = rvt::convert::ConvertOptions::rvt_ifc_defaults();
    opts.per_element_meshes = true;
    let exported = rvt::convert::export(&mut rf, &opts)?;
    let graph = rvt::convert::graph(&exported.model, &opts, true);
    let (model, stats) = to_skp::graph_to_skp(&graph);
    Ok((model, stats, exported.notes))
}
