use std::{
    collections::HashMap,
    io::{Cursor, Read},
    path::PathBuf,
    time::Duration,
};

use clap::Parser;
use color_eyre::{
    eyre::{eyre, Context},
    Result,
};
use minifb::{Key, KeyRepeat};

#[derive(Debug, Parser)]
struct Args {
    /// Open the subtitle image viewer.
    ///
    /// Use A and D to cycle trought the images.
    #[clap(long)]
    view: bool,

    /// input pgs/.sup file, must exist.
    /// if not specified then the input is read from stdin.
    input: Option<PathBuf>,

    /// output srt file, must not exist.
    /// if not specified then the output goes to stdout.
    output: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimeRange {
    begin: Duration,
    end: Duration,
}

impl TimeRange {
    fn new(begin: Duration, end: Duration) -> Self {
        Self { begin, end }
    }
}

#[derive(Debug, Default, Clone)]
struct Bitmap {
    width: u32,
    height: u32,
    /// RGBA 8-bit per channel data
    pixels: Vec<u8>,
}

impl Bitmap {
    fn sub_image(&self, top_left_x: u32, top_left_y: u32, width: u32, height: u32) -> Bitmap {
        let mut output_pixels = Vec::with_capacity((4 * width * height) as usize);

        for y in top_left_y..top_left_y.saturating_add(height).min(self.height) {
            let begin_offset = (y * self.width * 4) as usize + top_left_x as usize * 4;
            let end_offset = begin_offset + width as usize * 4;
            let line = &self.pixels[begin_offset..end_offset];
            output_pixels.extend(line);
        }

        Self {
            width,
            height,
            pixels: output_pixels,
        }
    }
}

#[derive(Debug, Clone)]
struct BitmapSubtitle {
    range: TimeRange,
    bitmap: Bitmap,
}

#[derive(Debug, Clone)]
struct TextSubtitle {
    range: TimeRange,
    text: String,
}

fn main() -> Result<()> {
    color_eyre::install().unwrap();
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    let input_data = match args.input {
        Some(path) => {
            tracing::info!("reading from {}", path.display());
            std::fs::read(&path).context("reading from input file")?
        }
        None => {
            tracing::info!("reading from stdin");
            let mut stdin = std::io::stdin().lock();
            let mut buf = Vec::default();
            stdin.read_to_end(&mut buf).context("reading from stdin")?;
            buf
        }
    };

    tracing::info!("extracting bitmap subtitles from input");
    let bitmap_subtitles = subtitles_extract(&input_data)?;
    tracing::info!("extracted {} bitmap subtitles", bitmap_subtitles.len());

    if args.view {
        subtitles_viewer(bitmap_subtitles)?;
    } else {
        tracing::info!("performing OCR on bitmap subtitles");
        let text_subtitles = subtitles_ocr(bitmap_subtitles)?;
        tracing::info!("OCR complete");

        tracing::info!("generating srt");
        let srt = subtitles_to_srt(text_subtitles);

        print!("{srt}");
    }

    Ok(())
}

fn subtitles_extract(pgs: &[u8]) -> Result<Vec<BitmapSubtitle>> {
    struct Object {
        width: u16,
        height: u16,
        finished: bool,
        data: Vec<u8>,
        bitmap: Bitmap,
    }

    fn bitmap_from_object_and_palette(object: &Object, palette: &pgs::PDS) -> Result<Bitmap> {
        let pixels_indexed = pgs::decode_rle_data(&object.data, object.width, object.height)
            .context("decoding ODS rle data")?;
        let mut pixels = Vec::with_capacity(pixels_indexed.len());
        for idx in pixels_indexed {
            let (r, g, b, a) = palette.entries[idx as usize].to_rgba();
            pixels.extend([r, g, b, a]);
        }
        Ok(Bitmap {
            width: u32::from(object.width),
            height: u32::from(object.height),
            pixels,
        })
    }

    let display_sets = pgs::decode_display_sets(Cursor::new(pgs)).context("parsing pgs")?;
    if display_sets.is_empty() {
        tracing::warn!("display_sets.len() = 0 ");
        return Ok(Default::default());
    }

    let display_set_0 = &display_sets[0];
    if display_set_0.pcs.composition_state != pgs::CompositionState::EpochStart {
        return Err(eyre!("display set 0 does not start an epoch"));
    }

    let display_width = display_set_0.pcs.width;
    let display_height = display_set_0.pcs.height;
    let mut current_epoch = 0;
    let mut objects: HashMap<u16, Object> = Default::default();
    let mut palettes: HashMap<u8, pgs::PDS> = Default::default();
    let mut subtitles: Vec<BitmapSubtitle> = Vec::default();
    // index of images inserted in the previous display set
    // used to patch the end time
    let mut previous_subtitles: Vec<usize> = Vec::default();

    for ds in display_sets {
        assert_eq!(ds.pcs.width, display_width);
        assert_eq!(ds.pcs.height, display_height);

        let current_time = pgs::clock_to_duration(ds.pcs.header.pts);
        for subtitle_idx in previous_subtitles.drain(..) {
            subtitles[subtitle_idx].range.end = current_time;
        }

        match ds.pcs.composition_state {
            pgs::CompositionState::EpochStart => {
                current_epoch += 1;
                objects.clear();
                palettes.clear();
                tracing::debug!("moving to epoch {current_epoch}");
            }
            pgs::CompositionState::Normal => {}
            pgs::CompositionState::AcquisitionPoint => {}
        }

        for pds in ds.pds {
            tracing::debug!("found palette {}", pds.palette_id);
            palettes.insert(pds.palette_id, pds);
        }

        let palette = match palettes.get(&ds.pcs.palette_id) {
            Some(palette) => palette,
            None => {
                return Err(eyre!("PCS referenced invalid palette"));
            }
        };

        for ods in ds.ods {
            let obj = objects.entry(ods.object_id).or_insert(Object {
                width: ods.width,
                height: ods.height,
                finished: false,
                data: Default::default(),
                bitmap: Default::default(),
            });

            match ods.last_in_sequence {
                pgs::LastInSequenceFlag::FirstAndLast => {
                    obj.finished = true;
                    obj.data.clear();
                    obj.data.extend(ods.data);
                    obj.bitmap = bitmap_from_object_and_palette(obj, palette)?;
                }
                pgs::LastInSequenceFlag::First => {
                    obj.finished = false;
                    obj.data.clear();
                    obj.data.extend(ods.data);
                }
                pgs::LastInSequenceFlag::Last => {
                    if obj.finished {
                        tracing::error!(
                            "received ODS with flag LAST but object was already finished"
                        );
                        return Err(eyre!("invalid ods segment"));
                    }
                    obj.finished = true;
                    obj.data.extend(ods.data);
                    obj.bitmap = bitmap_from_object_and_palette(obj, palette)?;
                }
            }
        }

        for comp in ds.pcs.composition_objects {
            let object = match objects.get(&comp.object_id) {
                Some(object) => object,
                None => {
                    tracing::warn!(
                        "invalid object id in composition object: {}",
                        comp.object_id
                    );
                    continue;
                }
            };

            if !object.finished {
                tracing::warn!(
                    "unfinished object in composition object: {}",
                    comp.object_id
                );
                continue;
            }

            let bitmap = if let Some(cropping) = comp.cropping {
                let image = object.bitmap.sub_image(
                    u32::from(cropping.horizontal_position),
                    u32::from(cropping.vertical_position),
                    u32::from(cropping.width),
                    u32::from(cropping.height),
                );
                image
            } else {
                object.bitmap.clone()
            };

            previous_subtitles.push(subtitles.len());
            subtitles.push(BitmapSubtitle {
                range: TimeRange::new(current_time, Default::default()),
                bitmap,
            });
        }
    }

    Ok(subtitles)
}

fn subtitles_ocr(subtitles: Vec<BitmapSubtitle>) -> Result<Vec<TextSubtitle>> {
    let mut text_subtitles = Vec::with_capacity(subtitles.len());
    let (ocr_in_sender, ocr_in_receiver) = crossbeam::channel::unbounded::<BitmapSubtitle>();
    let (ocr_out_sender, ocr_out_receiver) = crossbeam::channel::unbounded::<TextSubtitle>();

    for subtitle in subtitles {
        ocr_in_sender.send(subtitle).unwrap();
    }
    drop(ocr_in_sender);

    tracing::info!("starting ocr");
    std::thread::scope(|scope| -> Result<()> {
        let mut handles = Vec::new();
        for _ in 0..std::thread::available_parallelism()
            .map(|v| usize::from(v))
            .unwrap_or(4)
        {
            let handle = scope.spawn(|| -> Result<()> {
                let mut tesseract = tesseract::Tesseract::new(None, Some("eng"))
                    .context("initializing tesseract")?;
                while let Ok(subtitle) = ocr_in_receiver.recv() {
                    let image = &subtitle.bitmap;
                    tesseract = tesseract
                        .set_frame(
                            &image.pixels,
                            image.width as i32,
                            image.height as i32,
                            4,
                            image.width as i32 * 4,
                        )
                        .context("setting tesseract frame")?;
                    tesseract = tesseract.recognize().context("tesseract recognize")?;
                    let text = tesseract.get_text().context("tesseract get text")?;
                    ocr_out_sender
                        .send(TextSubtitle {
                            range: subtitle.range,
                            text,
                        })
                        .unwrap();
                }
                Ok(())
            });
            handles.push(handle);
        }
        for handle in handles {
            handle.join().unwrap()?;
        }
        Ok(())
    })?;

    drop(ocr_out_sender);
    while let Ok(text_subtitle) = ocr_out_receiver.recv() {
        text_subtitles.push(text_subtitle);
    }
    Ok(text_subtitles)
}

fn srt_duration_display(duration: Duration) -> impl std::fmt::Display {
    struct SrtDurationDisplay(Duration);

    impl std::fmt::Display for SrtDurationDisplay {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            let total_secs = self.0.as_secs();
            let hours = total_secs / 3600;
            let minutes = (total_secs / 60) % 60;
            let seconds = total_secs % 60;
            let millis = self.0.subsec_millis();
            write!(f, "{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
        }
    }

    SrtDurationDisplay(duration)
}

fn subtitles_to_srt(subtitles: Vec<TextSubtitle>) -> String {
    use std::fmt::Write;

    #[derive(Debug, PartialEq, Eq)]
    enum ActionKind {
        Add,
        Remove,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Action {
        kind: ActionKind,
        subtitle: usize,
        timestamp: Duration,
    }

    impl std::cmp::PartialOrd for Action {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }

    impl std::cmp::Ord for Action {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.timestamp.cmp(&other.timestamp)
        }
    }

    // index of subtitles that are currently in display
    let mut on_screen: Vec<usize> = Default::default();
    let mut on_screen_text = String::default();
    let mut actions: Vec<Action> = Default::default();
    let mut srt = String::default();
    let mut current_sub_num = 1;

    for (idx, subtitle) in subtitles.iter().enumerate() {
        actions.push(Action {
            kind: ActionKind::Add,
            subtitle: idx,
            timestamp: subtitle.range.begin,
        });
        actions.push(Action {
            kind: ActionKind::Remove,
            subtitle: idx,
            timestamp: subtitle.range.end,
        });
    }
    actions.sort();

    for (action_idx, action) in actions.iter().enumerate() {
        match action.kind {
            ActionKind::Add => on_screen.push(action.subtitle),
            ActionKind::Remove => on_screen.retain(|&x| x != action.subtitle),
        }

        on_screen_text.clear();
        for &idx in on_screen.iter() {
            on_screen_text.push_str(&subtitles[idx].text);
            on_screen_text.push('\n');
        }
        let on_screen_text = on_screen_text.trim();

        if !on_screen_text.is_empty() {
            let timestamp_begin = action.timestamp;
            let timestamp_end = match actions.get(action_idx + 1) {
                Some(action) => action.timestamp,
                None => Duration::MAX,
            };

            let _ = writeln!(srt, "{current_sub_num}");
            let _ = writeln!(
                srt,
                "{} --> {}",
                srt_duration_display(timestamp_begin),
                srt_duration_display(timestamp_end),
            );
            srt.push_str(&on_screen_text);
            srt.push_str("\n\n");
            current_sub_num += 1;
        }
    }

    srt
}

fn subtitles_viewer(subtitles: Vec<BitmapSubtitle>) -> Result<()> {
    let mut window = minifb::Window::new(
        "sup2srt",
        1200,
        1200,
        minifb::WindowOptions {
            scale_mode: minifb::ScaleMode::Center,
            ..Default::default()
        },
    )
    .unwrap();
    window.set_target_fps(60);

    let mut curr_image = 0usize;
    while window.is_open() && !window.is_key_down(Key::Escape) {
        if window.is_key_pressed(Key::A, KeyRepeat::No) {
            curr_image = curr_image.saturating_sub(1);
        }
        if window.is_key_pressed(Key::D, KeyRepeat::No) {
            curr_image = subtitles.len().saturating_sub(1).min(curr_image + 1);
        }

        let bitmap = &subtitles[curr_image].bitmap;
        let mut buffer = Vec::with_capacity(bitmap.pixels.len() / 4);
        for i in 0..bitmap.pixels.len() / 4 {
            let v = ((bitmap.pixels[i * 4] as u32) << 16)
                | ((bitmap.pixels[i * 4 + 1] as u32) << 8)
                | ((bitmap.pixels[i * 4 + 2] as u32) << 0);
            buffer.push(v);
        }
        window
            .update_with_buffer(&buffer, bitmap.width as usize, bitmap.height as usize)
            .context("updating window with image buffer")?;
    }

    Ok(())
}
