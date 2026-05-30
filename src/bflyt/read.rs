//! BFLYT v8 parser. Reads a complete file from a slice of bytes into the
//! in-memory representation defined in `sections.rs`.
//!
//! The format is forgiving in one respect: section sizes are explicit, so
//! we can advance to the next section by `current_offset + section.size`
//! even if our parser doesn't fully understand a section's contents. We
//! exploit that to keep the implementation small — pane sub-section bytes
//! we don't model (e.g. the per-character transform inside txt1) are
//! captured verbatim and rewritten unchanged.

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read, Seek, SeekFrom};

use super::sections::*;

const MAGIC_FLYT: [u8; 4] = *b"FLYT";

/// Parse a BFLYT v8 file from raw bytes.
pub fn read_bflyt(data: &[u8]) -> Result<BFLYT, BflytError> {
    if data.len() < 0x14 {
        return Err(BflytError::Format("file is too small to be a BFLYT".into()));
    }

    let mut hdr = Cursor::new(&data[..0x14]);
    let mut magic = [0u8; 4];
    hdr.read_exact(&mut magic)?;
    if magic != MAGIC_FLYT {
        return Err(BflytError::BadMagic(magic));
    }
    let bom = hdr.read_u16::<LittleEndian>()?;
    if bom != 0xFEFF {
        return Err(BflytError::BadBom(bom));
    }
    let _header_size = hdr.read_u16::<LittleEndian>()?;
    let version = hdr.read_u32::<LittleEndian>()?;
    let _file_size = hdr.read_u32::<LittleEndian>()?;
    let section_count = hdr.read_u16::<LittleEndian>()?;
    let _padding = hdr.read_u16::<LittleEndian>()?;

    // Switch BFLYT major version is 8 (older Switch titles) or 9 (Smash
    // Ultimate / newer). Both share the same parsing logic per the
    // Switch-Toolbox source (Cafe MAT1 uses `if (VersionMajor >= 8)` for
    // the v8+ field ordering). We accept anything >= 8 and reject older
    // (Wii U is v5, 3DS is v7).
    let version_major = (version >> 24) & 0xff;
    if version_major < 8 {
        return Err(BflytError::UnsupportedVersion(version));
    }

    let mut bflyt = BFLYT {
        version,
        layout: LayoutInfo {
            draw_centered: false,
            width: 0.0,
            height: 0.0,
            max_parts_width: 0.0,
            max_parts_height: 0.0,
            name: String::new(),
        },
        textures: Vec::new(),
        fonts: Vec::new(),
        materials: Vec::new(),
        root_pane: None,
        root_group: None,
        user_data: None,
        control_data: None,
        opaque_sections: Vec::new(),
        trailing_sections: Vec::new(),
    };

    // Set once the pane tree is done (first grp1 or cnt1). After this,
    // `usd1` / unknown sections are trailing, not pane-attached.
    let mut past_panes = false;

    // Pane parsing uses a flat arena keyed by index, mirroring the C#
    // `currentPane` + `parentPane` scalar pair. This avoids fighting the
    // borrow checker with parent pointers while preserving the pas1/pae1
    // semantics from Switch-Toolbox `Header.Read`:
    //   pan1/pic1/...  -> add as child of `parent_idx`, becomes `current_idx`
    //   pas1           -> `parent_idx = current_idx`
    //   pae1           -> `current_idx = parent_idx; parent_idx = arena[current_idx].parent`
    let mut arena: Vec<(BasePane, Option<usize>)> = Vec::new();
    let mut current_idx: Option<usize> = None;
    let mut parent_idx: Option<usize> = None;
    let mut root_idx: Option<usize> = None;

    // Group parsing uses the same arena approach.
    let mut group_arena: Vec<(Group, Option<usize>)> = Vec::new();
    let mut current_group: Option<usize> = None;
    let mut parent_group: Option<usize> = None;
    let mut root_group_idx: Option<usize> = None;

    let mut offset = 0x14usize;
    for sec_idx in 0..section_count {
        if offset + 8 > data.len() {
            return Err(BflytError::Format(format!(
                "section table truncated at 0x{offset:x}"
            )));
        }
        let mut sec_magic = [0u8; 4];
        sec_magic.copy_from_slice(&data[offset..offset + 4]);
        let sec_size = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;
        if sec_size < 8 || offset + sec_size > data.len() {
            return Err(BflytError::Format(format!(
                "section {:?} at 0x{offset:x} has invalid size 0x{sec_size:x}",
                std::str::from_utf8(&sec_magic).unwrap_or("?")
            )));
        }
        let payload = &data[offset + 8..offset + sec_size];

        let magic_str = std::str::from_utf8(&sec_magic).unwrap_or("?").to_string();
        let context = |e: BflytError| -> BflytError {
            BflytError::Format(format!(
                "section[{sec_idx}] '{magic_str}' at file offset 0x{offset:x}: {e}"
            ))
        };

        match &sec_magic {
            b"lyt1" => bflyt.layout = read_lyt1(payload).map_err(context)?,
            b"txl1" => bflyt.textures = read_string_list(payload).map_err(context)?,
            b"fnl1" => bflyt.fonts = read_string_list(payload).map_err(context)?,
            b"mat1" => bflyt.materials = read_mat1(payload).map_err(context)?,

            b"pan1" | b"pic1" | b"txt1" | b"wnd1" | b"prt1" | b"bnd1" => {
                let kind = match &sec_magic {
                    b"pan1" => PaneKind::Pane,
                    b"pic1" => PaneKind::Picture,
                    b"txt1" => PaneKind::Text,
                    b"wnd1" => PaneKind::Window,
                    b"prt1" => PaneKind::Parts,
                    b"bnd1" => PaneKind::Bounding,
                    _ => unreachable!(),
                };
                let pane = read_pane(kind, payload, sec_size as u32).map_err(context)?;
                let idx = arena.len();
                arena.push((pane, parent_idx));
                if root_idx.is_none() {
                    root_idx = Some(idx);
                }
                current_idx = Some(idx);
            }

            b"pas1" => {
                parent_idx = current_idx;
            }

            b"pae1" => {
                // Move "up" one level. After this, the most recently
                // closed pane is the new `current_idx`, and that pane's
                // parent (if any) becomes the new `parent_idx`.
                if let Some(p) = parent_idx {
                    current_idx = Some(p);
                    parent_idx = arena[p].1;
                }
            }

            b"grp1" => {
                past_panes = true;
                let g = read_group(payload)?;
                let idx = group_arena.len();
                group_arena.push((g, parent_group));
                if root_group_idx.is_none() {
                    root_group_idx = Some(idx);
                }
                current_group = Some(idx);
            }
            b"grs1" => parent_group = current_group,
            b"gre1" => {
                if let Some(p) = parent_group {
                    current_group = Some(p);
                    parent_group = group_arena[p].1;
                }
            }

            b"usd1" => {
                // A usd1 after the pane/group tree (e.g. trailing after
                // cnt1) is not a pane's user_data — preserve it as a
                // trailing section. Otherwise: file-level usd1 (no current
                // pane) → BFLYT.user_data; per-pane usd1 → current pane.
                if past_panes {
                    bflyt.trailing_sections.push(OpaqueSection {
                        magic: sec_magic,
                        payload: payload.to_vec(),
                        after_pane_name: None,
                    });
                } else if current_idx.is_none() {
                    bflyt.user_data = Some(UserData {
                        raw: payload.to_vec(),
                    });
                } else if let Some(idx) = current_idx {
                    arena[idx].0.user_data = Some(UserData {
                        raw: payload.to_vec(),
                    });
                }
            }

            b"cnt1" => {
                // Control data section (Smash Ultimate player layouts).
                // We don't decode it yet; preserve the bytes verbatim.
                past_panes = true;
                bflyt.control_data = Some(UserData {
                    raw: payload.to_vec(),
                });
            }

            // Any section we don't explicitly decode is preserved
            // verbatim, keeping the parser tolerant rather than
            // hard-failing (cross-game generality). Two cases:
            //
            // - Inside the pane tree (a pane has already been seen):
            //   `scr1`/`ali1`/`spi1` (Smash) and unknown sections are
            //   real panes that can open their own `pas1`/children/`pae1`
            //   scope, so we push them as opaque PANE nodes. Flattening
            //   them to anchored sections would unbalance the nesting and
            //   drop `pas1`/`pae1` (seen on TotK layouts where
            //   `pan1 pas1 ali1 pas1 ... ` nests under `ali1`).
            // - Before the pane tree (file-level): e.g. TotK's `ctl1`
            //   between `mat1` and the first pane. Captured as a
            //   file-level `OpaqueSection`, re-emitted right before the
            //   root pane (byte-identical).
            //
            // (`lyt1/txl1/fnl1/mat1/pan1/.../grp1/usd1/cnt1` are handled
            // above; this arm only sees genuinely unmodeled magics.)
            _ => {
                if past_panes {
                    // After the pane/group tree (e.g. alongside cnt1).
                    bflyt.trailing_sections.push(OpaqueSection {
                        magic: sec_magic,
                        payload: payload.to_vec(),
                        after_pane_name: None,
                    });
                } else if current_idx.is_some() {
                    let pane = BasePane::opaque(sec_magic, payload.to_vec());
                    let idx = arena.len();
                    arena.push((pane, parent_idx));
                    if root_idx.is_none() {
                        root_idx = Some(idx);
                    }
                    current_idx = Some(idx);
                } else {
                    bflyt.opaque_sections.push(OpaqueSection {
                        magic: sec_magic,
                        payload: payload.to_vec(),
                        after_pane_name: None,
                    });
                }
            }
        }

        offset += sec_size;
    }

    // Convert the arena into a nested tree.
    if let Some(root) = root_idx {
        bflyt.root_pane = Some(materialize_tree(arena, root));
    }
    if let Some(root) = root_group_idx {
        bflyt.root_group = Some(materialize_group_tree(group_arena, root));
    }

    Ok(bflyt)
}

fn materialize_tree(mut arena: Vec<(BasePane, Option<usize>)>, root: usize) -> BasePane {
    // Build child lists by walking the arena once: every non-root entry
    // is a child of its parent, in the order they appeared in the file.
    let mut child_lists: Vec<Vec<usize>> = (0..arena.len()).map(|_| Vec::new()).collect();
    for (i, (_, parent)) in arena.iter().enumerate() {
        if let Some(p) = parent {
            child_lists[*p].push(i);
        }
    }
    fn build(
        idx: usize,
        arena: &mut Vec<(BasePane, Option<usize>)>,
        child_lists: &mut Vec<Vec<usize>>,
    ) -> BasePane {
        let children_indices = std::mem::take(&mut child_lists[idx]);
        let mut node = std::mem::replace(
            &mut arena[idx].0,
            BasePane {
                kind: PaneKind::Pane,
                flag: 0,
                base_position: 0,
                alpha: 0,
                flag_ex: 0,
                name: String::new(),
                user_data_field: [0; PANE_USER_DATA_LEN],
                translate: Vec3::default(),
                rotate: Vec3::default(),
                scale: Vec2::default(),
                width: 0.0,
                height: 0.0,
                picture: None,
                text: None,
                window: None,
                parts: None,
                user_data: None,
                children: Vec::new(),
                trailing: Vec::new(),
                opaque: None,
            },
        );
        for ci in children_indices {
            node.children.push(build(ci, arena, child_lists));
        }
        node
    }
    build(root, &mut arena, &mut child_lists)
}

fn materialize_group_tree(mut arena: Vec<(Group, Option<usize>)>, root: usize) -> Group {
    let mut child_lists: Vec<Vec<usize>> = (0..arena.len()).map(|_| Vec::new()).collect();
    for (i, (_, parent)) in arena.iter().enumerate() {
        if let Some(p) = parent {
            child_lists[*p].push(i);
        }
    }
    fn build(
        idx: usize,
        arena: &mut Vec<(Group, Option<usize>)>,
        child_lists: &mut Vec<Vec<usize>>,
    ) -> Group {
        let children_indices = std::mem::take(&mut child_lists[idx]);
        let mut node = std::mem::take(&mut arena[idx].0);
        for ci in children_indices {
            node.children.push(build(ci, arena, child_lists));
        }
        node
    }
    build(root, &mut arena, &mut child_lists)
}

// ---------- lyt1 ----------

fn read_lyt1(payload: &[u8]) -> Result<LayoutInfo, BflytError> {
    if payload.len() < 0x14 {
        return Err(BflytError::TruncatedSection("lyt1".into()));
    }
    let mut c = Cursor::new(payload);
    let draw_centered = c.read_u8()? != 0;
    let mut padding = [0u8; 3];
    c.read_exact(&mut padding)?;
    let width = c.read_f32::<LittleEndian>()?;
    let height = c.read_f32::<LittleEndian>()?;
    let max_parts_width = c.read_f32::<LittleEndian>()?;
    let max_parts_height = c.read_f32::<LittleEndian>()?;
    let name = read_cstring_until_null(&mut c)?;
    Ok(LayoutInfo {
        draw_centered,
        width,
        height,
        max_parts_width,
        max_parts_height,
        name,
    })
}

// ---------- txl1 / fnl1 ----------

fn read_string_list(payload: &[u8]) -> Result<Vec<String>, BflytError> {
    // Layout (Switch v8):
    //   u16 count
    //   u16 padding
    //   u32[count] offsets, each relative to the FIRST byte after count+padding
    //   ... null-terminated strings ...
    if payload.len() < 4 {
        return Err(BflytError::TruncatedSection("txl1/fnl1".into()));
    }
    let mut c = Cursor::new(payload);
    let count = c.read_u16::<LittleEndian>()? as usize;
    let _padding = c.read_u16::<LittleEndian>()?;
    let table_base = 4usize; // offsets are measured from this position
    if table_base + count * 4 > payload.len() {
        return Err(BflytError::TruncatedSection(
            "txl1/fnl1 offset table".into(),
        ));
    }
    let mut offsets = Vec::with_capacity(count);
    for _ in 0..count {
        offsets.push(c.read_u32::<LittleEndian>()? as usize);
    }
    let mut out = Vec::with_capacity(count);
    for off in offsets {
        let abs = table_base + off;
        if abs >= payload.len() {
            return Err(BflytError::TruncatedSection(format!(
                "string offset 0x{off:x} out of bounds"
            )));
        }
        out.push(read_null_terminated(&payload[abs..])?);
    }
    Ok(out)
}

// ---------- mat1 ----------

fn read_mat1(payload: &[u8]) -> Result<Vec<Material>, BflytError> {
    if payload.len() < 4 {
        return Err(BflytError::TruncatedSection("mat1".into()));
    }
    let mut c = Cursor::new(payload);
    let count = c.read_u16::<LittleEndian>()? as usize;
    let _padding = c.read_u16::<LittleEndian>()?;

    let mut offsets = Vec::with_capacity(count);
    for _ in 0..count {
        offsets.push(c.read_u32::<LittleEndian>()? as usize);
    }

    // Convert file-absolute offsets to payload-relative positions and
    // compute each material's exact size from the next entry (or from the
    // section end for the last material). We pass the size into
    // `read_material` so it can capture trailing bytes for v9-specific
    // sub-sections we don't decode yet.
    let starts: Vec<usize> = offsets
        .iter()
        .map(|o| o.checked_sub(8).unwrap_or(0))
        .collect();
    let mut materials = Vec::with_capacity(count);
    for i in 0..count {
        let start = starts[i];
        let end = starts.get(i + 1).copied().unwrap_or(payload.len());
        if start > payload.len() || end > payload.len() || end < start {
            return Err(BflytError::Format(format!(
                "mat1 material[{i}] offset range [{start:x}..{end:x}] is invalid"
            )));
        }
        let mut mc = Cursor::new(&payload[start..end]);
        let mat = read_material(&mut mc, end - start).map_err(|e| {
            BflytError::Format(format!(
                "mat1 material[{i}] (size {} bytes): {e}",
                end - start
            ))
        })?;
        materials.push(mat);
    }

    Ok(materials)
}

fn read_material<R: Read + Seek>(r: &mut R, expected_size: usize) -> Result<Material, BflytError> {
    let mut name_bytes = [0u8; MAT_NAME_LEN];
    r.read_exact(&mut name_bytes)?;
    let name = parse_fixed_name(&name_bytes);

    // v8 layout: flags (4) + unknown (4) + black (4) + white (4)
    let flags_raw = r.read_u32::<LittleEndian>()?;
    let flags_unknown = r.read_u32::<LittleEndian>()?;
    let black_color = Color8::read(r)?;
    let white_color = Color8::read(r)?;

    let mut tex_count = (flags_raw & 0x3) as usize;
    let mut mtx_count = ((flags_raw >> 2) & 0x3) as usize;
    let mut tex_coord_gen_count = ((flags_raw >> 4) & 0x3) as usize;
    let mut tev_stage_count = ((flags_raw >> 6) & 0x7) as usize;
    let mut has_alpha_compare = ((flags_raw >> 9) & 0x1) != 0;
    let mut has_blend = ((flags_raw >> 10) & 0x1) != 0;
    let mut has_blend_logic = ((flags_raw >> 12) & 0x1) != 0;
    let mut has_indirect = ((flags_raw >> 14) & 0x1) != 0;
    let mut proj_tex_gen_count = ((flags_raw >> 15) & 0x3) as usize;
    let mut has_font_shadow = ((flags_raw >> 17) & 0x1) != 0;

    // Defensive sub-section budget check. Some community-mod BFLYTs (HDR
    // training-modpack mat1) have flag bits that don't match the actual
    // byte budget --- the C# Switch-Toolbox tolerates this by reading
    // available bytes and ignoring the rest. We do the same: if the
    // flag-implied sizes exceed `expected_size`, scale down sub-section
    // counts to fit. The dropped bytes (if any) get captured in
    // `Material.trailing` and re-emitted verbatim.
    let header_bytes = MAT_NAME_LEN + 4 + 4 + 4 + 4; // name + flags + unk + black + white = 44
    let budget = expected_size.saturating_sub(header_bytes);
    let demanded = tex_count * 4
        + mtx_count * 20
        + tex_coord_gen_count * 16
        + tev_stage_count * 4
        + if has_alpha_compare { 8 } else { 0 }
        + if has_blend { 4 } else { 0 }
        + if has_blend_logic { 4 } else { 0 }
        + if has_indirect { 12 } else { 0 }
        + proj_tex_gen_count * 20
        + if has_font_shadow { 8 } else { 0 };
    let flags_untrusted = demanded > budget;
    if demanded > budget {
        // Heuristic recovery: clamp the highest-count fields first.
        // Empirically, mtx_count is the most common offender on HDR
        // mods. This recovery only runs on malformed input.
        let mut shrink = demanded - budget;
        for (count_ref, size) in [
            (&mut mtx_count, 20),
            (&mut tex_coord_gen_count, 16),
            (&mut tex_count, 4),
            (&mut tev_stage_count, 4),
            (&mut proj_tex_gen_count, 20),
        ]
        .iter_mut()
        {
            while shrink >= *size && **count_ref > 0 {
                **count_ref -= 1;
                shrink -= *size;
            }
        }
        for (flag, size) in [
            (&mut has_alpha_compare, 8usize),
            (&mut has_blend, 4),
            (&mut has_blend_logic, 4),
            (&mut has_indirect, 12),
            (&mut has_font_shadow, 8),
        ] {
            if shrink >= size && *flag {
                *flag = false;
                shrink -= size;
            }
        }
    }

    let mut texture_maps = Vec::with_capacity(tex_count);
    for _ in 0..tex_count {
        let index = r.read_i16::<LittleEndian>()?;
        let wu = r.read_u8()?;
        let wv = r.read_u8()?;
        texture_maps.push(TextureRef {
            index,
            wrap_mode_u: wu,
            wrap_mode_v: wv,
        });
    }

    let mut texture_transforms = Vec::with_capacity(mtx_count);
    for _ in 0..mtx_count {
        let translate = Vec2::read(r)?;
        let rotate = r.read_f32::<LittleEndian>()?;
        let scale = Vec2::read(r)?;
        texture_transforms.push(TextureTransform {
            translate,
            rotate,
            scale,
        });
    }

    let mut tex_coord_gens = Vec::with_capacity(tex_coord_gen_count);
    for _ in 0..tex_coord_gen_count {
        let mut raw = [0u8; 16];
        r.read_exact(&mut raw)?;
        tex_coord_gens.push(TexCoordGen {
            matrix_type: raw[0],
            source: raw[1],
            unk: [raw[2], raw[3]],
            raw,
        });
    }

    let mut tev_stages = Vec::with_capacity(tev_stage_count);
    for _ in 0..tev_stage_count {
        let cb = r.read_u8()?;
        let ab = r.read_u8()?;
        let mut unk = [0u8; 2];
        r.read_exact(&mut unk)?;
        tev_stages.push(TevStage {
            color_blend: cb,
            alpha_blend: ab,
            unk,
        });
    }

    let alpha_compare = if has_alpha_compare {
        let function = r.read_u8()?;
        let mut pad = [0u8; 3];
        r.read_exact(&mut pad)?;
        let reference = r.read_f32::<LittleEndian>()?;
        Some(AlphaCompare {
            function,
            _padding: pad,
            reference,
        })
    } else {
        None
    };

    let blend_mode = if has_blend {
        Some(read_blend_mode(r)?)
    } else {
        None
    };
    let blend_mode_logic = if has_blend_logic {
        Some(read_blend_mode(r)?)
    } else {
        None
    };

    let indirect_param = if has_indirect {
        let mut raw = [0u8; 12];
        r.read_exact(&mut raw)?;
        Some(IndirectParameter { raw })
    } else {
        None
    };

    let mut proj_tex_gen_params = Vec::with_capacity(proj_tex_gen_count);
    for _ in 0..proj_tex_gen_count {
        let mut raw = [0u8; 20];
        r.read_exact(&mut raw)?;
        proj_tex_gen_params.push(ProjectionTexGenParam { raw });
    }

    let font_shadow_param = if has_font_shadow {
        let mut raw = [0u8; 8];
        r.read_exact(&mut raw)?;
        Some(FontShadowParameter { raw })
    } else {
        None
    };

    // Capture any bytes between the end of the decoded sub-sections and
    // the material's declared end. v9 BFLYT files include an undocumented
    // extension after FontShadowParameter on some materials; preserving
    // those bytes verbatim keeps round-trip byte-identical without
    // requiring us to commit to a possibly-wrong decoding.
    let consumed = r.stream_position()? as usize;
    let trailing = if consumed < expected_size {
        let mut buf = vec![0u8; expected_size - consumed];
        r.read_exact(&mut buf)?;
        buf
    } else {
        Vec::new()
    };

    Ok(Material {
        name,
        flags_unknown,
        black_color,
        white_color,
        flags_raw,
        texture_maps,
        texture_transforms,
        tex_coord_gens,
        tev_stages,
        alpha_compare,
        blend_mode,
        blend_mode_logic,
        indirect_param,
        proj_tex_gen_params,
        font_shadow_param,
        trailing,
        flags_untrusted,
        original_section_size: Some(expected_size as u32),
    })
}

fn read_blend_mode<R: Read>(r: &mut R) -> Result<BlendMode, BflytError> {
    let blend_op = r.read_u8()?;
    let src_factor = r.read_u8()?;
    let dst_factor = r.read_u8()?;
    let logic_op = r.read_u8()?;
    Ok(BlendMode {
        blend_op,
        src_factor,
        dst_factor,
        logic_op,
    })
}

// ---------- panes ----------

fn read_pane(kind: PaneKind, payload: &[u8], section_size: u32) -> Result<BasePane, BflytError> {
    if payload.len() < 0x4C {
        return Err(BflytError::TruncatedSection("pane base".into()));
    }
    let mut c = Cursor::new(payload);
    let mut base = read_pane_base(&mut c, kind)?;

    match kind {
        PaneKind::Picture => {
            base.picture = Some(read_pic1_payload(&mut c)?);
        }
        PaneKind::Text => {
            base.text = Some(read_txt1_payload(&mut c, payload)?);
        }
        PaneKind::Window => {
            base.window = Some(read_wnd1_payload(&mut c, payload)?);
        }
        PaneKind::Parts => {
            base.parts = Some(read_prt1_payload(&mut c, payload, section_size)?);
        }
        _ => { /* pan1 and bnd1 have no kind-specific extension */ }
    }

    // Capture pane-section trailing bytes for `pan1`, `bnd1`, and `pic1`.
    //
    // - `pan1`/`bnd1` have only the pane base; the cursor naturally ends
    //   at the pane base's last field.
    // - `pic1` has vertex_colors + material_index + tex_coords; cursor
    //   stops cleanly after the last tex_coord. Some community mods
    //   append 4 extra zero bytes per pic1 section.
    //
    // `txt1` and `prt1` use their own internal trailing fields
    // (`TextBoxPane.trailing` / `PartsPane.raw_property_data`) for
    // anything past the standard payload, and `wnd1` uses
    // back-patched offsets so its cursor doesn't necessarily hit the
    // end. Capturing base trailing for those would double-count.
    if matches!(
        kind,
        PaneKind::Pane | PaneKind::Bounding | PaneKind::Picture
    ) {
        let consumed = c.stream_position()? as usize;
        if consumed < payload.len() {
            base.trailing = payload[consumed..].to_vec();
        }
    }
    Ok(base)
}

fn read_pane_base<R: Read>(r: &mut R, kind: PaneKind) -> Result<BasePane, BflytError> {
    let flag = r.read_u8()?;
    let base_position = r.read_u8()?;
    let alpha = r.read_u8()?;
    let flag_ex = r.read_u8()?;
    let mut name_bytes = [0u8; PANE_NAME_LEN];
    r.read_exact(&mut name_bytes)?;
    let name = parse_fixed_name(&name_bytes);
    let mut user_data_field = [0u8; PANE_USER_DATA_LEN];
    r.read_exact(&mut user_data_field)?;
    let translate = Vec3::read(r)?;
    let rotate = Vec3::read(r)?;
    let scale = Vec2::read(r)?;
    let width = r.read_f32::<LittleEndian>()?;
    let height = r.read_f32::<LittleEndian>()?;

    Ok(BasePane {
        kind,
        flag,
        base_position,
        alpha,
        flag_ex,
        name,
        user_data_field,
        translate,
        rotate,
        scale,
        width,
        height,
        picture: None,
        text: None,
        window: None,
        parts: None,
        user_data: None,
        children: Vec::new(),
        trailing: Vec::new(),
        opaque: None,
    })
}

fn read_pic1_payload<R: Read>(r: &mut R) -> Result<PicturePane, BflytError> {
    let mut vertex_colors = [Color8::default(); 4];
    for vc in &mut vertex_colors {
        *vc = Color8::read(r)?;
    }
    let material_index = r.read_u16::<LittleEndian>()?;
    let tex_coord_count = r.read_u8()? as usize;
    let flags = r.read_u8()?;
    let mut tex_coords = Vec::with_capacity(tex_coord_count);
    for _ in 0..tex_coord_count {
        tex_coords.push(PaneTexCoord {
            top_left: Vec2::read(r)?,
            top_right: Vec2::read(r)?,
            bottom_left: Vec2::read(r)?,
            bottom_right: Vec2::read(r)?,
        });
    }
    Ok(PicturePane {
        vertex_colors,
        material_index,
        tex_coords,
        flags,
    })
}

fn read_txt1_payload<R: Read + Seek>(r: &mut R, payload: &[u8]) -> Result<TextBoxPane, BflytError> {
    let text_buf_bytes = r.read_u16::<LittleEndian>()?;
    let text_str_bytes = r.read_u16::<LittleEndian>()?;
    let material_index = r.read_u16::<LittleEndian>()?;
    let font_index = r.read_u16::<LittleEndian>()?;
    let text_position = r.read_u8()?;
    let text_alignment = r.read_u8()?;
    let text_box_flag = r.read_u16::<LittleEndian>()?;
    let italic_ratio = r.read_f32::<LittleEndian>()?;
    let text_str_offset = r.read_u32::<LittleEndian>()?;
    let mut text_cols = [Color8::default(); 2];
    for c in &mut text_cols {
        *c = Color8::read(r)?;
    }
    let font_size = Vec2::read(r)?;
    let char_space = r.read_f32::<LittleEndian>()?;
    let line_space = r.read_f32::<LittleEndian>()?;
    let text_id_offset = r.read_u32::<LittleEndian>()?;
    let shadow_offset = Vec2::read(r)?;
    let shadow_scale = Vec2::read(r)?;
    let mut shadow_cols = [Color8::default(); 2];
    for c in &mut shadow_cols {
        *c = Color8::read(r)?;
    }
    let shadow_italic_ratio = r.read_f32::<LittleEndian>()?;
    let line_width_offset_offset = r.read_u32::<LittleEndian>()?;
    let per_character_transform_offset = r.read_u32::<LittleEndian>()?;

    // Capture remaining payload bytes for round-trip fidelity.
    let pos = r.stream_position()? as usize;
    let trailing = if pos < payload.len() {
        payload[pos..].to_vec()
    } else {
        Vec::new()
    };

    Ok(TextBoxPane {
        text_buf_bytes,
        text_str_bytes,
        material_index,
        font_index,
        text_position,
        text_alignment,
        text_box_flag,
        italic_ratio,
        text_str_offset,
        text_cols,
        font_size,
        char_space,
        line_space,
        text_id_offset,
        shadow_offset,
        shadow_scale,
        shadow_cols,
        shadow_italic_ratio,
        line_width_offset_offset,
        per_character_transform_offset,
        trailing,
    })
}

fn read_wnd1_payload<R: Read + Seek>(r: &mut R, payload: &[u8]) -> Result<WindowPane, BflytError> {
    // Window pane base record, after the common pane header.
    let stretch_l = r.read_u16::<LittleEndian>()?;
    let stretch_r = r.read_u16::<LittleEndian>()?;
    let stretch_t = r.read_u16::<LittleEndian>()?;
    let stretch_b = r.read_u16::<LittleEndian>()?;
    let frame_size_l = r.read_u16::<LittleEndian>()?;
    let frame_size_r = r.read_u16::<LittleEndian>()?;
    let frame_size_t = r.read_u16::<LittleEndian>()?;
    let frame_size_b = r.read_u16::<LittleEndian>()?;
    let frame_count = r.read_u8()?;
    let flag = r.read_u8()?;
    let _pad = r.read_u16::<LittleEndian>()?;
    let content_offset = r.read_u32::<LittleEndian>()? as usize;
    let frame_offset = r.read_u32::<LittleEndian>()? as usize;

    // Content lives at content_offset bytes from the start of the pane
    // section (i.e. start of payload + 8 because content offsets are file-
    // relative, but the pane section header is 8 bytes too — so within
    // `payload` it's `content_offset - 8`).
    let content_pos = content_offset
        .checked_sub(8)
        .ok_or_else(|| BflytError::Format("wnd1 content_offset < 8".into()))?;
    let mut cc = Cursor::new(&payload[content_pos..]);
    let content = read_window_content(&mut cc)?;

    let mut frames = Vec::with_capacity(frame_count as usize);
    if frame_count > 0 {
        let frame_table_pos = frame_offset
            .checked_sub(8)
            .ok_or_else(|| BflytError::Format("wnd1 frame_offset < 8".into()))?;
        let mut fc = Cursor::new(&payload[frame_table_pos..]);
        let mut frame_offsets = Vec::with_capacity(frame_count as usize);
        for _ in 0..frame_count {
            frame_offsets.push(fc.read_u32::<LittleEndian>()? as usize);
        }
        for f_off in frame_offsets {
            let f_pos = f_off
                .checked_sub(8)
                .ok_or_else(|| BflytError::Format("wnd1 frame entry offset < 8".into()))?;
            let mut frc = Cursor::new(&payload[f_pos..]);
            frames.push(read_window_frame(&mut frc)?);
        }
    }

    Ok(WindowPane {
        stretch_l,
        stretch_r,
        stretch_t,
        stretch_b,
        frame_size_l,
        frame_size_r,
        frame_size_t,
        frame_size_b,
        frame_count,
        flag,
        content,
        frames,
    })
}

fn read_window_content<R: Read>(r: &mut R) -> Result<WindowContent, BflytError> {
    let mut vertex_colors = [Color8::default(); 4];
    for vc in &mut vertex_colors {
        *vc = Color8::read(r)?;
    }
    let material_index = r.read_u16::<LittleEndian>()?;
    let tex_coord_count = r.read_u8()? as usize;
    let _pad = r.read_u8()?;
    let mut tex_coords = Vec::with_capacity(tex_coord_count);
    for _ in 0..tex_coord_count {
        tex_coords.push(PaneTexCoord {
            top_left: Vec2::read(r)?,
            top_right: Vec2::read(r)?,
            bottom_left: Vec2::read(r)?,
            bottom_right: Vec2::read(r)?,
        });
    }
    Ok(WindowContent {
        vertex_colors,
        material_index,
        tex_coords,
    })
}

fn read_window_frame<R: Read>(r: &mut R) -> Result<WindowFrame, BflytError> {
    let material_index = r.read_u16::<LittleEndian>()?;
    let texture_flip = r.read_u8()?;
    let _padding = r.read_u8()?;
    Ok(WindowFrame {
        material_index,
        texture_flip,
        _padding,
    })
}

fn read_prt1_payload<R: Read + Seek>(
    r: &mut R,
    payload: &[u8],
    section_size: u32,
) -> Result<PartsPane, BflytError> {
    let property_count = r.read_u32::<LittleEndian>()?;
    let magnify = Vec2::read(r)?;
    let mut properties = Vec::with_capacity(property_count as usize);
    for _ in 0..property_count {
        let mut name_bytes = [0u8; PANE_NAME_LEN];
        r.read_exact(&mut name_bytes)?;
        let name = parse_fixed_name(&name_bytes);
        let usage_flag = r.read_u8()?;
        let basic_usage_flag = r.read_u8()?;
        let material_usage_flag = r.read_u8()?;
        let system_ext_user_data_override_flag = r.read_u8()?;
        let property_offset = r.read_u32::<LittleEndian>()?;
        let ext_user_data_offset = r.read_u32::<LittleEndian>()?;
        let pane_basic_info_offset = r.read_u32::<LittleEndian>()?;
        properties.push(PartsProperty {
            name,
            usage_flag,
            basic_usage_flag,
            material_usage_flag,
            system_ext_user_data_override_flag,
            property_offset,
            ext_user_data_offset,
            pane_basic_info_offset,
        });
    }
    // Part name (null-terminated, 4-byte aligned).
    let part_name = read_cstring_until_null(r)?;
    let pos = r.stream_position()? as usize;
    let pad = (4 - (pos % 4)) % 4;
    if pad > 0 {
        r.seek(SeekFrom::Current(pad as i64))?;
    }
    let after_pos = r.stream_position()? as usize;

    // Capture the rest of the section (embedded property/user data sub-sections)
    // verbatim. SGPO doesn't edit prt1 panes; round-trip preservation is
    // sufficient.
    let raw_property_data = if after_pos < payload.len() {
        payload[after_pos..].to_vec()
    } else {
        Vec::new()
    };

    Ok(PartsPane {
        property_count,
        magnify,
        properties,
        part_name,
        raw_property_data,
        declared_size: section_size,
    })
}

// ---------- groups ----------

/// Group section name slot size for Switch-era BFLYT (v5+).
const GRP1_NAME_LEN: usize = 34;

fn read_group(payload: &[u8]) -> Result<Group, BflytError> {
    // v5+ (Switch) layout: 34-byte name, then u16 numNodes (no padding).
    // v<5 layout was 24-byte name + u16 numNodes + u16 padding. We're
    // Switch-only, so v5+ is the only case.
    if payload.len() < GRP1_NAME_LEN + 2 {
        return Err(BflytError::TruncatedSection("grp1".into()));
    }
    let mut c = Cursor::new(payload);
    let mut name_bytes = [0u8; GRP1_NAME_LEN];
    c.read_exact(&mut name_bytes)?;
    let name = parse_fixed_name(&name_bytes);
    let pane_count = c.read_u16::<LittleEndian>()? as usize;
    let mut panes = Vec::with_capacity(pane_count);
    for _ in 0..pane_count {
        let mut nb = [0u8; PANE_NAME_LEN];
        c.read_exact(&mut nb)?;
        panes.push(parse_fixed_name(&nb));
    }
    Ok(Group {
        name,
        panes,
        children: Vec::new(),
    })
}

// ---------- helpers ----------

fn parse_fixed_name(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn read_null_terminated(buf: &[u8]) -> Result<String, BflytError> {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    Ok(String::from_utf8_lossy(&buf[..end]).into_owned())
}

fn read_cstring_until_null<R: Read>(r: &mut R) -> Result<String, BflytError> {
    let mut bytes = Vec::with_capacity(32);
    let mut buf = [0u8; 1];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        if buf[0] == 0 {
            break;
        }
        bytes.push(buf[0]);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}
