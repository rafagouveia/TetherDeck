//! Minimal HEVC SPS geometry reader (ITU-T H.265 sequence_parameter_set_rbsp).
//! It does not decode pictures or infer padding from screenshot dimensions.

#[derive(Clone, Copy, Debug)]
pub struct HevcGeometry {
    pub coded_width: u32,
    pub coded_height: u32,
    pub display_width: u32,
    pub display_height: u32,
    pub crop_left: u32,
    pub crop_top: u32,
    pub crop_right: u32,
    pub crop_bottom: u32,
}

/// Read type-33 SPS from an Annex-B access unit. Limits are deliberately tighter
/// than the picture parser; malformed metadata cannot request large allocations.
pub fn geometry(annex_b: &[u8]) -> Option<HevcGeometry> {
    let mut offset = 0;
    while let Some((start, header)) = next_start(annex_b, offset) {
        let end = next_start(annex_b, header).map_or(annex_b.len(), |(start, _)| start);
        if end >= header + 2 && (annex_b[header] >> 1) & 0x3f == 33 {
            let nal = annex_b.get(header + 2..end)?;
            if nal.len() > 65_536 {
                return None;
            }
            return parse_sps(nal);
        }
        offset = end.max(start + 3);
    }
    None
}

fn next_start(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut i = from;
    while i + 3 <= bytes.len() {
        if bytes[i..].starts_with(&[0, 0, 0, 1]) {
            return Some((i, i + 4));
        }
        if bytes[i..].starts_with(&[0, 0, 1]) {
            return Some((i, i + 3));
        }
        i += 1;
    }
    None
}

fn parse_sps(escaped: &[u8]) -> Option<HevcGeometry> {
    let mut rbsp = Vec::with_capacity(escaped.len());
    let mut zeros = 0;
    for &byte in escaped {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        rbsp.push(byte);
        zeros = if byte == 0 { zeros + 1 } else { 0 };
    }
    let mut bits = Bits {
        bytes: &rbsp,
        offset: 0,
    };
    bits.skip(4)?; // sps_video_parameter_set_id
    let sublayers = bits.read(3)? as usize;
    if sublayers > 6 {
        return None;
    }
    bits.skip(1)?; // sps_temporal_id_nesting_flag
    // profile_tier_level: general profile(8), compatibility(32), constraints(48), level(8).
    bits.skip(96)?;
    let mut profile = [false; 7];
    let mut level = [false; 7];
    for i in 0..sublayers {
        profile[i] = bits.read(1)? != 0;
        level[i] = bits.read(1)? != 0;
    }
    if sublayers > 0 {
        bits.skip(2 * (8 - sublayers))?;
    }
    for i in 0..sublayers {
        if profile[i] {
            bits.skip(88)?;
        }
        if level[i] {
            bits.skip(8)?;
        }
    }
    bits.ue()?; // sps_seq_parameter_set_id
    let chroma = bits.ue()?;
    if chroma > 3 {
        return None;
    }
    let separate_plane = chroma == 3 && bits.read(1)? != 0;
    let width = bits.ue()?;
    let height = bits.ue()?;
    if !(1..=8192).contains(&width) || !(1..=8192).contains(&height) {
        return None;
    }
    let mut crop = [0u32; 4];
    if bits.read(1)? != 0 {
        for value in &mut crop {
            *value = bits.ue()?;
        }
    }
    let array_type = if separate_plane { 0 } else { chroma };
    let sub_w = if array_type == 1 || array_type == 2 {
        2
    } else {
        1
    };
    let sub_h = if array_type == 1 { 2 } else { 1 };
    let left = crop[0].checked_mul(sub_w)?;
    let right = crop[1].checked_mul(sub_w)?;
    let top = crop[2].checked_mul(sub_h)?;
    let bottom = crop[3].checked_mul(sub_h)?;
    let display_width = width.checked_sub(left.checked_add(right)?)?;
    let display_height = height.checked_sub(top.checked_add(bottom)?)?;
    if display_width == 0 || display_height == 0 {
        return None;
    }
    Some(HevcGeometry {
        coded_width: width,
        coded_height: height,
        display_width,
        display_height,
        crop_left: left,
        crop_top: top,
        crop_right: right,
        crop_bottom: bottom,
    })
}

struct Bits<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl Bits<'_> {
    fn skip(&mut self, count: usize) -> Option<()> {
        self.offset = self.offset.checked_add(count)?;
        (self.offset <= self.bytes.len() * 8).then_some(())
    }
    fn read(&mut self, count: usize) -> Option<u32> {
        if count > 32 || self.offset.checked_add(count)? > self.bytes.len() * 8 {
            return None;
        }
        let mut value = 0;
        for _ in 0..count {
            value = (value << 1)
                | u32::from((self.bytes[self.offset / 8] >> (7 - self.offset % 8)) & 1);
            self.offset += 1;
        }
        Some(value)
    }
    fn ue(&mut self) -> Option<u32> {
        let mut leading = 0;
        while self.read(1)? == 0 {
            leading += 1;
            if leading > 30 {
                return None;
            }
        }
        let suffix = self.read(leading)?;
        Some(((1u32 << leading) - 1) + suffix)
    }
}
