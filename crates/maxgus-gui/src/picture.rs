//! The picture a buffer stands in for, fitted into its window.
//!
//! The buffer holds a caption on its first line; the picture goes in the
//! rows under it, at its own size where the window has room for that and
//! shrunk to fit — never stretched — where it does not, and centred either
//! way.

use maxgus_core::Editor;
use maxgus_core::picture::Pixels;
use maxgus_tui::Rect;

/// Where one window's picture goes, in pixels from the grid's corner,
/// and how big.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    pub buffer: maxgus_text::BufferId,
    pub position: [f32; 2],
    pub size: [f32; 2],
}

/// Every picture on screen: one per window showing a buffer that has one,
/// fitted under the caption. A window too short for anything but the
/// caption shows only the caption.
pub fn placed(editor: &Editor, cell: (f32, f32), scale: f32) -> Vec<Placed> {
    editor
        .windows
        .iter()
        .filter_map(|window| {
            let pixels = &editor.pictures.get(&window.buffer)?.pixels;
            let area = maxgus_core::text_area(editor, window.id)?;
            let (position, size) = fit(pixels, under_caption(area)?, cell, scale)?;
            Some(Placed {
                buffer: window.buffer,
                position,
                size,
            })
        })
        .collect()
}

/// The rows of `area` below its first, which the caption has.
fn under_caption(area: Rect) -> Option<Rect> {
    (area.height > 1).then(|| Rect::new(area.x, area.y + 1, area.width, area.height - 1))
}

/// Fits the picture into `area`, in pixels: at its own size, scaled by the
/// display's, when that fits; shrunk to the largest size that does when it
/// does not; and centred either way.
fn fit(pixels: &Pixels, area: Rect, cell: (f32, f32), scale: f32) -> Option<([f32; 2], [f32; 2])> {
    if pixels.width == 0 || pixels.height == 0 {
        return None;
    }
    let (left, top) = (area.x as f32 * cell.0, area.y as f32 * cell.1);
    let (room_w, room_h) = (area.width as f32 * cell.0, area.height as f32 * cell.1);
    let (natural_w, natural_h) = (pixels.width as f32 * scale, pixels.height as f32 * scale);
    let shrink = (room_w / natural_w).min(room_h / natural_h).min(1.0);
    let width = (natural_w * shrink).floor().max(1.0);
    let height = (natural_h * shrink).floor().max(1.0);
    let x = (left + (room_w - width) / 2.0).floor();
    let y = (top + (room_h - height) / 2.0).floor();
    Some(([x, y], [width, height]))
}

/// The picture's pixels at `size`, for the texture: the same pixels when
/// that is their size, and a resampling when it is not — done here, once,
/// because a GPU sampling a big picture down to a small one drops most of
/// it and the result shimmers.
pub fn resampled(pixels: &Pixels, size: (u32, u32)) -> std::borrow::Cow<'_, [u8]> {
    if size == (pixels.width, pixels.height) {
        return std::borrow::Cow::Borrowed(&pixels.rgba);
    }
    let Some(source) =
        image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba.to_vec())
    else {
        return std::borrow::Cow::Borrowed(&pixels.rgba);
    };
    let scaled = image::imageops::resize(
        &source,
        size.0.max(1),
        size.1.max(1),
        image::imageops::FilterType::Triangle,
    );
    std::borrow::Cow::Owned(scaled.into_raw())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn pixels(width: u32, height: u32) -> Pixels {
        Pixels {
            width,
            height,
            rgba: Arc::from(vec![200u8; (width * height * 4) as usize]),
        }
    }

    #[test]
    fn a_small_picture_is_drawn_at_its_own_size_in_the_middle() {
        // Twenty columns of ten pixels, ten rows of twenty: 200 by 200.
        let area = Rect::new(0, 1, 20, 10);
        let (position, size) = fit(&pixels(100, 50), area, (10.0, 20.0), 1.0).unwrap();
        assert_eq!(size, [100.0, 50.0]);
        assert_eq!(position, [50.0, 20.0 + 75.0]);
        // At a 2× display it is twice the size, which is still its own.
        let (_, size) = fit(&pixels(100, 50), area, (20.0, 40.0), 2.0).unwrap();
        assert_eq!(size, [200.0, 100.0]);
    }

    #[test]
    fn a_big_picture_is_shrunk_to_fit_and_keeps_its_shape() {
        let area = Rect::new(0, 1, 20, 10);
        let (position, size) = fit(&pixels(1000, 250), area, (10.0, 20.0), 1.0).unwrap();
        assert_eq!(size, [200.0, 50.0], "the width is what limits it");
        assert_eq!(position, [0.0, 20.0 + 75.0]);
        let (_, size) = fit(&pixels(100, 1000), area, (10.0, 20.0), 1.0).unwrap();
        assert_eq!(size, [20.0, 200.0], "and here the height");
    }

    #[test]
    fn the_caption_row_is_kept_and_a_window_of_one_row_shows_only_it() {
        assert_eq!(
            under_caption(Rect::new(3, 2, 10, 5)),
            Some(Rect::new(3, 3, 10, 4))
        );
        assert_eq!(under_caption(Rect::new(0, 0, 10, 1)), None);
    }

    #[test]
    fn the_pixels_are_resampled_only_when_the_size_differs() {
        let source = pixels(4, 4);
        assert!(matches!(
            resampled(&source, (4, 4)),
            std::borrow::Cow::Borrowed(_)
        ));
        let half = resampled(&source, (2, 2));
        assert_eq!(half.len(), 2 * 2 * 4);
        assert_eq!(&half[..4], &[200, 200, 200, 200]);
    }
}
