use std::{
    collections::{HashMap, VecDeque},
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
use slotmap::{SecondaryMap, SlotMap};

#[derive(Debug, Parser)]
struct Args {
    input: Option<PathBuf>,
}

slotmap::new_key_type! {
   struct SubtitleId;
}

struct Object {
    width: u16,
    height: u16,
    finished: bool,
    data: Vec<u8>,
    image: Image,
}

#[derive(Default, Clone)]
struct Image {
    width: u32,
    height: u32,
    /// RGBA 8-bit per channel
    pixels: Vec<u8>,
}

impl Image {
    fn sub_image(&self, top_left_x: u32, top_left_y: u32, width: u32, height: u32) -> Image {
        self.clone()
    }
}

struct DurationSrtDisplay(Duration);

impl std::fmt::Display for DurationSrtDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total_secs = self.0.as_secs();
        let hours = total_secs / 3600;
        let minutes = (total_secs / 60) % 60;
        let seconds = total_secs % 60;
        let millis = self.0.subsec_millis();
        write!(f, "{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
    }
}

struct Subtitle {
    image: Image,
    start: Duration,
    end: Duration,
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

    let mut subtitles: SlotMap<SubtitleId, Subtitle> = Default::default();
    let mut subtitles_text: SecondaryMap<SubtitleId, String> = Default::default();

    // extract subtitles
    for subtitle in extract_subtitles_from_pgs(&input_data)? {
        subtitles.insert(subtitle);
    }
    tracing::info!("extracted {} subtitles", subtitles.len());

    // subtitle ocr
    let (ocr_in_sender, ocr_in_receiver) = crossbeam::channel::unbounded::<SubtitleId>();
    let (ocr_out_sender, ocr_out_receiver) =
        crossbeam::channel::unbounded::<(SubtitleId, String)>();

    for subtitle_id in subtitles.keys() {
        ocr_in_sender.send(subtitle_id).unwrap();
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
                while let Ok(subtitle_id) = ocr_in_receiver.recv() {
                    let image = &subtitles[subtitle_id].image;
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
                    ocr_out_sender.send((subtitle_id, text)).unwrap();
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
    while let Ok((subtitle_id, text)) = ocr_out_receiver.recv() {
        subtitles_text.insert(subtitle_id, text);
    }
    tracing::info!("ocr finished");

    // produce srt
    let mut queue: VecDeque<SubtitleId> = Default::default();
    for subtitle_id in subtitles.keys() {
        queue.push_back(subtitle_id);
    }

    let mut srt = String::default();
    let mut current_text = String::default();
    let mut current_sub_num = 1;
    for (subtitle_id, subtitle) in subtitles.iter() {
        use std::fmt::Write;

        let subtitle_text = &subtitles_text[subtitle_id];
        let _ = writeln!(srt, "{current_sub_num}");
        let _ = writeln!(
            srt,
            "{} --> {}",
            DurationSrtDisplay(subtitle.start),
            DurationSrtDisplay(subtitle.end)
        );
        srt.push_str(&subtitle_text);
        srt.push_str("\n");
        current_sub_num += 1;
    }
    println!("{}", srt);

    return Ok(());

    // let mut window = minifb::Window::new(
    //     "sup2srt",
    //     1200,
    //     1200,
    //     minifb::WindowOptions {
    //         scale_mode: minifb::ScaleMode::Center,
    //         ..Default::default()
    //     },
    // )
    // .unwrap();
    // window.set_target_fps(60);
    //
    // let mut curr_image = 0usize;
    // while window.is_open() && !window.is_key_down(Key::Escape) {
    //     if window.is_key_pressed(Key::A, KeyRepeat::No) {
    //         curr_image = curr_image.saturating_sub(1);
    //     }
    //     if window.is_key_pressed(Key::D, KeyRepeat::No) {
    //         curr_image = subtitles.len().saturating_sub(1).min(curr_image + 1);
    //     }
    //
    //     let image = &subtitles[curr_image].image;
    //     let mut buffer = Vec::with_capacity(image.pixels.len() / 4);
    //     for i in 0..image.pixels.len() / 4 {
    //         let v = ((image.pixels[i * 4] as u32) << 16)
    //             | ((image.pixels[i * 4 + 1] as u32) << 8)
    //             | ((image.pixels[i * 4 + 2] as u32) << 0);
    //         buffer.push(v);
    //     }
    //     window
    //         .update_with_buffer(&buffer, image.width as usize, image.height as usize)
    //         .unwrap();
    // }

    Ok(())
}

fn image_from_object_and_palette(object: &Object, palette: &pgs::PDS) -> Result<Image> {
    let pixels_indexed = pgs::decode_rle_data(&object.data, object.width, object.height)
        .context("decoding ODS rle data")?;
    let mut pixels = Vec::with_capacity(pixels_indexed.len());
    for idx in pixels_indexed {
        let (r, g, b, a) = palette.entries[idx as usize].to_rgba();
        pixels.extend([r, g, b, a]);
    }
    Ok(Image {
        width: u32::from(object.width),
        height: u32::from(object.height),
        pixels,
    })
}

fn extract_subtitles_from_pgs(pgs: &[u8]) -> Result<Vec<Subtitle>> {
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
    let mut subtitles: Vec<Subtitle> = Vec::default();
    // index of images inserted in the previous display set
    // used to patch the end time
    let mut previous_subtitles: Vec<usize> = Vec::default();

    for ds in display_sets {
        assert_eq!(ds.pcs.width, display_width);
        assert_eq!(ds.pcs.height, display_height);

        let current_time = pgs::clock_to_duration(ds.pcs.header.pts);
        for subtitle_idx in previous_subtitles.drain(..) {
            subtitles[subtitle_idx].end = current_time;
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
                image: Default::default(),
            });

            match ods.last_in_sequence {
                pgs::LastInSequenceFlag::FirstAndLast => {
                    obj.finished = true;
                    obj.data.clear();
                    obj.data.extend(ods.data);
                    obj.image = image_from_object_and_palette(obj, palette)?;
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
                    obj.image = image_from_object_and_palette(obj, palette)?;
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

            let image = if let Some(cropping) = comp.cropping {
                let image = object.image.sub_image(
                    u32::from(cropping.horizontal_position),
                    u32::from(cropping.vertical_position),
                    u32::from(cropping.width),
                    u32::from(cropping.height),
                );
                image
            } else {
                object.image.clone()
            };

            previous_subtitles.push(subtitles.len());
            subtitles.push(Subtitle {
                image,
                start: current_time,
                end: Default::default(),
            });
        }
    }

    Ok(subtitles)
}
