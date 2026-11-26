use std::{
    fs::{self, File},
    io::BufWriter,
    path::Path,
    sync::Arc,
};

use anyhow::Result;
use gpui_kit::{MouseButton, PlatformTextSystem, base::ThemeAppearance, px, size};
use image::{RgbaImage, imageops::FilterType};
use png::{BitDepth, BlendOp, ColorType, Compression, DisposeOp, Encoder, Filter};

use crate::session::{CaptureSession, Mode};

pub fn record_demo(
    dataset: &Path,
    text_system: Arc<dyn PlatformTextSystem>,
    output_dir: &Path,
    target_width: u32,
) -> Result<()> {
    fs::create_dir_all(output_dir)?;

    for (name, appearance) in [
        ("demo-light.png", ThemeAppearance::Light),
        ("demo-dark.png", ThemeAppearance::Dark),
    ] {
        let mut s = CaptureSession::open(
            dataset,
            Some(size(px(1024.), px(640.))),
            text_system.clone(),
            output_dir.to_path_buf(),
            Mode::Standard,
        )?;
        s.set_theme(appearance)?;

        let mut frames: Vec<(RgbaImage, u16)> = Vec::new();
        let mut push = |s: &mut CaptureSession, delay_ms: u16| -> Result<()> {
            let img = s.snapshot()?;
            let scaled = if img.width() == target_width {
                img
            } else {
                let target_height = ((u64::from(img.height()) * u64::from(target_width))
                    / u64::from(img.width()))
                .max(1) as u32;
                image::imageops::resize(&img, target_width, target_height, FilterType::Lanczos3)
            };
            frames.push((scaled, delay_ms));
            Ok(())
        };

        push(&mut s, 850)?;

        s.click_cell("Station", 1, MouseButton::Left)?;
        push(&mut s, 450)?;

        s.key("cmd-f")?;
        push(&mut s, 250)?;

        let query: Vec<char> = "survey".chars().collect();
        for (i, ch) in query.iter().enumerate() {
            s.input(&ch.to_string())?;
            let delay = if i + 1 == query.len() { 800 } else { 120 };
            push(&mut s, delay)?;
        }

        s.key("escape")?;
        push(&mut s, 350)?;

        s.click_cell("Water temperature", 0, MouseButton::Left)?;
        push(&mut s, 350)?;

        s.action("analyze")?;
        s.action("chart")?;
        push(&mut s, 550)?;

        s.click_label("Log Y", None)?;
        push(&mut s, 450)?;

        let point = s.scatter_point(80, 96, 16.4, 10.2, 19.2, true)?;
        s.click_point(point, MouseButton::Left)?;
        push(&mut s, 800)?;

        s.action("filter")?;
        if s.find_by(|el| el.label() == Some("Display options"), None)?
            .is_none()
        {
            s.click_label("Range", None)?;
        }
        push(&mut s, 550)?;

        let start = s
            .require_by(
                "[12.7,",
                |el| el.label().is_some_and(|l| l.starts_with("[12.7,")),
                None,
            )?
            .bounds()
            .center();
        let finish = s
            .require_by(
                "15.6) · 4 rows",
                |el| el.label().is_some_and(|l| l.ends_with("15.6) · 4 rows")),
                None,
            )?
            .bounds()
            .center();
        s.drag(start, finish)?;
        push(&mut s, 650)?;

        s.click_label("Apply", None)?;
        push(&mut s, 1200)?;

        let path = output_dir.join(name);
        let (bytes, unique_frames, total_ms) = write_apng(&path, &frames)?;
        let (w, h) = (frames[0].0.width(), frames[0].0.height());
        println!(
            "{}\t{}x{}\t{} frames\t{:.2}s\t{:.1} KB",
            path.display(),
            w,
            h,
            unique_frames,
            f64::from(total_ms) / 1000.0,
            bytes as f64 / 1024.0
        );
    }
    Ok(())
}

fn write_apng(path: &Path, raw_frames: &[(RgbaImage, u16)]) -> Result<(u64, usize, u32)> {
    let mut frames: Vec<(&RgbaImage, u16)> = Vec::with_capacity(raw_frames.len());
    for (img, delay) in raw_frames {
        if let Some((prev, prev_delay)) = frames.last_mut()
            && prev.as_raw() == img.as_raw()
        {
            *prev_delay = prev_delay.saturating_add(*delay);
        } else {
            frames.push((img, *delay));
        }
    }

    let (width, height) = (frames[0].0.width(), frames[0].0.height());
    let file = BufWriter::new(File::create(path)?);
    let mut encoder = Encoder::new(file, width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_compression(Compression::High);
    encoder.set_filter(Filter::Adaptive);
    encoder.set_animated(frames.len() as u32, 0)?;
    encoder.set_frame_delay(frames[0].1, 1000)?;
    encoder.set_dispose_op(DisposeOp::None)?;
    encoder.set_blend_op(BlendOp::Source)?;

    let mut writer = encoder.write_header()?;
    writer.write_image_data(frames[0].0.as_raw())?;

    let total_ms = frames.iter().map(|(_, d)| u32::from(*d)).sum();
    for i in 1..frames.len() {
        let prev = frames[i - 1].0;
        let (curr, delay) = frames[i];
        let ((x0, y0, dw, dh), transparent_diff_safe) = dirty_rect(prev, curr);
        let buf = crop_frame(prev, curr, x0, y0, dw, dh, transparent_diff_safe);
        let blend = if transparent_diff_safe {
            BlendOp::Over
        } else {
            BlendOp::Source
        };

        writer.reset_frame_position()?;
        writer.set_frame_dimension(dw, dh)?;
        writer.set_frame_position(x0, y0)?;
        writer.set_frame_delay(delay, 1000)?;
        writer.set_dispose_op(DisposeOp::None)?;
        writer.set_blend_op(blend)?;
        writer.write_image_data(&buf)?;
    }
    writer.finish()?;

    let size_bytes = fs::metadata(path)?.len();
    Ok((size_bytes, frames.len(), total_ms))
}

fn dirty_rect(prev: &RgbaImage, curr: &RgbaImage) -> ((u32, u32, u32, u32), bool) {
    let (w, h) = (curr.width(), curr.height());
    let prev_raw = prev.as_raw();
    let curr_raw = curr.as_raw();

    let mut min_x = w;
    let mut min_y = h;
    let mut max_x = 0;
    let mut max_y = 0;
    let mut opaque_changes = true;

    for y in 0..h {
        let row_start = (y * w * 4) as usize;
        for x in 0..w {
            let idx = row_start + (x * 4) as usize;
            let p = &prev_raw[idx..idx + 4];
            let c = &curr_raw[idx..idx + 4];
            if p != c {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                if c[3] != 255 {
                    opaque_changes = false;
                }
            }
        }
    }

    if min_x > max_x {
        return ((0, 0, 1, 1), false);
    }
    (
        (min_x, min_y, max_x - min_x + 1, max_y - min_y + 1),
        opaque_changes,
    )
}

fn crop_frame(
    prev: &RgbaImage,
    curr: &RgbaImage,
    x0: u32,
    y0: u32,
    dw: u32,
    dh: u32,
    transparent_diff: bool,
) -> Vec<u8> {
    let w = curr.width();
    let prev_raw = prev.as_raw();
    let curr_raw = curr.as_raw();
    let mut out = vec![0u8; (dw * dh * 4) as usize];

    for dy in 0..dh {
        let y = y0 + dy;
        for dx in 0..dw {
            let x = x0 + dx;
            let src = ((y * w + x) * 4) as usize;
            let dst = ((dy * dw + dx) * 4) as usize;
            let p = &prev_raw[src..src + 4];
            let c = &curr_raw[src..src + 4];
            if !transparent_diff || p != c {
                out[dst..dst + 4].copy_from_slice(c);
            }
        }
    }
    out
}
