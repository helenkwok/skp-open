use anyhow::{Result, bail};
use skp_open::{Format, SkpTarget};
use std::path::Path;

const USAGE: &str = "skp-open convert <in> <out> [--release 2026|2017|2016|...]
skp-open info <file>

convert  .rvt|.rfa|.rte|.rft -> .skp   (--release: 2026 container by default, or a year 2013 to 2020)
         .skp -> .ifc .ifcx .glb .obj .usda .usdc .frag .dxf
info     counts of a .skp, or the identity of a .rvt";

fn main() {
    if let Err(e) = run() {
        eprintln!("skp-open: {e:#}");
        std::process::exit(1);
    }
}

fn ext(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["info", file] => info(Path::new(file)),
        ["convert", input, output] => convert(
            Path::new(input),
            Path::new(output),
            SkpTarget::Container2026,
        ),
        ["convert", input, output, "--release", r] => {
            convert(Path::new(input), Path::new(output), SkpTarget::parse(r)?)
        }
        _ => bail!("{USAGE}"),
    }
}

fn info(p: &Path) -> Result<()> {
    if ext(p) == "skp" {
        let m = skpwright::Model::read(p).map_err(|e| anyhow::anyhow!("{}", e.0))?;
        let c = skp_open::counts(&m);
        println!("{}: version {}, {:?}", p.display(), m.version, m.container);
        println!("{c:?}");
    } else {
        let mut rf = rvt::RevitFile::open(p)?;
        println!("{}", rvt::convert::info_json(&mut rf, false)?);
    }
    Ok(())
}

fn convert(input: &Path, output: &Path, target: SkpTarget) -> Result<()> {
    match (ext(input).as_str(), ext(output).as_str()) {
        ("rvt" | "rfa" | "rte" | "rft", "skp") => {
            let (m, st, notes) = skp_open::rvt_to_skp(input)?;
            for n in notes {
                eprintln!("note: {n}");
            }
            for n in skp_open::write_skp(&m, output, target)? {
                eprintln!("note: {n}");
            }
            eprintln!("{st:?}");
            eprintln!("{:?}", skp_open::counts(&m));
        }
        ("skp", _) => {
            let Some(f) = Format::from_path(output) else {
                bail!(
                    "cannot write {}: use .ifc .ifcx .glb .obj .usda .usdc .frag or .dxf",
                    output.display()
                );
            };
            let (model, st) = skp_open::skp_to_model(input)?;
            for n in skp_open::write_format(&model, f, output)? {
                eprintln!("note: {n}");
            }
            eprintln!("{st:?}");
        }
        _ => bail!("{USAGE}"),
    }
    Ok(())
}
