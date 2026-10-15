//! Playback control icons drawn as images, so terminals with the Kitty graphics protocol can show
//! them bigger than a single text cell (terminals have no way to enlarge individual characters).

use std::f32::consts::{FRAC_PI_2, PI};

use image::{Rgba, RgbaImage};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Icon {
    Shuffle,
    Previous,
    Play,
    Pause,
    Next,
    Repeat,
    RepeatOne,
}

/// A shape in a unit square (x right, y down, both 0..1).
enum Shape {
    Rect {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    },
    Polygon(Vec<(f32, f32)>),
    /// A line segment `width` thick.
    Line {
        a: (f32, f32),
        b: (f32, f32),
        width: f32,
    },
    /// A circular arc `width` thick, between two angles (radians, measured clockwise from +x since
    /// y points down).
    Arc {
        cx: f32,
        cy: f32,
        r: f32,
        width: f32,
        from: f32,
        to: f32,
    },
}

impl Shape {
    fn contains(&self, x: f32, y: f32) -> bool {
        match *self {
            Self::Rect { x0, y0, x1, y1 } => (x0..=x1).contains(&x) && (y0..=y1).contains(&y),
            Self::Polygon(ref points) => {
                // Even-odd ray casting.
                let mut inside = false;
                let mut j = points.len() - 1;
                for i in 0..points.len() {
                    let (xi, yi) = points[i];
                    let (xj, yj) = points[j];
                    if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                        inside = !inside;
                    }
                    j = i;
                }
                inside
            }
            Self::Line { a, b, width } => {
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let length_sq = dx * dx + dy * dy;
                let t = if length_sq > 0.0 {
                    (((x - a.0) * dx + (y - a.1) * dy) / length_sq).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (px, py) = (a.0 + t * dx, a.1 + t * dy);
                (x - px).powi(2) + (y - py).powi(2) <= (width / 2.0).powi(2)
            }
            Self::Arc {
                cx,
                cy,
                r,
                width,
                from,
                to,
            } => {
                let distance = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                let angle = (y - cy).atan2(x - cx);
                (distance - r).abs() <= width / 2.0 && from <= angle && angle <= to
            }
        }
    }
}

const STROKE: f32 = 0.08;

/// Shapes making up `icon`, and shapes cut out of them.
fn shapes(icon: Icon) -> (Vec<Shape>, Vec<Shape>) {
    use Shape::{Arc, Line, Polygon, Rect};
    match icon {
        Icon::Play => (
            vec![Polygon(vec![(0.27, 0.20), (0.27, 0.80), (0.80, 0.50)])],
            vec![],
        ),
        Icon::Pause => (
            vec![
                Rect {
                    x0: 0.28,
                    y0: 0.22,
                    x1: 0.42,
                    y1: 0.78,
                },
                Rect {
                    x0: 0.58,
                    y0: 0.22,
                    x1: 0.72,
                    y1: 0.78,
                },
            ],
            vec![],
        ),
        Icon::Previous => (
            vec![
                Rect {
                    x0: 0.22,
                    y0: 0.27,
                    x1: 0.30,
                    y1: 0.73,
                },
                Polygon(vec![(0.78, 0.27), (0.78, 0.73), (0.33, 0.50)]),
            ],
            vec![],
        ),
        Icon::Next => (
            vec![
                Rect {
                    x0: 0.70,
                    y0: 0.27,
                    x1: 0.78,
                    y1: 0.73,
                },
                Polygon(vec![(0.22, 0.27), (0.22, 0.73), (0.67, 0.50)]),
            ],
            vec![],
        ),
        Icon::Shuffle => {
            let line = |a, b| Line {
                a,
                b,
                width: STROKE,
            };
            (
                vec![
                    // Two paths crossing in the middle, each ending in an arrow on the right.
                    line((0.12, 0.30), (0.34, 0.30)),
                    line((0.34, 0.30), (0.62, 0.70)),
                    line((0.62, 0.70), (0.78, 0.70)),
                    Polygon(vec![(0.76, 0.60), (0.90, 0.70), (0.76, 0.80)]),
                    line((0.12, 0.70), (0.34, 0.70)),
                    line((0.34, 0.70), (0.62, 0.30)),
                    line((0.62, 0.30), (0.78, 0.30)),
                    Polygon(vec![(0.76, 0.20), (0.90, 0.30), (0.76, 0.40)]),
                ],
                vec![],
            )
        }
        Icon::Repeat | Icon::RepeatOne => {
            let line = |a, b| Line {
                a,
                b,
                width: STROKE,
            };
            let arc = |cx, cy, from, to| Arc {
                cx,
                cy,
                r: 0.14,
                width: STROKE,
                from,
                to,
            };
            // A rounded loop with an arrow at the top right.
            let mut add = vec![
                line((0.29, 0.28), (0.62, 0.28)),
                Polygon(vec![(0.60, 0.17), (0.76, 0.28), (0.60, 0.39)]),
                arc(0.29, 0.42, -PI, -FRAC_PI_2),
                line((0.15, 0.42), (0.15, 0.58)),
                arc(0.29, 0.58, FRAC_PI_2, PI),
                line((0.29, 0.72), (0.71, 0.72)),
                arc(0.71, 0.58, 0.0, FRAC_PI_2),
                line((0.85, 0.58), (0.85, 0.46)),
            ];
            if icon == Icon::RepeatOne {
                add.push(Rect {
                    x0: 0.47,
                    y0: 0.40,
                    x1: 0.54,
                    y1: 0.62,
                });
                add.push(Line {
                    a: (0.42, 0.46),
                    b: (0.50, 0.41),
                    width: 0.06,
                });
            }
            (add, vec![])
        }
    }
}

/// Render `icon` in `color` into a `width`×`height` image with a transparent background. The
/// icon is drawn in a centered square, anti-aliased by sampling each pixel 4×4 times.
pub fn render(icon: Icon, width: u32, height: u32, color: [u8; 3]) -> RgbaImage {
    const SAMPLES: u32 = 4;
    let (add, cut) = shapes(icon);
    let side = width.min(height).max(1) as f32;
    let (left, top) = ((width as f32 - side) / 2.0, (height as f32 - side) / 2.0);

    RgbaImage::from_fn(width, height, |px, py| {
        let mut covered = 0;
        for sy in 0..SAMPLES {
            for sx in 0..SAMPLES {
                let x = (px as f32 + (sx as f32 + 0.5) / SAMPLES as f32 - left) / side;
                let y = (py as f32 + (sy as f32 + 0.5) / SAMPLES as f32 - top) / side;
                if add.iter().any(|s| s.contains(x, y)) && !cut.iter().any(|s| s.contains(x, y)) {
                    covered += 1;
                }
            }
        }
        let alpha = (covered * 255 / (SAMPLES * SAMPLES)) as u8;
        Rgba([color[0], color[1], color[2], alpha])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(image: &RgbaImage, x: f32, y: f32) -> u8 {
        let px = ((x * image.width() as f32) as u32).min(image.width() - 1);
        let py = ((y * image.height() as f32) as u32).min(image.height() - 1);
        image.get_pixel(px, py).0[3]
    }

    #[test]
    fn test_play_is_a_solid_triangle() {
        let image = render(Icon::Play, 64, 64, [255, 255, 255]);
        assert_eq!(alpha_at(&image, 0.45, 0.5), 255, "inside the triangle");
        assert_eq!(alpha_at(&image, 0.9, 0.2), 0, "outside the triangle");
    }

    #[test]
    fn test_icons_are_centered_in_wide_images() {
        // 2:1 image: the icon is drawn in the middle square only.
        let image = render(Icon::Pause, 128, 64, [255, 255, 255]);
        // Left bar sits at 0.28-0.42 of the middle square, i.e. 0.39-0.46 of the full width.
        assert_eq!(alpha_at(&image, 0.42, 0.5), 255);
        assert_eq!(alpha_at(&image, 0.1, 0.5), 0);
    }

    #[test]
    fn test_every_icon_draws_something() {
        for icon in [
            Icon::Shuffle,
            Icon::Previous,
            Icon::Play,
            Icon::Pause,
            Icon::Next,
            Icon::Repeat,
            Icon::RepeatOne,
        ] {
            let image = render(icon, 40, 40, [1, 2, 3]);
            assert!(image.pixels().any(|p| p.0[3] > 0), "{icon:?} is empty");
            assert!(image.pixels().all(|p| p.0[..3] == [1, 2, 3]));
        }
    }
}
