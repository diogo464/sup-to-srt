use std::{
    collections::HashMap,
    io::{Cursor, Write},
    process::Command,
};

use minifb::{Key, KeyRepeat};
use pgs::wire::PALETTE_UPDATE_FLAG_TRUE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Color {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

struct Palette {
    colors: Vec<Color>,
}

impl Palette {
    pub fn get_color(&self, id: u32) -> Option<Color> {
        self.colors.get(id as usize).copied()
    }
}

struct Image {
    data: Vec<u32>,
    width: usize,
    height: usize,
}

fn main() {
    let data = std::fs::read("output.sup").unwrap();
    let cursor = Cursor::new(&data);
    let display_sets = pgs::decode_display_sets(cursor).unwrap();
    //println!("num display sets = {}", display_sets.len());

    assert!(display_sets.len() > 0);
    let display_set_0 = &display_sets[0];
    assert_eq!(
        display_set_0.pcs.composition_state,
        pgs::CompositionState::EpochStart
    );

    let display_width = display_set_0.pcs.width;
    let display_height = display_set_0.pcs.height;
    let mut current_epoch = 0;
    let mut objects: HashMap<u16, pgs::ODS> = Default::default();
    let mut windows: HashMap<u8, pgs::Window> = Default::default();
    let mut palettes: HashMap<u8, pgs::PDS> = Default::default();

    let mut tesseract = tesseract::Tesseract::new(None, Some("eng")).unwrap();
    let mut frame: Vec<u8> = Vec::default();
    let mut frame_rgb: Vec<u32> = Default::default();
    let mut frame_width = 0;
    let mut frame_height = 0;
    let mut images: Vec<Image> = Default::default();
    let mut palette_shown = false;

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

    for ds in display_sets {
        assert_eq!(ds.pcs.width, display_width);
        assert_eq!(ds.pcs.height, display_height);
        // println!(
        //     "time = {}",
        //     pgs::clock_to_duration(ds.pcs.header.pts).as_secs_f64()
        // );
        match ds.pcs.composition_state {
            pgs::CompositionState::EpochStart => {
                current_epoch += 1;
                objects.clear();
                windows.clear();
                palettes.clear();
                //println!("transition to epoch {current_epoch}");
            }
            pgs::CompositionState::Normal => {}
            pgs::CompositionState::AcquisitionPoint => {}
        }

        for composition in ds.pcs.composition_objects {
            // println!(
            //     "composition obj={} wnd={}",
            //     composition.object_id, composition.window_id
            // );
        }

        for pds in ds.pds {
            //println!("palette id = {}", pds.palette_id);
            // if !palette_shown {
            //     palette_shown = true;
            //     for entry in &pds.entries {
            //         println!("#{} {:?}", entry.entry_id, entry.to_rgba());
            //     }
            // }
            palettes.insert(pds.palette_id, pds);
        }

        for wds in ds.wds {
            for window in wds.windows {
                // println!(
                //     "window id = {} {}x{}",
                //     window.window_id, window.width, window.height
                // );
                windows.insert(window.window_id, window);
            }
        }

        for object in ds.ods {
            let palette = palettes.get_mut(&0).unwrap();
            frame.clear();
            frame_rgb.clear();
            frame_width = object.width as u32;
            frame_height = object.height as u32;
            for &pixel in object.pixels.iter() {
                let (mut r, mut g, mut b, a) = match pixel {
                    pgs::ColorCode::Transparent => {
                        //println!("0");
                        //(0, 0, 0, 0)
                        let entry = &palette.entries[0];
                        entry.to_rgba()
                    }
                    pgs::ColorCode::Color(idx) => {
                        //println!("{idx}");
                        let entry = &palette.entries[idx as usize];
                        entry.to_rgba()
                    }
                };
                if a == 0 {
                    r = 0;
                    g = 0;
                    b = 0;
                }
                frame.push(r);
                frame.push(g);
                frame.push(b);
                frame.push(a);
                frame_rgb.push((r as u32) << 16 | (g as u32) << 8 | (b as u32));
            }
            images.push(Image {
                data: frame_rgb.clone(),
                width: frame_width as usize,
                height: frame_height as usize,
            });
            tesseract = tesseract
                .set_frame(
                    &frame,
                    object.width as i32,
                    object.height as i32,
                    4,
                    4 * object.width as i32,
                )
                .unwrap();
            tesseract = tesseract.recognize().unwrap();
            let text = tesseract.get_text().unwrap();
            println!("{text}");

            // use std::fmt::Write;
            //
            // let mut ppm = String::default();
            // //println!("object id = {}", object.object_id);
            // writeln!(ppm, "P3\n{} {}\n255", object.width, object.height);
            // let palette = &palettes[&0];
            // let mut i = 0usize;
            // for &pixel in object.pixels.iter() {
            //     let entry = &palette.entries[pixel as usize];
            //     let (r, g, b, _a) = entry.to_rgba();
            //     write!(ppm, "{r} {g} {b}");
            //     if i > 0 && i % object.width as usize == 0 {
            //         writeln!(ppm, "");
            //     } else {
            //         write!(ppm, " ");
            //     }
            //     i += 1;
            // }
            //
            // let text = recognize(&ppm);
            // println!("{text}\n\n");

            objects.insert(object.object_id, object);
        }
    }

    // let mut curr_image = 0usize;
    // while window.is_open() && !window.is_key_down(Key::Escape) {
    //     if window.is_key_pressed(Key::A, KeyRepeat::No) {
    //         curr_image = curr_image.saturating_sub(1);
    //     }
    //     if window.is_key_pressed(Key::D, KeyRepeat::No) {
    //         curr_image = images.len().saturating_sub(1).min(curr_image + 1);
    //     }
    //
    //     let image = &images[curr_image];
    //     window
    //         .update_with_buffer(&image.data, image.width, image.height)
    //         .unwrap();
    // }
}

fn recognize(ppm: &str) -> String {
    let mut child = Command::new("tesseract")
        .arg("-")
        .arg("-")
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.as_mut().unwrap();
    stdin.write_all(ppm.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    String::from_utf8(output.stdout).unwrap()
}
