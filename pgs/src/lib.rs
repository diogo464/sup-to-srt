use std::io::{Cursor, Read};

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

#[derive(Debug, Clone, Copy)]
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
    /// uniquely identifies the window
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

#[derive(Debug, Clone, Copy)]
pub struct PaletteEntry {
    pub entry_id: u8,
    pub luminance: u8,
    pub color_diff_red: u8,
    pub color_diff_blue: u8,
    pub transparency: u8,
}

#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub pts: u32,
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
    ///
    pub palette_update: bool,
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
    /// uniquely identifies the palette
    pub palette_id: u8,
    /// version of the palette within the epoch
    pub palette_version: u8,
    pub entries: Vec<PaletteEntry>,
}

/// Object Definition Segment
#[derive(Debug, Clone)]
pub struct ODS {
    pub header: Header,
    /// uniquely identifies this object in the epoch.
    /// an [`ODS`] segment with an object id that was already seen in the current epoch should
    /// update the existing object.
    pub object_id: u16,
    pub object_version: u8,
    pub last_in_sequence: LastInSequenceFlag,
    /// the width for an object id should always be the same for a given epoch.
    pub width: u16,
    /// the height for an object id should always be the same for a given epoch.
    pub height: u16,
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
            let mut entries = Vec::new();
            while cursor.position() < buffer.len() as u64 {
                let entry = wire::PaletteEntry::read(&mut cursor)?;
                entries.push(PaletteEntry {
                    entry_id: entry.palette_entry_id,
                    luminance: entry.luminance,
                    color_diff_red: entry.color_diff_red,
                    color_diff_blue: entry.color_diff_blue,
                    transparency: entry.transparency,
                });
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

            Ok(Segment::ODS(ODS {
                header: Header::from(header),
                object_id: ods.object_id,
                object_version: ods.object_version,
                last_in_sequence: flag,
                width: ods.width,
                height: ods.height,
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
