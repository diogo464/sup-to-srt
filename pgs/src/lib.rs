use std::{
    io::{Cursor, Read},
    time::Duration,
};

pub mod wire;

/// The graphics stream is made up of Functional Segments.
/// The functional segments are:
/// + Presentation Composition Segment (PCS)
/// + Window Definition Segment (WDS)
/// + Palette Definition Segment (PDS)
/// + Object Definition Segment (ODS)
/// + END of display set segment (END)
///
/// Screen Composition Segment: PCS
/// Definition Segment: WDS, PDS, ODS, END

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CompositionState {
    Normal,
    AcquisitionPoint,
    EpochStart,
}

#[derive(Debug, Clone, Copy)]
pub enum LastInSequenceFlag {
    Last,
    First,
    FirstAndLast,
}

#[derive(Debug, Clone)]
pub enum Segment {
    PCS(PCS),
    WDS(WDS),
    PDS(PDS),
    ODS(ODS),
    END(END),
}

#[derive(Debug, Clone, Copy)]
pub struct CompositionObjectCropping {
    pub width: u16,
    pub height: u16,
    pub horizontal_position: u16,
    pub vertical_position: u16,
}

#[derive(Debug, Clone)]
pub struct CompositionObject {
    pub object_id: u16,
    pub window_id: u8,
    pub horizontal_position: u16,
    pub vertical_position: u16,
    pub cropping: Option<CompositionObjectCropping>,
}

#[derive(Debug, Clone, Copy)]
pub struct Window {
    /// uniquely identifies the window in the epoch
    pub window_id: u8,
    /// range of 1 to (video width)-(window horizontal position)
    pub width: u16,
    /// range of 1 to (videoheight)-(windowVerticalposition)
    pub height: u16,
    /// top left pixel position
    pub horizontal_position: u16,
    /// top left pixel position
    pub vertical_position: u16,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct PaletteEntry {
    pub entry_id: u8,
    pub luminance: u8,
    pub color_diff_red: u8,
    pub color_diff_blue: u8,
    pub transparency: u8,
}

impl PaletteEntry {
    pub fn to_rgb(&self) -> (u8, u8, u8) {
        ycbcr_to_rgb(self.luminance, self.color_diff_red, self.color_diff_blue)
    }

    pub fn to_rgba(&self) -> (u8, u8, u8, u8) {
        if self.transparency == 0 {
            (0, 0, 0, 0)
        } else {
            let (r, g, b) = ycbcr_to_rgb(self.luminance, self.color_diff_red, self.color_diff_blue);
            (r, g, b, self.transparency)
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Header {
    /// presentation time stamp (measured in ticks of 90khz clock)
    pub pts: u32,
    /// decoding time stamp (measured in ticks of 90khz clock)
    pub dts: u32,
}

impl From<wire::SegmentHeader> for Header {
    fn from(value: wire::SegmentHeader) -> Self {
        Self {
            pts: value.pts,
            dts: value.dts,
        }
    }
}

/// Presentation Composition Segment
#[derive(Debug, Clone)]
pub struct PCS {
    pub header: Header,
    pub width: u16,
    pub height: u16,
    /// identifies this Graphics Update in the current Display Segment.
    /// in range 0 - 15
    pub composition_number: u16,
    pub composition_state: CompositionState,
    /// indicates if this PCS describes a Palette only Display Update
    pub palette_update: bool,
    /// identifies the palette to be used in the Palette only Display Update
    pub palette_id: u8,
    pub composition_objects: Vec<CompositionObject>,
}

/// Window Definition Segment
#[derive(Debug, Clone)]
pub struct WDS {
    pub header: Header,
    pub windows: Vec<Window>,
}

/// Palette Definition Segment
#[derive(Debug, Clone)]
pub struct PDS {
    pub header: Header,
    /// uniquely identifies the palette in the epoch
    pub palette_id: u8,
    /// version of the palette within the epoch
    pub palette_version: u8,
    pub entries: [PaletteEntry; 256],
}

/// Object Definition Segment
#[derive(Debug, Clone)]
pub struct ODS {
    pub header: Header,
    /// uniquely identifies this object in the epoch.
    /// an [`ODS`] segment with an object id that was already seen in the current epoch should
    /// update the existing object.
    pub object_id: u16,
    /// version of the object within the epoch
    pub object_version: u8,
    pub last_in_sequence: LastInSequenceFlag,
    /// the width for an object id should always be the same for a given epoch.
    pub width: u16,
    /// the height for an object id should always be the same for a given epoch.
    pub height: u16,
    /// vector with rle image data
    pub data: Vec<u8>,
}

/// END of display set segment
#[derive(Debug, Clone)]
pub struct END {
    pub header: Header,
}

#[derive(Debug, Clone)]
pub struct DisplaySet {
    pub pcs: PCS,
    pub wds: Vec<WDS>,
    pub pds: Vec<PDS>,
    pub ods: Vec<ODS>,
    pub end: END,
}

pub fn decode_segment<R: Read>(mut reader: R) -> std::io::Result<Segment> {
    use wire::Wire;

    let header = wire::SegmentHeader::read(&mut reader)?;
    if header.magic_number != wire::MAGIC_NUMBER {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid header magic value",
        ));
    }

    let mut buffer = Vec::new();
    buffer.resize(header.segment_size as usize, 0);
    reader.read_exact(&mut buffer)?;
    let mut cursor = Cursor::new(&buffer);

    match header.segment_type {
        wire::SEGMENT_TYPE_PCS => {
            let pcs = wire::SegmentPCS::read(&mut cursor)?;
            let mut objects = Vec::with_capacity(pcs.number_of_composition_objects as usize);

            for _ in 0..pcs.number_of_composition_objects {
                let object = wire::CompositionObject::read(&mut cursor)?;
                let cropping = match object.object_cropped_flag {
                    wire::OBJECT_CROPPED_FLAG_OFF => None,
                    wire::OBJECT_CROPPED_FLAG_FORCE => Some(CompositionObjectCropping {
                        width: object.object_cropping_width,
                        height: object.object_cropping_height,
                        horizontal_position: object.object_cropping_horizontal_position,
                        vertical_position: object.object_cropping_vertical_position,
                    }),
                    _ => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "invalid object cropped flag",
                        ))
                    }
                };
                objects.push(CompositionObject {
                    object_id: object.object_id,
                    window_id: object.window_id,
                    horizontal_position: object.object_horizontal_position,
                    vertical_position: object.object_vertical_position,
                    cropping,
                });
            }

            let palette_update = match pcs.palette_update_flag {
                wire::PALETTE_UPDATE_FLAG_TRUE => true,
                wire::PALETTE_UPDATE_FLAG_FALSE => false,
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid palette update flag",
                    ))
                }
            };

            let composition_state = match pcs.composition_state {
                wire::COMPOSITION_STATE_EPOCH_START => CompositionState::EpochStart,
                wire::COMPOSITION_STATE_ACQUISITION_POINT => CompositionState::AcquisitionPoint,
                wire::COMPOSITION_STATE_NORMAL => CompositionState::Normal,
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid composition state",
                    ))
                }
            };

            Ok(Segment::PCS(PCS {
                header: Header::from(header),
                width: pcs.width,
                height: pcs.height,
                composition_number: pcs.composition_number,
                composition_state,
                palette_update,
                palette_id: pcs.palette_id,
                composition_objects: objects,
            }))
        }
        wire::SEGMENT_TYPE_WDS => {
            let wds = wire::SegmentWDS::read(&mut cursor)?;
            let mut windows = Vec::with_capacity(wds.number_of_windows as usize);
            for _ in 0..wds.number_of_windows {
                let wnd = wire::Window::read(&mut cursor)?;
                windows.push(Window {
                    window_id: wnd.window_id,
                    width: wnd.window_width,
                    height: wnd.window_height,
                    horizontal_position: wnd.window_horizontal_position,
                    vertical_position: wnd.window_vertical_position,
                });
            }
            Ok(Segment::WDS(WDS {
                header: Header::from(header),
                windows,
            }))
        }
        wire::SEGMENT_TYPE_PDS => {
            let pds = wire::SegmentPDS::read(&mut cursor)?;
            let mut entries: [PaletteEntry; 256] = unsafe { std::mem::zeroed() };
            while cursor.position() < buffer.len() as u64 {
                let entry = wire::PaletteEntry::read(&mut cursor)?;
                entries[entry.palette_entry_id as usize] = PaletteEntry {
                    entry_id: entry.palette_entry_id,
                    luminance: entry.luminance,
                    color_diff_red: entry.color_diff_red,
                    color_diff_blue: entry.color_diff_blue,
                    transparency: entry.transparency,
                };
            }
            Ok(Segment::PDS(PDS {
                header: Header::from(header),
                palette_id: pds.palette_id,
                palette_version: pds.palette_version,
                entries,
            }))
        }
        wire::SEGMENT_TYPE_ODS => {
            let ods = wire::SegmentODS::read(&mut cursor)?;
            let mut data = vec![0u8; ods.object_data_length.saturating_sub(4) as usize];
            cursor.read_exact(&mut data)?;

            let flag = match ods.last_in_sequence_flag {
                wire::LAST_IN_SEQUENCE_FLAG_FIRST_IN_SEQ => LastInSequenceFlag::First,
                wire::LAST_IN_SEQUENCE_FLAG_LAST_IN_SEQ => LastInSequenceFlag::Last,
                wire::LAST_IN_SEQUENCE_FLAG_FIRST_AND_LAST_IN_SEQ => {
                    LastInSequenceFlag::FirstAndLast
                }
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "invalid last in sequence flag",
                    ))
                }
            };

            // let pixels_expected_count = ods.width as usize * ods.height as usize;
            // let mut pixels = Vec::with_capacity(pixels_expected_count);
            // let mut pixels_in_line = 0u16;
            // for code in wire::decode_image_data(&data) {
            //     let code = code?;
            //     let (color, count) = match code {
            //         wire::ImageDataCode::Color { color, count } => (color, count),
            //         wire::ImageDataCode::EndOfLine => (0, pixels_in_line.saturating_sub(ods.width)),
            //     };
            //     pixels_in_line += count;
            //     pixels.extend(std::iter::repeat_n(color, count as usize));
            //     if code == wire::ImageDataCode::EndOfLine {
            //         pixels_in_line = 0;
            //     }
            // }
            // assert_eq!(pixels.len(), pixels_expected_count);

            Ok(Segment::ODS(ODS {
                header: Header::from(header),
                object_id: ods.object_id,
                object_version: ods.object_version,
                last_in_sequence: flag,
                width: ods.width,
                height: ods.height,
                data,
            }))
        }
        wire::SEGMENT_TYPE_END => Ok(Segment::END(END {
            header: Header::from(header),
        })),
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid segment type",
            ))
        }
    }
}

pub fn decode_display_set<R: Read>(mut reader: R) -> std::io::Result<DisplaySet> {
    let pcs = match decode_segment(&mut reader)? {
        Segment::PCS(pcs) => pcs,
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "expected pcs as first segment in display set",
            ))
        }
    };

    let mut vwds = Vec::new();
    let mut vpds = Vec::new();
    let mut vods = Vec::new();

    loop {
        match decode_segment(&mut reader)? {
            Segment::PCS(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "found PCS in the middle of display set",
                ))
            }
            Segment::WDS(wds) => vwds.push(wds),
            Segment::PDS(pds) => vpds.push(pds),
            Segment::ODS(ods) => vods.push(ods),
            Segment::END(end) => {
                return Ok(DisplaySet {
                    pcs,
                    wds: vwds,
                    pds: vpds,
                    ods: vods,
                    end,
                })
            }
        }
    }
}

pub fn decode_display_sets<R: Read>(mut reader: R) -> std::io::Result<Vec<DisplaySet>> {
    let mut display_sets = Vec::default();
    loop {
        match decode_display_set(&mut reader) {
            Ok(display_set) => display_sets.push(display_set),
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        }
    }
    Ok(display_sets)
}

pub fn ycbcr_to_rgb(luminance: u8, cr: u8, cb: u8) -> (u8, u8, u8) {
    // Convert YCbCr to RGB using the formula
    let luminance = luminance as f64;
    let cr = cr as f64;
    let cb = cb as f64;

    let r = luminance + 1.402 * (cr - 128.0);
    let g = luminance - 0.344136 * (cb - 128.0) - 0.714136 * (cr - 128.0);
    let b = luminance + 1.772 * (cb - 128.0);

    // Ensure RGB values are within the 0-255 range
    let r = r.clamp(0.0, 255.0) as u8;
    let g = g.clamp(0.0, 255.0) as u8;
    let b = b.clamp(0.0, 255.0) as u8;

    (r, g, b)
}

/// convert timestamp in the 90khz clock to a [`std::time::Duration`].
pub fn clock_to_duration(timestamp: u32) -> Duration {
    let seconds = timestamp / 90_000;
    let remain = timestamp % 90_000;
    let nanos_per_tick = 1_000_000_000 / 90_000;
    let nanos = remain * nanos_per_tick;
    Duration::new(u64::from(seconds), nanos)
}

/// decode the rle image data into a vector containing the pixels of the image.
/// each pixel value is an index into the color palette.
pub fn decode_rle_data(data: &[u8], width: u16, height: u16) -> std::io::Result<Vec<u8>> {
    let expected_pixel_count = width as usize * height as usize;
    let mut pixels = Vec::with_capacity(expected_pixel_count);
    for code in wire::decode_image_data(data) {
        let code = code?;
        if let wire::ImageDataCode::Color { color, count } = code {
            pixels.extend(std::iter::repeat_n(color, count as usize));
        }
    }
    if pixels.len() != expected_pixel_count {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "rle data expanded with invalid lenght",
        ));
    }
    Ok(pixels)
}
