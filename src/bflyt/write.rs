//! BFLYT v8 writer. Reverses the parser, recomputing section sizes and the
//! file header. A parse + write round-trip on an unmodified BFLYT yields a
//! byte-identical output (modulo any padding the original game tool may
//! have left undefined — we always zero-fill).

use byteorder::{LittleEndian, WriteBytesExt};
use std::io::Write;

use super::sections::*;

const MAGIC_FLYT: [u8; 4] = *b"FLYT";

pub fn write_bflyt(b: &BFLYT) -> Result<Vec<u8>, BflytError> {
    let mut out = Vec::with_capacity(8 * 1024);

    // ---- File header (20 bytes; back-patched at the end). ----
    out.extend_from_slice(&MAGIC_FLYT);
    out.write_u16::<LittleEndian>(0xFEFF)?; // BOM
    out.write_u16::<LittleEndian>(0x14)?;   // header size
    out.write_u32::<LittleEndian>(b.version)?;
    out.write_u32::<LittleEndian>(0)?;      // file size — back-patched
    out.write_u16::<LittleEndian>(0)?;      // section count — back-patched
    out.write_u16::<LittleEndian>(0)?;      // padding

    let mut section_count: u16 = 0;

    // lyt1 (always first, always present)
    let lyt = build_lyt1(&b.layout)?;
    write_section(&mut out, b"lyt1", &lyt)?;
    section_count += 1;

    // txl1 / fnl1 (only if non-empty; matches Switch Toolbox behavior)
    if !b.textures.is_empty() {
        let txl = build_string_list(&b.textures)?;
        write_section(&mut out, b"txl1", &txl)?;
        section_count += 1;
    }
    if !b.fonts.is_empty() {
        let fnl = build_string_list(&b.fonts)?;
        write_section(&mut out, b"fnl1", &fnl)?;
        section_count += 1;
    }

    // mat1
    if !b.materials.is_empty() {
        let mat = build_mat1(&b.materials)?;
        write_section(&mut out, b"mat1", &mat)?;
        section_count += 1;
    }

    // Pane tree as flat sections with pas1/pae1 markers.
    if let Some(root) = &b.root_pane {
        write_pane_tree(&mut out, root, &mut section_count)?;
    }

    // Group tree.
    if let Some(root_grp) = &b.root_group {
        write_group_tree(&mut out, root_grp, &mut section_count)?;
    }

    // usd1 (file-level)
    if let Some(ud) = &b.user_data {
        write_section(&mut out, b"usd1", &ud.raw)?;
        section_count += 1;
    }

    // Back-patch file_size and section_count.
    let total_size = out.len() as u32;
    out[0x0C..0x10].copy_from_slice(&total_size.to_le_bytes());
    out[0x10..0x12].copy_from_slice(&section_count.to_le_bytes());

    Ok(out)
}

/// Write a section with `[magic:4][size:4][payload][optional padding to 4]`,
/// where `size` includes the 8-byte header and any trailing padding.
fn write_section(out: &mut Vec<u8>, magic: &[u8; 4], payload: &[u8]) -> Result<(), BflytError> {
    let start = out.len();
    out.extend_from_slice(magic);
    out.write_u32::<LittleEndian>(0)?; // size placeholder
    out.extend_from_slice(payload);
    while (out.len() - start) % 4 != 0 {
        out.push(0);
    }
    let size = (out.len() - start) as u32;
    out[start + 4..start + 8].copy_from_slice(&size.to_le_bytes());
    Ok(())
}

// ---- lyt1 ----

fn build_lyt1(lyt: &LayoutInfo) -> Result<Vec<u8>, BflytError> {
    let mut p = Vec::with_capacity(0x20 + lyt.name.len());
    p.write_u8(if lyt.draw_centered { 1 } else { 0 })?;
    p.extend_from_slice(&[0u8; 3]); // padding
    p.write_f32::<LittleEndian>(lyt.width)?;
    p.write_f32::<LittleEndian>(lyt.height)?;
    p.write_f32::<LittleEndian>(lyt.max_parts_width)?;
    p.write_f32::<LittleEndian>(lyt.max_parts_height)?;
    p.extend_from_slice(lyt.name.as_bytes());
    p.push(0); // null terminator
    Ok(p)
}

// ---- txl1 / fnl1 ----

fn build_string_list(strings: &[String]) -> Result<Vec<u8>, BflytError> {
    let mut p = Vec::new();
    p.write_u16::<LittleEndian>(strings.len() as u16)?;
    p.write_u16::<LittleEndian>(0)?; // padding

    let table_base = p.len();
    p.extend(std::iter::repeat(0u8).take(strings.len() * 4));

    let mut offsets = Vec::with_capacity(strings.len());
    for s in strings {
        offsets.push((p.len() - table_base) as u32);
        p.extend_from_slice(s.as_bytes());
        p.push(0);
    }
    for (i, off) in offsets.iter().enumerate() {
        let pos = table_base + i * 4;
        p[pos..pos + 4].copy_from_slice(&off.to_le_bytes());
    }
    Ok(p)
}

// ---- mat1 ----

fn build_mat1(materials: &[Material]) -> Result<Vec<u8>, BflytError> {
    let mut p = Vec::new();
    p.write_u16::<LittleEndian>(materials.len() as u16)?;
    p.write_u16::<LittleEndian>(0)?; // padding

    let table_pos = p.len();
    p.extend(std::iter::repeat(0u8).take(materials.len() * 4));

    let mut offsets = Vec::with_capacity(materials.len());
    for mat in materials {
        let mut m = mat.clone();
        m.rebuild_flags();
        // Material offsets are file-absolute (relative to the section's
        // `magic` byte). Within `p` (section payload), absolute = 8 + p.len()
        // because the section header is 8 bytes.
        offsets.push((8 + p.len()) as u32);
        write_material(&mut p, &m)?;
    }
    for (i, off) in offsets.iter().enumerate() {
        let pos = table_pos + i * 4;
        p[pos..pos + 4].copy_from_slice(&off.to_le_bytes());
    }
    Ok(p)
}

fn write_material<W: Write>(w: &mut W, m: &Material) -> std::io::Result<()> {
    let mut name_buf = [0u8; MAT_NAME_LEN];
    let bytes = m.name.as_bytes();
    let n = bytes.len().min(MAT_NAME_LEN - 1);
    name_buf[..n].copy_from_slice(&bytes[..n]);
    w.write_all(&name_buf)?;

    w.write_u32::<LittleEndian>(m.flags_raw)?;
    w.write_u32::<LittleEndian>(m.flags_unknown)?;
    w.write_all(&[m.black_color.r, m.black_color.g, m.black_color.b, m.black_color.a])?;
    w.write_all(&[m.white_color.r, m.white_color.g, m.white_color.b, m.white_color.a])?;

    for tr in &m.texture_maps {
        w.write_i16::<LittleEndian>(tr.index)?;
        w.write_u8(tr.wrap_mode_u)?;
        w.write_u8(tr.wrap_mode_v)?;
    }
    for tt in &m.texture_transforms {
        w.write_f32::<LittleEndian>(tt.translate.x)?;
        w.write_f32::<LittleEndian>(tt.translate.y)?;
        w.write_f32::<LittleEndian>(tt.rotate)?;
        w.write_f32::<LittleEndian>(tt.scale.x)?;
        w.write_f32::<LittleEndian>(tt.scale.y)?;
    }
    for cg in &m.tex_coord_gens {
        w.write_all(&cg.raw)?;
    }
    for ts in &m.tev_stages {
        w.write_u8(ts.color_blend)?;
        w.write_u8(ts.alpha_blend)?;
        w.write_all(&ts.unk)?;
    }
    if let Some(ac) = &m.alpha_compare {
        w.write_u8(ac.function)?;
        w.write_all(&ac._padding)?;
        w.write_f32::<LittleEndian>(ac.reference)?;
    }
    if let Some(b) = &m.blend_mode {
        w.write_all(&[b.blend_op, b.src_factor, b.dst_factor, b.logic_op])?;
    }
    if let Some(b) = &m.blend_mode_logic {
        w.write_all(&[b.blend_op, b.src_factor, b.dst_factor, b.logic_op])?;
    }
    if let Some(ip) = &m.indirect_param {
        w.write_all(&ip.raw)?;
    }
    for pp in &m.proj_tex_gen_params {
        w.write_all(&pp.raw)?;
    }
    if let Some(fs) = &m.font_shadow_param {
        w.write_all(&fs.raw)?;
    }
    Ok(())
}

// ---- Pane tree ----

fn write_pane_tree(
    out: &mut Vec<u8>,
    pane: &BasePane,
    section_count: &mut u16,
) -> Result<(), BflytError> {
    write_pane_section(out, pane)?;
    *section_count += 1;

    if !pane.children.is_empty() {
        write_section(out, b"pas1", &[])?;
        *section_count += 1;
        for child in &pane.children {
            write_pane_tree(out, child, section_count)?;
        }
        write_section(out, b"pae1", &[])?;
        *section_count += 1;
    }
    Ok(())
}

fn write_pane_section(out: &mut Vec<u8>, p: &BasePane) -> Result<(), BflytError> {
    let magic: &[u8; 4] = match p.kind {
        PaneKind::Pane => b"pan1",
        PaneKind::Picture => b"pic1",
        PaneKind::Text => b"txt1",
        PaneKind::Window => b"wnd1",
        PaneKind::Parts => b"prt1",
        PaneKind::Bounding => b"bnd1",
    };

    let start = out.len();
    out.extend_from_slice(magic);
    out.write_u32::<LittleEndian>(0)?; // size placeholder

    write_pane_base(out, p)?;

    match p.kind {
        PaneKind::Picture => {
            if let Some(pic) = &p.picture {
                write_pic_payload(out, pic)?;
            }
        }
        PaneKind::Text => {
            if let Some(t) = &p.text {
                write_txt_payload(out, t)?;
            }
        }
        PaneKind::Window => {
            if let Some(w) = &p.window {
                write_wnd_payload(out, w)?;
            }
        }
        PaneKind::Parts => {
            if let Some(pp) = &p.parts {
                write_prt_payload(out, pp)?;
            }
        }
        _ => {}
    }
    while (out.len() - start) % 4 != 0 {
        out.push(0);
    }
    let size = (out.len() - start) as u32;
    out[start + 4..start + 8].copy_from_slice(&size.to_le_bytes());
    Ok(())
}

fn write_pane_base<W: Write>(w: &mut W, p: &BasePane) -> std::io::Result<()> {
    w.write_u8(p.flag)?;
    w.write_u8(p.base_position)?;
    w.write_u8(p.alpha)?;
    w.write_u8(p.flag_ex)?;

    let mut name_buf = [0u8; PANE_NAME_LEN];
    let bytes = p.name.as_bytes();
    let n = bytes.len().min(PANE_NAME_LEN - 1);
    name_buf[..n].copy_from_slice(&bytes[..n]);
    w.write_all(&name_buf)?;

    w.write_all(&p.user_data_field)?;

    w.write_f32::<LittleEndian>(p.translate.x)?;
    w.write_f32::<LittleEndian>(p.translate.y)?;
    w.write_f32::<LittleEndian>(p.translate.z)?;
    w.write_f32::<LittleEndian>(p.rotate.x)?;
    w.write_f32::<LittleEndian>(p.rotate.y)?;
    w.write_f32::<LittleEndian>(p.rotate.z)?;
    w.write_f32::<LittleEndian>(p.scale.x)?;
    w.write_f32::<LittleEndian>(p.scale.y)?;
    w.write_f32::<LittleEndian>(p.width)?;
    w.write_f32::<LittleEndian>(p.height)?;
    Ok(())
}

fn write_pic_payload(out: &mut Vec<u8>, pic: &PicturePane) -> Result<(), BflytError> {
    for c in &pic.vertex_colors {
        out.write_all(&[c.r, c.g, c.b, c.a])?;
    }
    out.write_u16::<LittleEndian>(pic.material_index)?;
    out.write_u8(pic.tex_coords.len() as u8)?;
    out.write_u8(pic.flags)?;
    for tc in &pic.tex_coords {
        out.write_f32::<LittleEndian>(tc.top_left.x)?;
        out.write_f32::<LittleEndian>(tc.top_left.y)?;
        out.write_f32::<LittleEndian>(tc.top_right.x)?;
        out.write_f32::<LittleEndian>(tc.top_right.y)?;
        out.write_f32::<LittleEndian>(tc.bottom_left.x)?;
        out.write_f32::<LittleEndian>(tc.bottom_left.y)?;
        out.write_f32::<LittleEndian>(tc.bottom_right.x)?;
        out.write_f32::<LittleEndian>(tc.bottom_right.y)?;
    }
    Ok(())
}

fn write_txt_payload(out: &mut Vec<u8>, t: &TextBoxPane) -> Result<(), BflytError> {
    out.write_u16::<LittleEndian>(t.text_buf_bytes)?;
    out.write_u16::<LittleEndian>(t.text_str_bytes)?;
    out.write_u16::<LittleEndian>(t.material_index)?;
    out.write_u16::<LittleEndian>(t.font_index)?;
    out.write_u8(t.text_position)?;
    out.write_u8(t.text_alignment)?;
    out.write_u16::<LittleEndian>(t.text_box_flag)?;
    out.write_f32::<LittleEndian>(t.italic_ratio)?;
    out.write_u32::<LittleEndian>(t.text_str_offset)?;
    for c in &t.text_cols {
        out.write_all(&[c.r, c.g, c.b, c.a])?;
    }
    out.write_f32::<LittleEndian>(t.font_size.x)?;
    out.write_f32::<LittleEndian>(t.font_size.y)?;
    out.write_f32::<LittleEndian>(t.char_space)?;
    out.write_f32::<LittleEndian>(t.line_space)?;
    out.write_u32::<LittleEndian>(t.text_id_offset)?;
    out.write_f32::<LittleEndian>(t.shadow_offset.x)?;
    out.write_f32::<LittleEndian>(t.shadow_offset.y)?;
    out.write_f32::<LittleEndian>(t.shadow_scale.x)?;
    out.write_f32::<LittleEndian>(t.shadow_scale.y)?;
    for c in &t.shadow_cols {
        out.write_all(&[c.r, c.g, c.b, c.a])?;
    }
    out.write_f32::<LittleEndian>(t.shadow_italic_ratio)?;
    out.write_u32::<LittleEndian>(t.line_width_offset_offset)?;
    out.write_u32::<LittleEndian>(t.per_character_transform_offset)?;
    out.write_all(&t.trailing)?;
    Ok(())
}

fn write_wnd_payload(out: &mut Vec<u8>, wnd: &WindowPane) -> Result<(), BflytError> {
    // Capture the section start so we can convert payload offsets to
    // file-absolute offsets relative to the `magic` byte. The current
    // section header occupies the 8 bytes ending at `out.len() - <pane base size>`.
    // For simplicity we recover the section start by scanning back from
    // the call site context — but we don't have access to that here.
    // Instead, we know that:
    //   abs_offset(of x) = (out.len() at moment of x) - section_start
    // So we record `section_start` from the caller's stack. Cleanest fix:
    // require the caller to pass it in.
    //
    // The caller (`write_pane_section`) records `start` as the section
    // start byte. We can recover it here as `start = out.len() - <fixed
    // pane base size> - 4 - 4 - <prefix already written for wnd>`. That's
    // brittle; use the simpler invariant: at the time write_wnd_payload
    // runs, the only thing not yet written for THIS section is the wnd1
    // payload. We capture the current `out.len()` here as the pre-payload
    // mark; the section's own `magic` byte is at `pre - (header 8 + base 0x4C)`.
    let pre = out.len();
    let section_start = pre - (8 + 0x4C);

    out.write_u16::<LittleEndian>(wnd.stretch_l)?;
    out.write_u16::<LittleEndian>(wnd.stretch_r)?;
    out.write_u16::<LittleEndian>(wnd.stretch_t)?;
    out.write_u16::<LittleEndian>(wnd.stretch_b)?;
    out.write_u16::<LittleEndian>(wnd.frame_size_l)?;
    out.write_u16::<LittleEndian>(wnd.frame_size_r)?;
    out.write_u16::<LittleEndian>(wnd.frame_size_t)?;
    out.write_u16::<LittleEndian>(wnd.frame_size_b)?;
    out.write_u8(wnd.frame_count)?;
    out.write_u8(wnd.flag)?;
    out.write_u16::<LittleEndian>(0)?;       // padding
    let content_off_pos = out.len();
    out.write_u32::<LittleEndian>(0)?;       // back-patch
    let frame_off_pos = out.len();
    out.write_u32::<LittleEndian>(0)?;       // back-patch

    // Content immediately after.
    let content_pos = out.len();
    let content_off = (content_pos - section_start) as u32;
    out[content_off_pos..content_off_pos + 4].copy_from_slice(&content_off.to_le_bytes());

    for c in &wnd.content.vertex_colors {
        out.write_all(&[c.r, c.g, c.b, c.a])?;
    }
    out.write_u16::<LittleEndian>(wnd.content.material_index)?;
    out.write_u8(wnd.content.tex_coords.len() as u8)?;
    out.write_u8(0)?; // padding
    for tc in &wnd.content.tex_coords {
        out.write_f32::<LittleEndian>(tc.top_left.x)?;
        out.write_f32::<LittleEndian>(tc.top_left.y)?;
        out.write_f32::<LittleEndian>(tc.top_right.x)?;
        out.write_f32::<LittleEndian>(tc.top_right.y)?;
        out.write_f32::<LittleEndian>(tc.bottom_left.x)?;
        out.write_f32::<LittleEndian>(tc.bottom_left.y)?;
        out.write_f32::<LittleEndian>(tc.bottom_right.x)?;
        out.write_f32::<LittleEndian>(tc.bottom_right.y)?;
    }

    if wnd.frames.is_empty() {
        // frame_offset stays 0.
        return Ok(());
    }

    // Frame offset table.
    let frame_table_pos = out.len();
    let frame_off = (frame_table_pos - section_start) as u32;
    out[frame_off_pos..frame_off_pos + 4].copy_from_slice(&frame_off.to_le_bytes());

    let table_bytes = wnd.frames.len() * 4;
    out.extend(std::iter::repeat(0u8).take(table_bytes));

    let mut frame_offsets = Vec::with_capacity(wnd.frames.len());
    for f in &wnd.frames {
        frame_offsets.push((out.len() - section_start) as u32);
        out.write_u16::<LittleEndian>(f.material_index)?;
        out.write_u8(f.texture_flip)?;
        out.write_u8(f._padding)?;
    }
    for (i, off) in frame_offsets.iter().enumerate() {
        let pos = frame_table_pos + i * 4;
        out[pos..pos + 4].copy_from_slice(&off.to_le_bytes());
    }
    Ok(())
}

fn write_prt_payload(out: &mut Vec<u8>, p: &PartsPane) -> Result<(), BflytError> {
    out.write_u32::<LittleEndian>(p.property_count)?;
    out.write_f32::<LittleEndian>(p.magnify.x)?;
    out.write_f32::<LittleEndian>(p.magnify.y)?;
    for prop in &p.properties {
        let mut name_buf = [0u8; PANE_NAME_LEN];
        let bytes = prop.name.as_bytes();
        let n = bytes.len().min(PANE_NAME_LEN - 1);
        name_buf[..n].copy_from_slice(&bytes[..n]);
        out.write_all(&name_buf)?;
        out.write_u8(prop.usage_flag)?;
        out.write_u8(prop.basic_usage_flag)?;
        out.write_u8(prop.material_usage_flag)?;
        out.write_u8(prop.system_ext_user_data_override_flag)?;
        out.write_u32::<LittleEndian>(prop.property_offset)?;
        out.write_u32::<LittleEndian>(prop.ext_user_data_offset)?;
        out.write_u32::<LittleEndian>(prop.pane_basic_info_offset)?;
    }
    out.write_all(p.part_name.as_bytes())?;
    out.write_u8(0)?; // null terminator
    while out.len() % 4 != 0 {
        out.push(0);
    }
    out.write_all(&p.raw_property_data)?;
    Ok(())
}

// ---- Groups ----

fn write_group_tree(
    out: &mut Vec<u8>,
    g: &Group,
    section_count: &mut u16,
) -> Result<(), BflytError> {
    let mut payload = Vec::new();
    let mut name_buf = [0u8; PANE_NAME_LEN];
    let bytes = g.name.as_bytes();
    let n = bytes.len().min(PANE_NAME_LEN - 1);
    name_buf[..n].copy_from_slice(&bytes[..n]);
    payload.extend_from_slice(&name_buf);
    payload.write_u16::<LittleEndian>(g.panes.len() as u16)?;
    payload.write_u16::<LittleEndian>(0)?; // padding
    for pane_name in &g.panes {
        let mut nb = [0u8; PANE_NAME_LEN];
        let pb = pane_name.as_bytes();
        let pn = pb.len().min(PANE_NAME_LEN - 1);
        nb[..pn].copy_from_slice(&pb[..pn]);
        payload.extend_from_slice(&nb);
    }
    write_section(out, b"grp1", &payload)?;
    *section_count += 1;

    if !g.children.is_empty() {
        write_section(out, b"grs1", &[])?;
        *section_count += 1;
        for child in &g.children {
            write_group_tree(out, child, section_count)?;
        }
        write_section(out, b"gre1", &[])?;
        *section_count += 1;
    }
    Ok(())
}
