//! Image files in the diff view: the picture before and the picture after
//! the change, side by side. Git shows most image files as binary, with no
//! text diff. An SVG file has a text diff too: a switch in the header shows
//! one or the other. The pictures load in the background when the file is
//! first shown.

use std::path::PathBuf;
use std::sync::Arc;

use super::diff::DiffCtx;
use super::*;

/// Larger files show no picture: GPUI decodes all of a file at once.
const MAX_BYTES: u64 = 20_000_000;
/// Larger pictures do not fit in a GPU texture.
const MAX_SIDE: i32 = 16_384;

/// The format of an image file, from the extension of its path.
pub(super) fn format(path: &str) -> Option<ImageFormat> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => ImageFormat::Png,
        "jpg" | "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::Webp,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        "ico" => ImageFormat::Ico,
        "pbm" | "pgm" | "ppm" | "pnm" => ImageFormat::Pnm,
        "svg" => ImageFormat::Svg,
        _ => return None,
    })
}

/// Whether `file` is an SVG file with a text diff, which can show as text
/// or as pictures.
pub(super) fn is_svg(file: &FileDiff) -> bool {
    !file.binary && format(&file.path) == Some(ImageFormat::Svg) && file.blob_ids() != (None, None)
}

/// The format to draw `file` in, when the diff view shows its pictures: a
/// binary image file, or an SVG file unless the settings ask for its text.
pub(super) fn picture(file: &FileDiff, cx: &App) -> Option<ImageFormat> {
    if is_svg(file) {
        return (!crate::settings::get(cx).svg_text).then_some(ImageFormat::Svg);
    }
    format(&file.path).filter(|_| file.binary && file.blob_ids() != (None, None))
}

/// Where one side of an image file comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Source {
    Blob(String),
    /// A file of a worktree, with the id that Git gives its content: when
    /// the file changes, its id changes too.
    File(PathBuf, String),
}

type Sources = (Option<Source>, Option<Source>);

/// One side of an image file, loaded.
enum Side {
    Picture {
        image: Arc<RenderImage>,
        bytes: u64,
        /// The size to show, in points: one point for each pixel of the
        /// file, or for each unit of an SVG.
        width: f32,
        height: f32,
    },
    /// Why the side shows no picture.
    Note(String),
}

/// The pictures of the shown image file. GPUI keeps a drawn picture in a
/// GPU texture until it is dropped, so a tab keeps only these.
#[derive(Default)]
pub(super) struct ImagePreview {
    sources: Option<Sources>,
    /// None while the sides load.
    sides: Option<(Option<Side>, Option<Side>)>,
    _task: Option<Task<()>>,
}

impl ImagePreview {
    /// Drop the pictures and free their textures.
    pub(super) fn clear(&mut self, cx: &mut App) {
        let images: Vec<_> = self
            .sides
            .take()
            .into_iter()
            .flat_map(|(old, new)| [old, new])
            .flatten()
            .filter_map(|side| match side {
                Side::Picture { image, .. } => Some(image),
                Side::Note(_) => None,
            })
            .collect();
        if !images.is_empty() {
            // `drop_image` skips a window while it draws, so wait for the end
            // of the frame.
            cx.defer(move |cx| {
                for image in images {
                    cx.drop_image(image, None);
                }
            });
        }
    }
}

impl GitApp {
    /// The body of the diff view for an image file.
    pub(super) fn render_image_diff(
        &mut self,
        file: &FileDiff,
        format: ImageFormat,
        ctx: DiffCtx,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sources = self.image_sources(file, ctx);
        if self.image.sources.as_ref() != Some(&sources) {
            self.load_image(sources, format, cx);
        }
        let Some((old, new)) = &self.image.sides else {
            return div().flex_1().into_any_element();
        };
        let both = old.is_some() && new.is_some();
        let red = cx.theme().colors.red;
        let green = cx.theme().colors.green;
        h_flex()
            .flex_1()
            .min_h_0()
            .p_4()
            .gap_4()
            .when_some(old.as_ref(), |d, side| {
                let label = both.then_some(("Before", red));
                d.child(side_view("image-old", label, side, cx))
            })
            .when_some(new.as_ref(), |d, side| {
                let label = both.then_some(("After", green));
                d.child(side_view("image-new", label, side, cx))
            })
            .into_any_element()
    }

    /// Where the sides of `file` come from. Git does not store the new side
    /// of a diff against a worktree: that side is the file on disk.
    fn image_sources(&self, file: &FileDiff, ctx: DiffCtx) -> Sources {
        let worktree = match self.view {
            View::Changes if ctx == DiffCtx::Unstaged => self.repo.as_ref().map(|r| r.root.clone()),
            View::Review => self.review.as_ref().and_then(|r| r.worktree.clone()),
            _ => None,
        };
        let (old, new) = file.blob_ids();
        let old = old.map(|id| Source::Blob(id.to_string()));
        let new = new.map(|id| match &worktree {
            Some(dir) => Source::File(dir.join(&file.path), id.to_string()),
            None => Source::Blob(id.to_string()),
        });
        (old, new)
    }

    fn load_image(&mut self, sources: Sources, format: ImageFormat, cx: &mut Context<Self>) {
        self.image.clear(cx);
        self.image.sources = Some(sources.clone());
        let Some(repo) = self.repo.clone() else {
            return;
        };
        let svg = cx.svg_renderer();
        let (old, new) = sources.clone();
        // Dropping the task of the file shown before stops its work.
        self.image._task = Some(cx.spawn(async move |this, cx| {
            let sides = cx
                .background_executor()
                .spawn(async move {
                    let load = |s: Option<Source>| s.map(|s| load_side(&repo, &s, format, &svg));
                    (load(old), load(new))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.image.sources.as_ref() == Some(&sources) {
                    this.image.sides = Some(sides);
                    cx.notify();
                }
            });
        }));
    }
}

fn load_side(repo: &Repo, source: &Source, format: ImageFormat, svg: &SvgRenderer) -> Side {
    read_side(repo, source, format, svg)
        .unwrap_or_else(|e| Side::Note(format!("Could not show the image: {e}")))
}

fn read_side(
    repo: &Repo,
    source: &Source,
    format: ImageFormat,
    svg: &SvgRenderer,
) -> anyhow::Result<Side> {
    let size = match source {
        Source::Blob(id) => git::blob_size(repo, id)?,
        Source::File(path, _) => std::fs::metadata(path)?.len(),
    };
    if size > MAX_BYTES {
        let text = format!("The file is too large to show ({}).", byte_size(size));
        return Ok(Side::Note(text));
    }
    let bytes = match source {
        Source::Blob(id) => git::blob(repo, id)?,
        Source::File(path, _) => std::fs::read(path)?,
    };
    let size = bytes.len() as u64;
    let image = Image::from_bytes(format, bytes).to_image_data(svg.clone())?;
    let pixels = image.size(0);
    if pixels.width.0 > MAX_SIDE || pixels.height.0 > MAX_SIDE {
        let text = format!(
            "The image is too large to show ({} × {} pixels).",
            pixels.width.0, pixels.height.0
        );
        return Ok(Side::Note(text));
    }
    // GPUI draws an SVG at twice its size, so that it is sharp on a Retina
    // display.
    let scale = match format {
        ImageFormat::Svg => SMOOTH_SVG_SCALE_FACTOR,
        _ => 1.,
    };
    Ok(Side::Picture {
        image,
        bytes: size,
        width: pixels.width.0 as f32 / scale,
        height: pixels.height.0 as f32 / scale,
    })
}

/// One side: the picture on a checkerboard, scaled down to fit, and below
/// it the label, the size in pixels (or SVG units) and the size of the file.
fn side_view(
    id: &'static str,
    label: Option<(&'static str, Hsla)>,
    side: &Side,
    cx: &App,
) -> impl IntoElement {
    let t = cx.theme();
    let muted = t.colors.muted_foreground;
    let checks = match side {
        Side::Picture { width, height, .. } => Some(checkerboard(*width, *height, t.is_dark())),
        Side::Note(_) => None,
    };
    let (body, facts) = match side {
        Side::Picture {
            image,
            bytes,
            width,
            height,
        } => (
            // At most its own size, and smaller to fit: Contain keeps the
            // shape when the box gets narrower or lower.
            img(image.clone())
                .id(id)
                .w(px(*width))
                .h(px(*height))
                .max_w_full()
                .max_h_full()
                .object_fit(ObjectFit::Contain)
                .into_any_element(),
            Some(format!(
                "{} × {} · {}",
                width.round(),
                height.round(),
                byte_size(*bytes)
            )),
        ),
        Side::Note(text) => (
            div()
                .text_color(muted)
                .child(text.clone())
                .into_any_element(),
            None,
        ),
    };
    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .gap_2()
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .rounded(t.radius)
                .border_1()
                .border_color(t.colors.border)
                .bg(t.colors.muted.opacity(0.35))
                .child(
                    div().absolute().inset_0().p_3().child(
                        div()
                            .relative()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(checks)
                            .child(body),
                    ),
                ),
        )
        .child(
            h_flex()
                .flex_none()
                .justify_center()
                .gap_1p5()
                .text_size(px(12.))
                .text_color(muted)
                .when_some(label, |d, (text, color)| {
                    d.child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(color)
                            .child(text),
                    )
                })
                .when_some(facts, |d, facts| d.child(facts)),
        )
}

/// The checkerboard behind a picture of `width` × `height` points, which
/// shows its transparent parts. Mid grays in the dark theme, so that black
/// and white lines both show on it.
fn checkerboard(width: f32, height: f32, dark: bool) -> impl IntoElement {
    let (l1, l2) = if dark { (0.44, 0.35) } else { (1., 0.86) };
    let (c1, c2) = (hsla(0., 0., l1, 1.), hsla(0., 0., l2, 1.));
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let Some(area) = fitted(bounds, width, height) else {
                return;
            };
            const CELL: f32 = 8.;
            window.paint_quad(fill(area, c1));
            let cols = (area.size.width / px(CELL)).ceil() as usize;
            let rows = (area.size.height / px(CELL)).ceil() as usize;
            for row in 0..rows {
                for col in (row % 2..cols).step_by(2) {
                    let at = area.origin + point(px(col as f32 * CELL), px(row as f32 * CELL));
                    let cell = Bounds::new(at, size(px(CELL), px(CELL))).intersect(&area);
                    window.paint_quad(fill(cell, c2));
                }
            }
        },
    )
    .absolute()
    .inset_0()
}

/// Where the picture of `width` × `height` points shows in `bounds`, as
/// `side_view` lays it out: centered, at most its own size, and smaller to
/// fit. None for a picture with no area.
fn fitted(bounds: Bounds<Pixels>, width: f32, height: f32) -> Option<Bounds<Pixels>> {
    if width <= 0. || height <= 0. {
        return None;
    }
    let scale = (bounds.size.width / px(width))
        .min(bounds.size.height / px(height))
        .min(1.);
    let shown = size(px(width * scale), px(height * scale));
    Some(Bounds::centered_at(bounds.center(), shown))
}

/// A file size as Finder shows it, where 1 KB is 1,000 bytes.
fn byte_size(n: u64) -> String {
    match n {
        0..1_000 => format!("{n} byte{}", history::plural(n as usize)),
        1_000..1_000_000 => format!("{:.1} KB", n as f64 / 1e3),
        _ => format!("{:.1} MB", n as f64 / 1e6),
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in GPUI's `test` macro.
    use super::{byte_size, format};
    use gpui_kit::ImageFormat;

    #[test]
    fn formats_by_extension() {
        assert_eq!(format("assets/icon.PNG"), Some(ImageFormat::Png));
        assert_eq!(format("a/b.jpeg"), Some(ImageFormat::Jpeg));
        assert_eq!(format("v1.2/README"), None);
        assert_eq!(format("logo.svg"), Some(ImageFormat::Svg));
    }

    #[test]
    fn byte_sizes() {
        assert_eq!(byte_size(1), "1 byte");
        assert_eq!(byte_size(999), "999 bytes");
        assert_eq!(byte_size(12_345), "12.3 KB");
        assert_eq!(byte_size(4_200_000), "4.2 MB");
    }
}
