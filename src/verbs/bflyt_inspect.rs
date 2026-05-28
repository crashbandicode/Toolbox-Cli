use anyhow::{Context, Result};
use clap::Parser;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::bflyt::{read_bflyt, BasePane, PaneKind, BFLYT};

#[derive(Parser, Debug)]
pub struct Args {
    /// Path to the .bflyt file.
    #[arg(short, long)]
    input: PathBuf,

    /// Emit JSON instead of human-readable text.
    #[arg(long)]
    json: bool,

    /// Indent JSON output. Pass --no-indent for compact JSON.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    indent: bool,
}

pub fn run(args: Args) -> Result<ExitCode> {
    let bytes = fs::read(&args.input)
        .with_context(|| format!("reading {}", args.input.display()))?;
    let bflyt = read_bflyt(&bytes).map_err(|e| anyhow::anyhow!("{}", e))?;
    let doc = build_document(&bflyt, &args.input, bytes.len());

    if args.json {
        if args.indent {
            println!("{}", serde_json::to_string_pretty(&doc)?);
        } else {
            println!("{}", serde_json::to_string(&doc)?);
        }
    } else {
        print_summary(&doc, &bflyt);
    }
    Ok(ExitCode::SUCCESS)
}

fn build_document(b: &BFLYT, path: &std::path::Path, file_size: usize) -> Value {
    let mut panes = Vec::new();
    if let Some(root) = &b.root_pane {
        collect_panes(root, None, &b.materials, &mut panes);
    }

    let materials: Vec<Value> = b
        .materials
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let refs: Vec<Value> = m
                .texture_maps
                .iter()
                .enumerate()
                .map(|(slot, tr)| {
                    json!({
                        "slot": slot,
                        "texture_index": tr.index,
                        "texture_name": b
                            .textures
                            .get(tr.index as usize)
                            .cloned()
                            .unwrap_or_default(),
                        "wrap_s": tr.wrap_mode_u,
                        "wrap_t": tr.wrap_mode_v,
                    })
                })
                .collect();
            json!({
                "index": i,
                "name": m.name,
                "white_color": [m.white_color.r, m.white_color.g, m.white_color.b, m.white_color.a],
                "black_color": [m.black_color.r, m.black_color.g, m.black_color.b, m.black_color.a],
                "texture_refs": refs,
            })
        })
        .collect();

    let textures: Vec<Value> = b
        .textures
        .iter()
        .enumerate()
        .map(|(i, name)| json!({"index": i, "name": name}))
        .collect();

    json!({
        "path": path.display().to_string(),
        "file_size": file_size,
        "endian": "little",
        "version": format!(
            "{}.{}.{}.{}",
            (b.version >> 24) & 0xff,
            (b.version >> 16) & 0xff,
            (b.version >> 8) & 0xff,
            b.version & 0xff
        ),
        "section_kinds": [
            {"kind": "lyt1", "present": true},
            {"kind": "txl1", "count": b.textures.len()},
            {"kind": "fnl1", "count": b.fonts.len()},
            {"kind": "mat1", "count": b.materials.len()},
        ],
        "texture_list": textures,
        "fonts": b.fonts,
        "materials": materials,
        "panes": panes,
        "counts": {
            "panes": count_panes(b.root_pane.as_ref()),
            "materials": b.materials.len(),
            "textures": b.textures.len(),
        },
    })
}

fn collect_panes(p: &BasePane, parent: Option<&str>, materials: &[crate::bflyt::Material], out: &mut Vec<Value>) {
    let kind = match p.kind {
        PaneKind::Pane => "pan1",
        PaneKind::Picture => "pic1",
        PaneKind::Text => "txt1",
        PaneKind::Window => "wnd1",
        PaneKind::Parts => "prt1",
        PaneKind::Bounding => "bnd1",
    };

    let (mat_idx, mat_name) = match (&p.picture, &p.text) {
        (Some(pic), _) => (
            Some(pic.material_index as i32),
            materials.get(pic.material_index as usize).map(|m| m.name.clone()),
        ),
        (_, Some(t)) => (
            Some(t.material_index as i32),
            materials.get(t.material_index as usize).map(|m| m.name.clone()),
        ),
        _ => (None, None),
    };

    let mut entry = json!({
        "kind": kind,
        "name": p.name,
        "parent": parent,
        "visible": p.visible(),
        "alpha": p.alpha,
        "translate": [p.translate.x, p.translate.y, p.translate.z],
        "scale": [p.scale.x, p.scale.y],
        "size": [p.width, p.height],
    });
    if let Some(m) = mat_idx {
        entry["material_index"] = json!(m);
        entry["material_name"] = json!(mat_name);
    }
    out.push(entry);

    for child in &p.children {
        collect_panes(child, Some(&p.name), materials, out);
    }
}

fn count_panes(root: Option<&BasePane>) -> usize {
    fn rec(p: &BasePane) -> usize {
        1 + p.children.iter().map(rec).sum::<usize>()
    }
    root.map(rec).unwrap_or(0)
}

fn print_summary(doc: &Value, b: &BFLYT) {
    println!("{}", doc["path"]);
    println!("  file_size = {}", doc["file_size"]);
    println!("  version   = {}", doc["version"]);
    println!("  panes     = {}", count_panes(b.root_pane.as_ref()));
    println!("  materials = {}", b.materials.len());
    println!("  textures  = {}", b.textures.len());
    println!("  fonts     = {}", b.fonts.len());
    println!("  (use --json for the full structured view)");
}
