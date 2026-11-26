//! The icons of BIcraft's own visuals (ROADMAP M12.4): one drawing per visual, original work of
//! the BIcraft contributors, written as marks on the UI icons' 24 × 24 design grid
//! (docs/design.md §6). The app's Visualizations pane paints them with its icon pen in the
//! theme's ink and the blue accent, exactly as it paints the built-in types (`bicraft-ui`
//! `icons::paint_visual_type`); the engine rasterizes the same marks into each package's 20 × 20
//! `assets/icon.png`. Nothing is traced from or modelled on any vendor's artwork (AGENTS.md §2).

/// A point of the 24 × 24 design grid.
pub type Pt = (f32, f32);

/// The side of the design grid.
pub const GRID: f32 = 24.0;

/// The accent of the visual-type pictures (`bicraft-ui` `icons::BLUE`), for drawings that do not
/// go through the UI (the packages' PNG icons).
pub const ACCENT_RGB: [u8; 3] = [0x2e, 0x6f, 0xd8];

/// A mark's colour: the ink (the theme's text colour) or the accent, at an opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Ink(f32),
    Accent(f32),
}

/// One drawing primitive. Stroke widths are multiples of the icon's stroke (≈ size/16 at 16 px,
/// size/20 at 32 px).
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// An open polyline with round caps, `width` times the stroke.
    Line { pts: Vec<Pt>, width: f32, tone: Tone },
    /// A filled simple polygon.
    Fill { pts: Vec<Pt>, tone: Tone },
    /// A rectangle with rounded corners, filled, outlined (one stroke) or both.
    Rect {
        min: Pt,
        max: Pt,
        radius: f32,
        fill: Option<Tone>,
        stroke: Option<Tone>,
    },
    /// A circle, filled, outlined (one stroke) or both.
    Circle {
        center: Pt,
        radius: f32,
        fill: Option<Tone>,
        stroke: Option<Tone>,
    },
}

/// The visuals with a drawing, by their `pbiviz.json` name, in the Visualizations pane's order.
pub const BICRAFT_VISUAL_ICONS: &[&str] = &[
    "bicraftSpec",
    "bicraftHistogram",
    "bicraftBullet",
    "bicraftBoxPlot",
    "bicraftDumbbell",
    "bicraftOrgChart",
    "bicraftVenn",
    "bicraftMarimekko",
    "bicraftCalendarHeatmap",
    "bicraftSankey",
    "bicraftScatterPlus",
    "bicraftLineAreaPlus",
    "bicraftWaterfallPlus",
    "bicraftGaugeKpi",
];

/// The drawing of BIcraft's visual named `name` (its `pbiviz.json` name, ignoring case), `None`
/// for any other name.
pub fn bicraft_visual_icon(name: &str) -> Option<Vec<Mark>> {
    let key = BICRAFT_VISUAL_ICONS
        .iter()
        .copied()
        .find(|n| n.eq_ignore_ascii_case(name))?;
    Some(match key {
        "bicraftSpec" => spec(),
        "bicraftHistogram" => histogram(),
        "bicraftBullet" => bullet(),
        "bicraftBoxPlot" => box_plot(),
        "bicraftDumbbell" => dumbbell(),
        "bicraftOrgChart" => org_chart(),
        "bicraftVenn" => venn(),
        "bicraftMarimekko" => marimekko(),
        "bicraftCalendarHeatmap" => calendar(),
        "bicraftSankey" => sankey(),
        "bicraftScatterPlus" => scatter_plus(),
        "bicraftLineAreaPlus" => line_area_plus(),
        "bicraftWaterfallPlus" => waterfall(),
        _ => gauge(),
    })
}

const INK_AXIS: Tone = Tone::Ink(0.45);
const ACC: Tone = Tone::Accent(1.0);

fn line(pts: &[Pt], width: f32, tone: Tone) -> Mark {
    Mark::Line {
        pts: pts.to_vec(),
        width,
        tone,
    }
}

fn bar(x0: f32, y0: f32, x1: f32, y1: f32, tone: Tone) -> Mark {
    Mark::Rect {
        min: (x0, y0),
        max: (x1, y1),
        radius: 0.4,
        fill: Some(tone),
        stroke: None,
    }
}

/// The light axes of a chart picture (the built-in types' `axis`).
fn axis() -> Mark {
    line(&[(4.0, 3.5), (4.0, 20.5), (21.0, 20.5)], 1.0, INK_AXIS)
}

/// Spec: a column chart between braces (a chart written as a spec).
fn spec() -> Vec<Mark> {
    let left = [
        (7.0, 4.0),
        (5.9, 4.0),
        (5.0, 4.9),
        (5.0, 10.6),
        (3.4, 12.0),
        (5.0, 13.4),
        (5.0, 19.1),
        (5.9, 20.0),
        (7.0, 20.0),
    ];
    let right: Vec<Pt> = left.iter().map(|&(x, y)| (GRID - x, y)).collect();
    vec![
        line(&left, 1.0, Tone::Ink(0.7)),
        line(&right, 1.0, Tone::Ink(0.7)),
        bar(8.0, 11.5, 10.2, 17.5, Tone::Accent(0.55)),
        bar(10.9, 7.0, 13.1, 17.5, ACC),
        bar(13.8, 9.5, 16.0, 17.5, ACC),
    ]
}

/// Histogram: contiguous bins rising and falling around a peak.
fn histogram() -> Vec<Mark> {
    let mut v = vec![axis()];
    for (i, top) in [15.0, 11.0, 6.0, 4.5, 9.0, 14.0].into_iter().enumerate() {
        let x0 = 5.5 + 2.5 * i as f32;
        v.push(bar(x0, top, x0 + 1.9, 20.0, ACC));
    }
    v
}

/// Bullet: two rows, each a measure bar over its qualitative bands with a target tick.
fn bullet() -> Vec<Mark> {
    let mut v = Vec::new();
    for (y, good, value, target) in [(4.5, 15.0, 16.5, 18.5), (13.5, 12.0, 11.0, 14.5)] {
        v.push(bar(3.0, y, 21.0, y + 5.5, Tone::Accent(0.18)));
        v.push(bar(3.0, y, good, y + 5.5, Tone::Accent(0.25)));
        v.push(bar(3.0, y + 1.9, value, y + 3.6, ACC));
        v.push(line(
            &[(target, y - 0.4), (target, y + 5.9)],
            1.3,
            Tone::Ink(0.85),
        ));
    }
    v
}

/// Box plot: two boxes with their medians and whiskers.
fn box_plot() -> Vec<Mark> {
    let mut v = vec![axis()];
    for (cx, lo, q1, med, q3, hi) in [
        (9.0, 18.5, 15.0, 12.5, 9.0, 5.0),
        (17.0, 16.0, 12.0, 9.5, 7.0, 4.0),
    ] {
        let whisker = Tone::Ink(0.65);
        v.push(line(&[(cx, hi), (cx, q3)], 1.0, whisker));
        v.push(line(&[(cx, q1), (cx, lo)], 1.0, whisker));
        v.push(line(&[(cx - 1.5, hi), (cx + 1.5, hi)], 1.0, whisker));
        v.push(line(&[(cx - 1.5, lo), (cx + 1.5, lo)], 1.0, whisker));
        v.push(Mark::Rect {
            min: (cx - 2.5, q3),
            max: (cx + 2.5, q1),
            radius: 0.5,
            fill: Some(Tone::Accent(0.3)),
            stroke: Some(ACC),
        });
        v.push(line(&[(cx - 2.5, med), (cx + 2.5, med)], 1.3, ACC));
    }
    v
}

/// Dumbbell: three rows, each a start (a ring) joined to an end (a dot).
fn dumbbell() -> Vec<Mark> {
    let mut v = vec![line(&[(3.5, 3.5), (3.5, 20.5)], 1.0, INK_AXIS)];
    let r = 1.9;
    for (y, a, b) in [(6.0, 7.0, 17.5), (12.0, 10.5, 19.5), (18.0, 6.5, 13.5)] {
        v.push(line(&[(a + r, y), (b - r, y)], 1.4, Tone::Accent(0.5)));
        v.push(Mark::Circle {
            center: (a, y),
            radius: r,
            fill: None,
            stroke: Some(ACC),
        });
        v.push(Mark::Circle {
            center: (b, y),
            radius: r,
            fill: Some(ACC),
            stroke: None,
        });
    }
    v
}

/// Org chart: a head card linked to two report cards.
fn org_chart() -> Vec<Mark> {
    let link = Tone::Ink(0.6);
    let card = |x0: f32, y0: f32, x1: f32, y1: f32, tone: Tone| Mark::Rect {
        min: (x0, y0),
        max: (x1, y1),
        radius: 0.9,
        fill: Some(tone),
        stroke: None,
    };
    vec![
        line(&[(12.0, 8.0), (12.0, 11.0)], 1.0, link),
        line(&[(6.5, 14.0), (6.5, 11.0), (17.5, 11.0), (17.5, 14.0)], 1.0, link),
        card(8.5, 3.5, 15.5, 8.0, ACC),
        card(3.0, 14.0, 10.0, 19.5, Tone::Accent(0.55)),
        card(14.0, 14.0, 21.0, 19.5, Tone::Accent(0.55)),
    ]
}

/// Venn: three overlapping sets (their overlaps darker).
fn venn() -> Vec<Mark> {
    [(9.3, 9.4), (14.7, 9.4), (12.0, 14.2)]
        .into_iter()
        .map(|center| Mark::Circle {
            center,
            radius: 5.4,
            fill: Some(Tone::Accent(0.28)),
            stroke: Some(ACC),
        })
        .collect()
}

/// Marimekko: full-height columns of different widths, each split in two.
fn marimekko() -> Vec<Mark> {
    let mut v = vec![axis()];
    for (x0, x1, split) in [(5.5, 10.9, 11.0), (12.1, 15.4, 7.5), (16.6, 20.5, 13.5)] {
        v.push(bar(x0, 4.0, x1, split - 0.4, Tone::Accent(0.4)));
        v.push(bar(x0, split, x1, 20.0, ACC));
    }
    v
}

/// Calendar heatmap: a week grid of days in shades of the accent under the calendar's top.
fn calendar() -> Vec<Mark> {
    const SHADES: [[f32; 5]; 4] = [
        [0.2, 0.55, 1.0, 0.35, 0.75],
        [0.75, 0.2, 0.55, 1.0, 0.35],
        [0.35, 1.0, 0.2, 0.55, 0.75],
        [0.55, 0.35, 0.75, 0.2, 1.0],
    ];
    let mut v = vec![line(&[(3.5, 4.0), (20.5, 4.0)], 1.0, Tone::Ink(0.6))];
    for (i, row) in SHADES.iter().enumerate() {
        for (j, &a) in row.iter().enumerate() {
            let (x, y) = (3.5 + 3.5 * j as f32, 6.5 + 3.5 * i as f32);
            v.push(Mark::Rect {
                min: (x, y),
                max: (x + 3.0, y + 3.0),
                radius: 0.5,
                fill: Some(Tone::Accent(a)),
                stroke: None,
            });
        }
    }
    v
}

/// The points of a cubic Bézier from `a` to `b` leaving and arriving horizontally.
fn s_curve(a: Pt, b: Pt, n: usize) -> Vec<Pt> {
    let mx = (a.0 + b.0) * 0.5;
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (
                w0 * a.0 + w1 * mx + w2 * mx + w3 * b.0,
                w0 * a.1 + w1 * a.1 + w2 * b.1 + w3 * b.1,
            )
        })
        .collect()
}

/// Sankey: two sources and two destinations joined by crossing flows.
fn sankey() -> Vec<Mark> {
    let (x0, x1) = (5.0, 19.0);
    let mut v = Vec::new();
    // (source top, bottom) → (destination top, bottom)
    for (s0, s1, d0, d1) in [
        (4.0, 7.5, 4.0, 7.5),
        (7.5, 12.0, 12.0, 16.5),
        (14.0, 16.0, 7.5, 9.5),
        (16.0, 20.0, 16.5, 20.5),
    ] {
        let mut pts = s_curve((x0, s0), (x1, d0), 12);
        pts.extend(s_curve((x1, d1), (x0, s1), 12));
        v.push(Mark::Fill {
            pts,
            tone: Tone::Accent(0.4),
        });
    }
    for (x, y0, y1) in [
        (3.0, 4.0, 12.0),
        (3.0, 14.0, 20.0),
        (19.0, 4.0, 9.5),
        (19.0, 12.0, 20.5),
    ] {
        v.push(bar(x, y0, x + 2.0, y1, ACC));
    }
    v
}

/// Scatter+: points of several sizes, filled or ringed, along a trend line, under a marginal
/// histogram.
fn scatter_plus() -> Vec<Mark> {
    let mut v = vec![axis(), line(&[(6.0, 18.5), (20.0, 8.0)], 1.0, Tone::Ink(0.6))];
    for (x0, top) in [(7.0, 4.2), (9.2, 2.6), (11.4, 3.4), (13.6, 2.2), (15.8, 3.8)] {
        v.push(bar(x0, top, x0 + 1.6, 5.6, Tone::Accent(0.45)));
    }
    for (center, radius, ring) in [
        ((8.0, 16.2), 1.4, false),
        ((11.0, 13.6), 2.0, true),
        ((14.4, 11.2), 1.5, false),
        ((17.6, 9.0), 2.4, true),
        ((12.6, 17.4), 1.2, false),
    ] {
        v.push(Mark::Circle {
            center,
            radius,
            fill: Some(if ring { Tone::Accent(0.45) } else { ACC }),
            stroke: ring.then_some(ACC),
        });
    }
    v
}

/// Line / Area+: a band shaded between two lines, the first ending in a dot.
fn line_area_plus() -> Vec<Mark> {
    let upper = [(5.0, 13.0), (9.0, 9.0), (13.0, 11.0), (17.0, 6.0), (20.0, 7.0)];
    let lower = [(5.0, 16.0), (9.0, 14.5), (13.0, 15.0), (17.0, 12.0), (20.0, 13.5)];
    let mut band: Vec<Pt> = upper.to_vec();
    band.extend(lower.iter().rev());
    vec![
        axis(),
        Mark::Fill {
            pts: band,
            tone: Tone::Accent(0.25),
        },
        line(&lower, 1.0, Tone::Ink(0.55)),
        line(&upper, 1.4, ACC),
        Mark::Circle {
            center: (20.0, 7.0),
            radius: 1.6,
            fill: Some(ACC),
            stroke: None,
        },
    ]
}

/// Waterfall+: floating steps up and down joined by connectors, ending on a full total column
/// with a target tick across it.
fn waterfall() -> Vec<Mark> {
    let mut v = vec![axis()];
    let link = Tone::Ink(0.55);
    // (left, top, bottom) of each step; the steps rise, dip, rise again
    let steps = [(5.5, 13.0, 20.0), (9.0, 8.0, 13.0), (12.5, 8.0, 11.0)];
    for (i, (x, top, bottom)) in steps.iter().enumerate() {
        let tone = if i == 2 { Tone::Accent(0.45) } else { ACC };
        v.push(bar(*x, *top, x + 2.6, *bottom, tone));
    }
    v.push(line(&[(8.1, 13.0), (9.0, 13.0)], 0.8, link));
    v.push(line(&[(11.6, 8.0), (12.5, 8.0)], 0.8, link));
    v.push(line(&[(15.1, 11.0), (16.5, 11.0)], 0.8, link));
    v.push(bar(16.5, 11.0, 19.6, 20.0, ACC));
    v.push(line(&[(15.8, 6.5), (20.3, 6.5)], 1.3, Tone::Ink(0.85)));
    v
}

/// Gauge / KPI+: a half ring of three bands with a needle to the value and a target tick.
fn gauge() -> Vec<Mark> {
    let (cx, cy, r0, r1) = (12.0, 17.0, 5.6, 9.0);
    let at = |r: f32, f: f32| {
        let a = std::f32::consts::PI * (1.0 - f);
        (cx + r * a.cos(), cy - r * a.sin())
    };
    let ring = |f0: f32, f1: f32| {
        let n = 8;
        let mut pts: Vec<Pt> = (0..=n)
            .map(|i| at(r1, f0 + (f1 - f0) * i as f32 / n as f32))
            .collect();
        pts.extend(
            (0..=n)
                .rev()
                .map(|i| at(r0, f0 + (f1 - f0) * i as f32 / n as f32)),
        );
        pts
    };
    let mut v = Vec::new();
    for (f0, f1, a) in [(0.0, 0.42, 0.3), (0.44, 0.72, 0.55), (0.74, 1.0, 1.0)] {
        v.push(Mark::Fill {
            pts: ring(f0, f1),
            tone: Tone::Accent(a),
        });
    }
    v.push(line(&[(cx, cy), at(7.6, 0.64)], 1.4, Tone::Ink(0.85)));
    v.push(Mark::Circle {
        center: (cx, cy),
        radius: 1.5,
        fill: Some(Tone::Ink(0.85)),
        stroke: None,
    });
    v.push(line(&[at(5.0, 0.86), at(9.8, 0.86)], 1.0, Tone::Ink(0.6)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn points(m: &Mark) -> Vec<Pt> {
        match m {
            Mark::Line { pts, .. } | Mark::Fill { pts, .. } => pts.clone(),
            Mark::Rect { min, max, .. } => vec![*min, *max],
            Mark::Circle { center, radius, .. } => vec![
                (center.0 - radius, center.1 - radius),
                (center.0 + radius, center.1 + radius),
            ],
        }
    }

    /// Every visual has a drawing inside the grid, and no two drawings are the same.
    #[test]
    fn every_visual_has_its_own_drawing() {
        let all: Vec<Vec<Mark>> = BICRAFT_VISUAL_ICONS
            .iter()
            .map(|n| bicraft_visual_icon(n).unwrap_or_else(|| panic!("{n} has no drawing")))
            .collect();
        for (n, marks) in BICRAFT_VISUAL_ICONS.iter().zip(&all) {
            assert!(marks.len() >= 3, "{n}: too few marks");
            for p in marks.iter().flat_map(points) {
                assert!(
                    p.0.is_finite()
                        && p.1.is_finite()
                        && (0.0..=GRID).contains(&p.0)
                        && (0.0..=GRID).contains(&p.1),
                    "{n}: {p:?} is off the grid"
                );
            }
        }
        for (i, a) in all.iter().enumerate() {
            for (j, b) in all.iter().enumerate().skip(i + 1) {
                assert_ne!(
                    a, b,
                    "{} and {}",
                    BICRAFT_VISUAL_ICONS[i], BICRAFT_VISUAL_ICONS[j]
                );
            }
        }
        assert_eq!(
            bicraft_visual_icon("BICRAFTVENN"),
            bicraft_visual_icon("bicraftVenn")
        );
        assert!(bicraft_visual_icon("card").is_none());
        assert!(bicraft_visual_icon("").is_none());
    }
}
