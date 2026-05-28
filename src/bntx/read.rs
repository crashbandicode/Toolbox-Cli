//! BNTX parser. Implements just enough of the format to round-trip the
//! files we care about (Switch BNTX with BC7/BC1/BC3 textures from
//! Smash Ultimate-style assets).

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Cursor, Read};

use super::*;
use super::error::Error;

const MAGIC_BNTX: [u8; 4] = *b"BNTX";
const MAGIC_NX: [u8; 4] = *b"NX  ";
const MAGIC_BRTI: [u8; 4] = *b"BRTI";

pub fn read_bntx(data: &[u8]) -> Result<BNTX, Error> {
    let mut c = Cursor::new(data);

    let mut magic = [0u8; 4];
    c.read_exact(&mut magic)?;
    if magic != MAGIC_BNTX {
        return Err(Error::BadMagic(magic));
    }
    let _padding = c.read_u32::<LittleEndian>()?;
    let _version = c.read_u32::<LittleEndian>()?;
    let bom = c.read_u16::<LittleEndian>()?;
    if bom != 0xFEFF {
        return Err(Error::Format(format!("unexpected BOM 0x{bom:04x}")));
    }
    let _alignment_shift = c.read_u8()?;
    let _target_address_size = c.read_u8()?;
    let file_name_offset = c.read_u32::<LittleEndian>()?;
    let _flag = c.read_u16::<LittleEndian>()?;
    let _first_block_offset = c.read_u16::<LittleEndian>()?;
    let _relocation_table_offset = c.read_u32::<LittleEndian>()?;
    let _file_size = c.read_u32::<LittleEndian>()?;

    // NX header.
    let mut nx_magic = [0u8; 4];
    c.read_exact(&mut nx_magic)?;
    if nx_magic != MAGIC_NX {
        return Err(Error::Format(format!(
            "expected 'NX  ' header, got {:?}",
            std::str::from_utf8(&nx_magic).unwrap_or("?")
        )));
    }
    let count = c.read_u32::<LittleEndian>()?;
    let info_ptrs_offset = c.read_u64::<LittleEndian>()?;
    let _data_ptr_offset = c.read_u64::<LittleEndian>()?;
    let _dict_ptr_offset = c.read_u64::<LittleEndian>()?;
    let _str_pool_offset = c.read_u32::<LittleEndian>()?;
    let _str_pool_size = c.read_u32::<LittleEndian>()?;

    // The container name lives at file_name_offset (a length-prefixed string
    // in BNTX's "Bin Str" format: u16 length, then bytes, then null term.)
    let name = read_bntx_str(data, file_name_offset as usize)?;

    // Read the array of texture-info pointers.
    let mut texture_offsets = Vec::with_capacity(count as usize);
    let mut p = info_ptrs_offset as usize;
    for _ in 0..count {
        if p + 8 > data.len() {
            return Err(Error::Truncated("BNTX info pointer table".into()));
        }
        let off = u64::from_le_bytes([
            data[p], data[p + 1], data[p + 2], data[p + 3],
            data[p + 4], data[p + 5], data[p + 6], data[p + 7],
        ]);
        texture_offsets.push(off);
        p += 8;
    }

    let mut textures = Vec::with_capacity(count as usize);
    for off in texture_offsets {
        textures.push(read_brti(data, off as usize)?);
    }

    Ok(BNTX { name, textures })
}

fn read_brti(data: &[u8], offset: usize) -> Result<Texture, Error> {
    // BRTI layout (matches Syroot.NintenTools.NSW.Bntx and jam1garner/bntx):
    //   0x00  magic "BRTI" (4)
    //   0x04  size (u32)
    //   0x08  size2 (u64)
    //   0x10  flags (u8)
    //   0x11  dim (u8)
    //   0x12  tile_mode (u16)
    //   0x14  swizzle (u16)
    //   0x16  mip_count (u16)
    //   0x18  num_multi_sample (u32)
    //   0x1C  format (u32, surface_format)
    //   0x20  unk2 (u32)
    //   0x24  width (u32)
    //   0x28  height (u32)
    //   0x2C  depth (u32)
    //   0x30  array_len (u32)
    //   0x34  size_range / block_height_log2 (i32)
    //   0x38  unk4 (6 * u32) = 24 bytes
    //   0x50  image_size (u32)
    //   0x54  align (u32)
    //   0x58  comp_sel (u32) — channel swizzle, 4 bytes packed
    //   0x5C  ty (u32) — image dim type
    //   0x60  name_addr (u64) — file-absolute offset to BntxStr
    //   0x68  parent_addr (u64)
    //   0x70  texture_offset (u64) — file-absolute offset to image bytes
    if offset + 0x80 > data.len() {
        return Err(Error::Truncated("BRTI header".into()));
    }
    let mut c = Cursor::new(&data[offset..]);

    let mut magic = [0u8; 4];
    c.read_exact(&mut magic)?;
    if magic != MAGIC_BRTI {
        return Err(Error::Format(format!(
            "expected BRTI, got {:?}",
            std::str::from_utf8(&magic).unwrap_or("?")
        )));
    }
    let _size = c.read_u32::<LittleEndian>()?;
    let _size2 = c.read_u64::<LittleEndian>()?;
    let _flags = c.read_u8()?;
    let surface_dim_b = c.read_u8()?;
    let tile_mode = c.read_u16::<LittleEndian>()?;
    let _swizzle = c.read_u16::<LittleEndian>()?;
    let mip_count = c.read_u16::<LittleEndian>()? as u32;
    let _multi_sample_count = c.read_u32::<LittleEndian>()?;
    let surface_format = c.read_u32::<LittleEndian>()?;
    let _unk2 = c.read_u32::<LittleEndian>()?;
    let width = c.read_u32::<LittleEndian>()?;
    let height = c.read_u32::<LittleEndian>()?;
    let depth = c.read_u32::<LittleEndian>()?;
    let array_count = c.read_u32::<LittleEndian>()?;
    let block_height_log2 = c.read_u32::<LittleEndian>()?;
    let mut _unk4 = [0u32; 6];
    for slot in &mut _unk4 {
        *slot = c.read_u32::<LittleEndian>()?;
    }
    let image_size = c.read_u32::<LittleEndian>()?;
    let _alignment = c.read_u32::<LittleEndian>()?;
    let channel_swizzle = c.read_u32::<LittleEndian>()?;
    let _ty = c.read_u32::<LittleEndian>()?;
    let name_offset = c.read_u64::<LittleEndian>()?;
    let _parent_offset = c.read_u64::<LittleEndian>()?;
    let texture_offset = c.read_u64::<LittleEndian>()?;
    // Switch Toolbox tracks `texture_layout` / `texture_layout2` for write
    // back; we don't surface them in the in-memory model yet.
    let texture_layout = 0u32;
    let texture_layout2 = 0u32;

    let format = TextureFormat::from_surface_format(surface_format)
        .ok_or(Error::UnsupportedFormat(surface_format))?;
    let surface_dim = match surface_dim_b {
        2 => SurfaceDim::Dim2D,
        8 => SurfaceDim::DimCube,
        _ => SurfaceDim::Dim2D,
    };

    let channels = [
        decode_channel((channel_swizzle >> 0) as u8 & 0xff),
        decode_channel((channel_swizzle >> 8) as u8 & 0xff),
        decode_channel((channel_swizzle >> 16) as u8 & 0xff),
        decode_channel((channel_swizzle >> 24) as u8 & 0xff),
    ];

    let name = read_bntx_str(data, name_offset as usize)?;

    // For inspection we don't actually need the swizzled image bytes —
    // bntx-inspect just reports metadata. Capture them as an empty Vec
    // here; the texture pipeline will populate this when we implement
    // import.
    let texture_data = if texture_offset != 0
        && (texture_offset as usize) < data.len()
        && (texture_offset as usize + image_size as usize) <= data.len()
    {
        data[texture_offset as usize..texture_offset as usize + image_size as usize].to_vec()
    } else {
        Vec::new()
    };

    Ok(Texture {
        name,
        width,
        height,
        depth,
        mip_count,
        array_count,
        format,
        channels,
        surface_dim,
        tile_mode: tile_mode as u8,
        texture_layout,
        texture_layout2,
        block_height_log2,
        data: texture_data,
        mip_offsets: Vec::new(),
    })
}

fn decode_channel(b: u8) -> Channel {
    match b {
        0 => Channel::Zero,
        1 => Channel::One,
        2 => Channel::Red,
        3 => Channel::Green,
        4 => Channel::Blue,
        5 => Channel::Alpha,
        _ => Channel::Zero,
    }
}

fn read_bntx_str(data: &[u8], offset: usize) -> Result<String, Error> {
    if offset + 2 > data.len() {
        return Err(Error::Truncated(format!(
            "BNTX string at offset 0x{offset:x}"
        )));
    }
    let len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
    let start = offset + 2;
    if start + len > data.len() {
        return Err(Error::Truncated(format!(
            "BNTX string body at offset 0x{offset:x}"
        )));
    }
    Ok(String::from_utf8_lossy(&data[start..start + len]).into_owned())
}
