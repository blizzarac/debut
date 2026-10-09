//! Title templates (GFX-02): a style plus an animated Transform effect, sized
//! for the sequence they are dropped into. Animation is ordinary keyframes on
//! the clip's Transform effect, so everything stays editable in the inspector.

use debut_core::{Curve, Interp, Rational};
use debut_project::{Effect, TextAlign, Title, TitleStyle, TransformFx};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
}

pub const TEMPLATES: &[Template] = &[
    Template {
        id: "title",
        name: "Title",
        description: "Centred, static",
    },
    Template {
        id: "fade_title",
        name: "Fade title",
        description: "Centred, fades in and out",
    },
    Template {
        id: "lower_third",
        name: "Lower third",
        description: "Bar at bottom left, slides in and out",
    },
    Template {
        id: "subtitle",
        name: "Subtitle",
        description: "Small, bottom centre, fades",
    },
];

/// What a template produces for a clip: the title and its effect stack.
#[derive(Clone, Debug, PartialEq)]
pub struct Built {
    pub title: Title,
    pub effects: Vec<Effect>,
}

/// Length of the in / out animation.
const RAMP: Rational = Rational { num: 1, den: 2 };

fn fade(duration: Rational) -> Curve {
    let mut c = Curve::constant(1.0);
    c.set(Rational::ZERO, 0.0, Interp::EaseOut);
    c.set(RAMP, 1.0, Interp::Linear);
    c.set(duration - RAMP, 1.0, Interp::EaseIn);
    c.set(duration, 0.0, Interp::Linear);
    c
}

/// Instantiate `id` for a sequence `width` x `height` and a clip `duration`
/// (seconds). `None` for an unknown template.
pub fn build(id: &str, text: &str, width: u32, height: u32, duration: Rational) -> Option<Built> {
    let k = height as f32 / 1080.0;
    let (w, h) = (width as f32, height as f32);
    let title = |style: TitleStyle| Title {
        text: text.to_string(),
        style,
    };
    let scaled = TitleStyle {
        size_px: 72.0 * k,
        ..TitleStyle::default()
    };
    Some(match id {
        "title" => Built {
            title: title(scaled),
            effects: Vec::new(),
        },
        "fade_title" => Built {
            title: title(scaled),
            effects: vec![Effect::Transform(TransformFx {
                opacity: fade(duration),
                ..TransformFx::default()
            })],
        },
        "lower_third" => {
            let style = TitleStyle {
                size_px: 44.0 * k,
                align: TextAlign::Left,
                background: [16, 16, 16, 210],
                padding_px: 18.0 * k,
                min_width_px: w * 0.5,
                ..TitleStyle::default()
            };
            // Bar height for a single line: glyph box plus padding on both sides.
            let bar_h = style.size_px * style.line_height + 2.0 * style.padding_px;
            let margin = 72.0 * k;
            let rest_x = -w * 0.5 + margin + style.min_width_px * 0.5;
            let rest_y = h * 0.5 - margin - bar_h * 0.5;
            let mut x = Curve::constant(rest_x as f64);
            x.set(Rational::ZERO, (rest_x - w) as f64, Interp::EaseOut);
            x.set(RAMP, rest_x as f64, Interp::Linear);
            x.set(duration - RAMP, rest_x as f64, Interp::EaseIn);
            x.set(duration, (rest_x - w) as f64, Interp::Linear);
            Built {
                title: title(style),
                effects: vec![Effect::Transform(TransformFx {
                    x,
                    y: Curve::constant(rest_y as f64),
                    ..TransformFx::default()
                })],
            }
        }
        "subtitle" => {
            let style = TitleStyle::captions_for_height(height);
            let box_h = style.size_px * style.line_height + 2.0 * style.padding_px;
            let rest_y = h * 0.5 - 72.0 * k - box_h * 0.5;
            Built {
                title: title(style),
                effects: vec![Effect::Transform(TransformFx {
                    y: Curve::constant(rest_y as f64),
                    opacity: fade(duration),
                    ..TransformFx::default()
                })],
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use debut_project::Param;

    #[test]
    fn templates_scale_with_the_sequence_and_animate_in_and_out() {
        let d = Rational::from_int(5);
        let lt = build("lower_third", "Name", 1920, 1080, d).unwrap();
        assert_eq!(lt.title.style.min_width_px, 960.0);
        let fx = &lt.effects[0];
        // Off screen at 0, parked from 0.5 s to 4.5 s, gone again at 5 s.
        let x0 = fx.value(Param::X, Rational::ZERO).unwrap();
        let x1 = fx.value(Param::X, Rational::ONE).unwrap();
        let x4 = fx.value(Param::X, Rational::from_int(4)).unwrap();
        let x5 = fx.value(Param::X, d).unwrap();
        assert!(
            x0 < -960.0 && (x1 - x4).abs() < 1e-9 && x5 == x0,
            "{x0} {x1} {x4} {x5}"
        );
        assert!(x1 > -960.0 && x1 < 0.0, "left half of the frame: {x1}");
        // Half-size sequence: half the bar.
        let small = build("lower_third", "Name", 960, 540, d).unwrap();
        assert_eq!(small.title.style.min_width_px, 480.0);
        assert_eq!(small.title.style.size_px, 22.0);

        let ft = build("fade_title", "T", 1920, 1080, d).unwrap();
        let o = |t: f64| {
            ft.effects[0]
                .value(Param::Opacity, Rational::new((t * 100.0) as i64, 100))
                .unwrap()
        };
        assert_eq!(
            (o(0.0), o(0.5), o(2.5), o(4.5), o(5.0)),
            (0.0, 1.0, 1.0, 1.0, 0.0)
        );
        assert!(o(0.25) > 0.0 && o(0.25) < 1.0);
        assert!(build("nope", "", 1, 1, d).is_none());
        assert!(build("title", "Plain", 1920, 1080, d)
            .unwrap()
            .effects
            .is_empty());
    }
}
