use std::io::Read;

// https://blog.thescorpius.com/index.php/2017/07/15/presentation-graphic-stream-sup-files-bluray-subtitle-format/

pub const MAGIC_NUMBER: u16 = 0x5047; // b"PG"

pub const SEGMENT_TYPE_PDS: u8 = 0x14;
pub const SEGMENT_TYPE_ODS: u8 = 0x15;
pub const SEGMENT_TYPE_PCS: u8 = 0x16;
pub const SEGMENT_TYPE_WDS: u8 = 0x17;
pub const SEGMENT_TYPE_END: u8 = 0x80;

pub const FRAME_RATE: u8 = 0x10;

pub const COMPOSITION_STATE_NORMAL: u8 = 0x00;
pub const COMPOSITION_STATE_ACQUISITION_POINT: u8 = 0x40;
pub const COMPOSITION_STATE_EPOCH_START: u8 = 0x80;

pub const PALETTE_UPDATE_FLAG_FALSE: u8 = 0x00;
pub const PALETTE_UPDATE_FLAG_TRUE: u8 = 0x80;

pub const OBJECT_CROPPED_FLAG_OFF: u8 = 0x00;
pub const OBJECT_CROPPED_FLAG_FORCE: u8 = 0x40;

pub const LAST_IN_SEQUENCE_FLAG_LAST_IN_SEQ: u8 = 0x40;
pub const LAST_IN_SEQUENCE_FLAG_FIRST_IN_SEQ: u8 = 0x80;
pub const LAST_IN_SEQUENCE_FLAG_FIRST_AND_LAST_IN_SEQ: u8 =
    LAST_IN_SEQUENCE_FLAG_FIRST_IN_SEQ | LAST_IN_SEQUENCE_FLAG_LAST_IN_SEQ;

pub trait Wire: Sized {
    fn read<R: Read>(reader: R) -> std::io::Result<Self>;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SegmentHeader {
    pub magic_number: u16,
    pub pts: u32,
    pub dts: u32,
    pub segment_type: u8,
    pub segment_size: u16,
}

/// Presentation Composition Segment
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SegmentPCS {
    pub width: u16,
    pub height: u16,
    pub framerate: u8,
    pub composition_number: u16,
    pub composition_state: u8,
    pub palette_update_flag: u8,
    pub palette_id: u8,
    pub number_of_composition_objects: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CompositionObject {
    pub object_id: u16,
    pub window_id: u8,
    pub object_cropped_flag: u8,
    pub object_horizontal_position: u16,
    pub object_vertical_position: u16,
    pub object_cropping_horizontal_position: u16,
    pub object_cropping_vertical_position: u16,
    pub object_cropping_width: u16,
    pub object_cropping_height: u16,
}

/// Window Definition Segment
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SegmentWDS {
    pub number_of_windows: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub window_id: u8,
    pub window_horizontal_position: u16,
    pub window_vertical_position: u16,
    pub window_width: u16,
    pub window_height: u16,
}

/// Palette Definition Segment
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SegmentPDS {
    pub palette_id: u8,
    pub palette_version: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PaletteEntry {
    pub palette_entry_id: u8,
    pub luminance: u8,
    pub color_diff_red: u8,
    pub color_diff_blue: u8,
    pub transparency: u8,
}

/// Object Definition Segment
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SegmentODS {
    pub object_id: u16,
    pub object_version: u8,
    pub last_in_sequence_flag: u8,
    pub object_data_length: u32,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDataCode {
    Color { color: u8, count: u16 },
    EndOfLine,
}

struct ImageDataDecoder<'a> {
    buf: &'a [u8],
    offset: usize,
}

impl<'a> ImageDataDecoder<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, offset: 0 }
    }
}

impl<'a> Iterator for ImageDataDecoder<'a> {
    type Item = std::io::Result<ImageDataCode>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset == self.buf.len() {
            return None;
        }
        let (code, n) = match decode_image_data_code(&self.buf[self.offset..]) {
            Ok((code, n)) => (code, n),
            Err(err) => return Some(Err(err)),
        };
        self.offset += n;
        Some(Ok(code))
    }
}

pub fn decode_image_data_code(buf: &[u8]) -> std::io::Result<(ImageDataCode, usize)> {
    if buf.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "empty buffer",
        ));
    }

    let v0 = buf[0];
    if v0 > 0 {
        return Ok((
            ImageDataCode::Color {
                color: v0,
                count: 1,
            },
            1,
        ));
    }

    if buf.len() < 2 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not enough data",
        ));
    }

    let v1 = buf[1];
    if v1 >= 1 && v1 <= 63 {
        return Ok((
            ImageDataCode::Color {
                color: 0,
                count: u16::from(v1),
            },
            2,
        ));
    }

    if v1 == 0 {
        return Ok((ImageDataCode::EndOfLine, 2));
    }

    if buf.len() < 3 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not enough data",
        ));
    }

    let v2 = buf[2];
    if v1 & 0b11000000 == 0b01000000 {
        let v1 = u16::from(v1);
        let v2 = u16::from(v2);
        let n = (v1 & 0b00111111) << 8 | v2;
        let n = n.saturating_sub(1);
        return Ok((ImageDataCode::Color { color: 0, count: n }, 3));
    }

    if v1 & 0b11000000 == 0b10000000 {
        let n = u16::from(v1 & 0b00111111);
        let c = v2;
        return Ok((ImageDataCode::Color { color: c, count: n }, 3));
    }

    if buf.len() < 4 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "not enough data",
        ));
    }

    let v3 = buf[3];
    if v1 & 0b11000000 == 0b11000000 {
        let v1 = u16::from(v1);
        let v2 = u16::from(v2);
        let c = v3;
        let n = (v1 & 0b00111111) << 8 | v2;
        return Ok((ImageDataCode::Color { color: c, count: n }, 4));
    }

    return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid rle data",
    ));
}

pub fn decode_image_data(buf: &[u8]) -> impl Iterator<Item = std::io::Result<ImageDataCode>> + '_ {
    ImageDataDecoder::new(buf)
}

fn read_u8<R: Read>(reader: &mut R) -> std::io::Result<u8> {
    let mut buf = [0u8; 1];
    reader.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn read_u16<R: Read>(reader: &mut R) -> std::io::Result<u16> {
    let mut buf = [0u8; 2];
    reader.read_exact(&mut buf)?;
    Ok(u16::from_be_bytes(buf))
}

fn read_u24<R: Read>(reader: &mut R) -> std::io::Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf[1..])?;
    Ok(u32::from_be_bytes(buf))
}

fn read_u32<R: Read>(reader: &mut R) -> std::io::Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

impl Wire for SegmentHeader {
    fn read<R: Read>(mut reader: R) -> std::io::Result<Self> {
        Ok(Self {
            magic_number: read_u16(&mut reader)?,
            pts: read_u32(&mut reader)?,
            dts: read_u32(&mut reader)?,
            segment_type: read_u8(&mut reader)?,
            segment_size: read_u16(&mut reader)?,
        })
    }
}

impl Wire for SegmentPCS {
    fn read<R: Read>(mut reader: R) -> std::io::Result<Self> {
        Ok(Self {
            width: read_u16(&mut reader)?,
            height: read_u16(&mut reader)?,
            framerate: read_u8(&mut reader)?,
            composition_number: read_u16(&mut reader)?,
            composition_state: read_u8(&mut reader)?,
            palette_update_flag: read_u8(&mut reader)?,
            palette_id: read_u8(&mut reader)?,
            number_of_composition_objects: read_u8(&mut reader)?,
        })
    }
}

impl Wire for CompositionObject {
    fn read<R: Read>(mut reader: R) -> std::io::Result<Self> {
        let mut s = Self::default();
        s.object_id = read_u16(&mut reader)?;
        s.window_id = read_u8(&mut reader)?;
        s.object_cropped_flag = read_u8(&mut reader)?;
        s.object_horizontal_position = read_u16(&mut reader)?;
        s.object_vertical_position = read_u16(&mut reader)?;
        if s.object_cropped_flag == OBJECT_CROPPED_FLAG_FORCE {
            s.object_cropping_horizontal_position = read_u16(&mut reader)?;
            s.object_cropping_vertical_position = read_u16(&mut reader)?;
            s.object_cropping_width = read_u16(&mut reader)?;
            s.object_cropping_height = read_u16(&mut reader)?;
        }
        Ok(s)
    }
}

impl Wire for SegmentWDS {
    fn read<R: Read>(mut reader: R) -> std::io::Result<Self> {
        Ok(Self {
            number_of_windows: read_u8(&mut reader)?,
        })
    }
}

impl Wire for Window {
    fn read<R: Read>(mut reader: R) -> std::io::Result<Self> {
        Ok(Self {
            window_id: read_u8(&mut reader)?,
            window_horizontal_position: read_u16(&mut reader)?,
            window_vertical_position: read_u16(&mut reader)?,
            window_width: read_u16(&mut reader)?,
            window_height: read_u16(&mut reader)?,
        })
    }
}

impl Wire for SegmentPDS {
    fn read<R: Read>(mut reader: R) -> std::io::Result<Self> {
        Ok(Self {
            palette_id: read_u8(&mut reader)?,
            palette_version: read_u8(&mut reader)?,
        })
    }
}

impl Wire for PaletteEntry {
    fn read<R: Read>(mut reader: R) -> std::io::Result<Self> {
        Ok(Self {
            palette_entry_id: read_u8(&mut reader)?,
            luminance: read_u8(&mut reader)?,
            color_diff_red: read_u8(&mut reader)?,
            color_diff_blue: read_u8(&mut reader)?,
            transparency: read_u8(&mut reader)?,
        })
    }
}

impl Wire for SegmentODS {
    fn read<R: Read>(mut reader: R) -> std::io::Result<Self> {
        Ok(Self {
            object_id: read_u16(&mut reader)?,
            object_version: read_u8(&mut reader)?,
            last_in_sequence_flag: read_u8(&mut reader)?,
            object_data_length: read_u24(&mut reader)?,
            width: read_u16(&mut reader)?,
            height: read_u16(&mut reader)?,
        })
    }
}
