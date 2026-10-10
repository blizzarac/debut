//! Shape rasterization (GFX-03): a shape's outline as polygons, filled and
//! stroked with analytic anti-aliasing into straight sRGB RGBA8.
//!
//! Coverage comes from the signed distance to the outline: inside by the
//! even-odd rule, distance to the nearest edge. Each row only looks at the
//! edges that come near it, so large shapes stay cheap.

use crate::text::Raster;
use debut_project::{Fill, Shape, ShapeKind};
use std::f32::consts::{PI, TAU};

type Pt = (f32, f32);

/// Outline of `shape` inside its `width` x `height` box: one or more closed
/// polygons, or for a line one open polyline (`closed` false).
fn outline(shape: &Shape) -> (Vec<Pt>, bool) {
    let (w, h) = (shape.width.max(1.0), shape.height.max(1.0));
    let (cx, cy) = (w / 2.0, h / 2.0);
    // Enough segments that the chord error stays under ~0.02 px.
    let segments = |r: f32, arc: f32| -> usize {
        let step = 2.0 * (1.0 - 0.02 / r.max(0.04)).clamp(-1.0, 1.0).acos();
        ((arc / step.max(1e-3)).ceil() as usize).clamp(4, 512)
    };
    match shape.kind {
        ShapeKind::Rectangle { corner_px } => {
            let r = corner_px.clamp(0.0, w.min(h) / 2.0);
            if r < 0.01 {
                return (vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)], true);
            }
            let n = segments(r, PI / 2.0);
            let mut pts = Vec::with_capacity(4 * (n + 1));
            // Corner centres clockwise from top-left, each arc a quarter turn.
            for (k, (ox, oy)) in [(r, r), (w - r, r), (w - r, h - r), (r, h - r)]
                .into_iter()
                .enumerate()
            {
                let start = PI + k as f32 * PI / 2.0;
                for i in 0..=n {
                    let a = start + PI / 2.0 * i as f32 / n as f32;
                    pts.push((ox + r * a.cos(), oy + r * a.sin()));
                }
            }
            (pts, true)
        }
        ShapeKind::Ellipse => {
            let n = segments(cx.max(cy), TAU);
            let pts = (0..n)
                .map(|i| {
                    let a = TAU * i as f32 / n as f32;
                    (cx + cx * a.cos(), cy + cy * a.sin())
                })
                .collect();
            (pts, true)
        }
        ShapeKind::Polygon { sides } => {
            let n = sides.clamp(3, 64) as usize;
            (fit((0..n).map(|i| polar(1.0, i, n)).collect(), w, h), true)
        }
        ShapeKind::Star { points, inner } => {
            let n = points.clamp(2, 64) as usize * 2;
            let inner = inner.clamp(0.05, 1.0);
            let pts = (0..n)
                .map(|i| polar(if i % 2 == 0 { 1.0 } else { inner }, i, n))
                .collect();
            (fit(pts, w, h), true)
        }
        ShapeKind::Arrow { head, shaft } => {
            let neck = w * (1.0 - head.clamp(0.05, 1.0));
            let half = h * shaft.clamp(0.05, 1.0) / 2.0;
            (
                vec![
                    (0.0, cy - half),
                    (neck, cy - half),
                    (neck, 0.0),
                    (w, cy),
                    (neck, h),
                    (neck, cy + half),
                    (0.0, cy + half),
                ],
                true,
            )
        }
        ShapeKind::Line => (vec![(0.0, cy), (w, cy)], false),
    }
}

/// Vertex `i` of `n` on a circle of radius `r`, starting straight up.
fn polar(r: f32, i: usize, n: usize) -> Pt {
    let a = -PI / 2.0 + TAU * i as f32 / n as f32;
    (r * a.cos(), r * a.sin())
}

/// Stretch points so their bounding box fills `w` x `h`.
fn fit(pts: Vec<Pt>, w: f32, h: f32) -> Vec<Pt> {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for &(x, y) in &pts {
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    let (sx, sy) = (w / (x1 - x0).max(1e-6), h / (y1 - y0).max(1e-6));
    pts.into_iter()
        .map(|(x, y)| ((x - x0) * sx, (y - y0) * sy))
        .collect()
}

fn segment_distance(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (qx, qy) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    (qx * qx + qy * qy).sqrt()
}

fn lerp(a: [u8; 4], b: [u8; 4], t: f32) -> [f32; 4] {
    let mut out = [0.0; 4];
    for i in 0..4 {
        out[i] = a[i] as f32 + (b[i] as f32 - a[i] as f32) * t;
    }
    out
}

/// Rasterize `shape` at 1 px per sequence pixel. The raster is the shape's
/// box plus room for half the stroke on every side; the box is centred in it.
pub fn render(shape: &Shape) -> Raster {
    let (w, h) = (shape.width.max(1.0), shape.height.max(1.0));
    let (pts, closed) = outline(shape);
    let stroke = shape.stroke_px.max(0.0);
    // A line with no stroke would be invisible: draw it 1 px wide.
    let stroke = if closed || stroke > 0.0 { stroke } else { 1.0 };
    let margin = (stroke / 2.0 + 1.0).ceil();
    let width = (w + 2.0 * margin).ceil() as u32;
    let height = (h + 2.0 * margin).ceil() as u32;
    let pts: Vec<Pt> = pts.iter().map(|&(x, y)| (x + margin, y + margin)).collect();
    let edges: Vec<(Pt, Pt)> = if closed {
        (0..pts.len())
            .map(|i| (pts[i], pts[(i + 1) % pts.len()]))
            .collect()
    } else {
        pts.windows(2).map(|p| (p[0], p[1])).collect()
    };
    let fill = if closed { shape.fill } else { Fill::None };
    // Gradient axis through the box centre.
    let (gx, gy, gspan) = match fill {
        Fill::Linear { angle_deg, .. } => {
            let a = angle_deg.to_radians();
            let span = (w * a.cos()).abs() + (h * a.sin()).abs();
            (a.cos(), a.sin(), span.max(1e-3))
        }
        _ => (1.0, 0.0, 1.0),
    };
    let (cx, cy) = (margin + w / 2.0, margin + h / 2.0);
    // Distances beyond this give coverage 0 or 1 whatever the edge.
    let reach = stroke / 2.0 + 1.0;

    let mut rgba8 = vec![0u8; (width * height * 4) as usize];
    let mut near: Vec<(Pt, Pt)> = Vec::new();
    let mut crossings: Vec<f32> = Vec::new();
    for y in 0..height {
        let py = y as f32 + 0.5;
        near.clear();
        crossings.clear();
        for &(a, b) in &edges {
            if a.1.min(b.1) - reach <= py && py <= a.1.max(b.1) + reach {
                near.push((a, b));
            }
            // Even-odd crossings of this row's centre line.
            if closed && (a.1 <= py) != (b.1 <= py) {
                crossings.push(a.0 + (py - a.1) / (b.1 - a.1) * (b.0 - a.0));
            }
        }
        crossings.sort_by(f32::total_cmp);
        for x in 0..width {
            let p = (x as f32 + 0.5, py);
            let d = near
                .iter()
                .map(|&(a, b)| segment_distance(p, a, b))
                .fold(f32::INFINITY, f32::min);
            let inside = crossings.iter().filter(|&&cx| cx < p.0).count() % 2 == 1;
            let fill_cover = if inside {
                (0.5 + d).min(1.0)
            } else {
                (0.5 - d).max(0.0)
            };
            let fill_rgba = match fill {
                Fill::None => [0.0; 4],
                Fill::Solid { color } => lerp(color, color, 0.0),
                Fill::Linear { from, to, .. } => {
                    let t = ((p.0 - cx) * gx + (p.1 - cy) * gy) / gspan + 0.5;
                    lerp(from, to, t.clamp(0.0, 1.0))
                }
            };
            let fa = fill_rgba[3] / 255.0 * fill_cover;
            let sa = if stroke > 0.0 {
                shape.stroke_color[3] as f32 / 255.0 * (stroke / 2.0 + 0.5 - d).clamp(0.0, 1.0)
            } else {
                0.0
            };
            // Stroke over fill, straight alpha.
            let a = sa + fa * (1.0 - sa);
            if a <= 0.0 {
                continue;
            }
            let i = ((y * width + x) * 4) as usize;
            for c in 0..3 {
                let v = (shape.stroke_color[c] as f32 * sa + fill_rgba[c] * fa * (1.0 - sa)) / a;
                rgba8[i + c] = (v + 0.5).clamp(0.0, 255.0) as u8;
            }
            rgba8[i + 3] = (a * 255.0 + 0.5) as u8;
        }
    }
    Raster {
        width,
        height,
        rgba8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(r: &Raster, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * r.width + x) * 4) as usize;
        r.rgba8[i..i + 4].try_into().unwrap()
    }

    fn coverage(r: &Raster) -> f32 {
        r.rgba8.chunks(4).map(|p| p[3] as f32 / 255.0).sum()
    }

    #[test]
    fn solid_rectangle_fills_its_box_with_a_margin() {
        let r = render(&Shape {
            width: 20.0,
            height: 10.0,
            ..Shape::default()
        });
        assert_eq!((r.width, r.height), (22, 12));
        assert_eq!(at(&r, 0, 0)[3], 0);
        assert_eq!(at(&r, 1, 1), [255, 255, 255, 255]);
        assert_eq!(at(&r, 20, 10), [255, 255, 255, 255]);
        assert_eq!(at(&r, 21, 5)[3], 0);
        assert!((coverage(&r) - 200.0).abs() < 0.5);
    }

    #[test]
    fn ellipse_area_and_soft_edges() {
        let r = render(&Shape {
            kind: ShapeKind::Ellipse,
            width: 100.0,
            height: 60.0,
            ..Shape::default()
        });
        let area = PI * 50.0 * 30.0;
        assert!(
            (coverage(&r) - area).abs() / area < 0.005,
            "{}",
            coverage(&r)
        );
        // Anti-aliased: some pixels are partly covered.
        assert!(r.rgba8.chunks(4).any(|p| p[3] > 20 && p[3] < 235));
        assert_eq!(at(&r, 51, 31)[3], 255);
        assert_eq!(at(&r, 2, 2)[3], 0);
    }

    #[test]
    fn rounded_corners_cut_the_corner() {
        let shape = |corner_px| Shape {
            kind: ShapeKind::Rectangle { corner_px },
            width: 40.0,
            height: 40.0,
            ..Shape::default()
        };
        let square = render(&shape(0.0));
        let round = render(&shape(10.0));
        assert_eq!(at(&square, 1, 1)[3], 255);
        assert_eq!(at(&round, 1, 1)[3], 0);
        // Four corners of (1 - pi/4) * r^2 each are gone.
        let lost = coverage(&square) - coverage(&round);
        let expect = 4.0 * (1.0 - PI / 4.0) * 100.0;
        assert!((lost - expect).abs() < 1.0, "{lost} vs {expect}");
    }

    #[test]
    fn stroke_only_outline_leaves_the_middle_empty() {
        let r = render(&Shape {
            width: 30.0,
            height: 30.0,
            fill: Fill::None,
            stroke_px: 4.0,
            stroke_color: [255, 0, 0, 255],
            ..Shape::default()
        });
        assert_eq!((r.width, r.height), (36, 36));
        assert_eq!(at(&r, 18, 18)[3], 0);
        // On the top edge (y = margin 3 + 0): full red.
        assert_eq!(at(&r, 18, 3), [255, 0, 0, 255]);
        // Ring area: outer 34^2 - inner 26^2, less the outer corners, which
        // are rounded (radius 2) like a round join.
        let expect = 34.0f32 * 34.0 - 26.0 * 26.0 - 4.0 * (1.0 - PI / 4.0) * 4.0;
        assert!((coverage(&r) - expect).abs() < 1.0, "{}", coverage(&r));
    }

    #[test]
    fn stroke_draws_over_the_fill() {
        let r = render(&Shape {
            width: 30.0,
            height: 30.0,
            fill: Fill::Solid {
                color: [0, 0, 255, 255],
            },
            stroke_px: 2.0,
            stroke_color: [255, 255, 0, 255],
            ..Shape::default()
        });
        assert_eq!(at(&r, 17, 17), [0, 0, 255, 255]);
        assert_eq!(at(&r, 17, 2), [255, 255, 0, 255]);
    }

    #[test]
    fn linear_gradient_runs_along_its_angle() {
        let shape = |angle_deg| Shape {
            width: 100.0,
            height: 50.0,
            fill: Fill::Linear {
                from: [0, 0, 0, 255],
                to: [255, 255, 255, 255],
                angle_deg,
            },
            ..Shape::default()
        };
        let r = render(&shape(0.0));
        assert!(at(&r, 2, 25)[0] < 5);
        assert!(at(&r, 99, 25)[0] > 250);
        assert!((at(&r, 51, 10)[0] as i32 - 128).abs() < 4);
        // Same column top and bottom.
        assert_eq!(at(&r, 30, 2), at(&r, 30, 48));
        let r = render(&shape(90.0));
        assert!(at(&r, 50, 1)[0] < 5);
        assert!(at(&r, 50, 50)[0] > 250);
    }

    #[test]
    fn star_polygon_and_arrow_have_the_expected_area() {
        let area = |kind| {
            coverage(&render(&Shape {
                kind,
                width: 100.0,
                height: 100.0,
                ..Shape::default()
            }))
        };
        // A triangle fitted to the box is half of it.
        assert!((area(ShapeKind::Polygon { sides: 3 }) - 5000.0).abs() < 10.0);
        // A square "polygon" with four sides is a diamond after fitting: half.
        assert!((area(ShapeKind::Polygon { sides: 4 }) - 5000.0).abs() < 10.0);
        // Arrow: shaft 60 x 40 plus head triangle 40 x 100 / 2.
        let arrow = area(ShapeKind::Arrow {
            head: 0.4,
            shaft: 0.4,
        });
        assert!((arrow - (2400.0 + 2000.0)).abs() < 10.0, "{arrow}");
        // A star is less than its pentagon.
        let star = area(ShapeKind::Star {
            points: 5,
            inner: 0.4,
        });
        assert!(star > 2000.0 && star < area(ShapeKind::Polygon { sides: 5 }));
    }

    #[test]
    fn a_line_is_drawn_with_the_stroke() {
        let r = render(&Shape {
            kind: ShapeKind::Line,
            width: 50.0,
            height: 10.0,
            fill: Fill::Solid {
                color: [255, 0, 0, 255],
            },
            stroke_px: 3.0,
            stroke_color: [0, 255, 0, 255],
        });
        // margin = ceil(1.5 + 1) = 3; centre row y = 3 + 5 = 8.
        assert_eq!(at(&r, 20, 8), [0, 255, 0, 255]);
        assert_eq!(at(&r, 20, 3)[3], 0);
        // The fill does not apply to an open line: 50 x 3 plus round caps.
        let c = coverage(&r);
        assert!(c > 150.0 && c < 160.0, "{c}");
    }
}
