use serde_json::{Value, json};

use super::*;

fn data(cols: Value) -> Data {
    Data::from_json(&json!({ "columns": cols }))
}

#[test]
fn slope_labels_move_apart_and_stay_above_the_bottom() {
    let got = dumbbell::spread(&[50.0, 52.0, 10.0, 51.0, 100.0], 11.0, 100.0);
    // in order from the top: 10, 50, 51, 52, 100; pushed down 11 apart, then up from 100
    assert_eq!(got, vec![50.0, 72.0, 10.0, 61.0, 100.0]);
    let tight = dumbbell::spread(&[95.0, 96.0, 97.0], 11.0, 100.0);
    assert_eq!(tight, vec![78.0, 89.0, 100.0]);
    assert!(dumbbell::spread(&[], 11.0, 0.0).is_empty());
}

#[test]
fn dumbbell_labels_move_apart_only_where_their_spans_meet() {
    // three labels 4 px apart: the first two side by side, the third far to the right
    let ys = [50.0, 54.0, 58.0];
    let spans = [(0.0, 20.0), (10.0, 30.0), (100.0, 120.0)];
    let got = dumbbell::spread_boxes(&ys, &spans, 12.0, 200.0);
    assert_eq!(got, vec![50.0, 62.0, 58.0]);
    // spans 2 px apart still count as meeting; every span meeting is plain `spread`
    let got = dumbbell::spread_boxes(&ys, &[(0.0, 10.0), (11.0, 20.0), (21.0, 30.0)], 12.0, 60.0);
    assert_eq!(got, dumbbell::spread(&ys, 12.0, 60.0));
    assert_eq!(got, vec![36.0, 48.0, 60.0]);
    assert!(dumbbell::spread_boxes(&[], &[], 12.0, 0.0).is_empty());
}

#[test]
fn labels_that_cannot_fit_are_left_out() {
    // five labels 11 px apart need 44 px; between 70 and 100 only three fit, the top ones go
    let ys = [80.0, 81.0, 82.0, 83.0, 84.0];
    let place = |keep: &[usize]| {
        let sub: Vec<f64> = keep.iter().map(|i| ys[*i]).collect();
        dumbbell::spread(&sub, 11.0, 100.0)
    };
    let got = dumbbell::fit_inside(&ys, 70.0, &place);
    assert_eq!(got, vec![None, None, Some(78.0), Some(89.0), Some(100.0)]);
    // room for all: as `spread` places them
    let got = dumbbell::fit_inside(&ys, 0.0, &place);
    assert_eq!(
        got,
        vec![Some(56.0), Some(67.0), Some(78.0), Some(89.0), Some(100.0)]
    );
    assert!(dumbbell::fit_inside(&[], 0.0, &place).is_empty());
}

#[test]
fn bins_by_count_and_by_size() {
    let v: Vec<(f64, f64)> = [1.0, 2.0, 2.5, 9.0, 10.0].iter().map(|x| (*x, 1.0)).collect();
    let b = histogram::bins(&v, 3, 0.0).unwrap();
    assert_eq!((b.start, b.size), (1.0, 3.0));
    // the largest value goes in the last bin
    assert_eq!(b.counts, vec![3.0, 0.0, 2.0]);
    let b = histogram::bins(&v, 3, 5.0).unwrap();
    // bins of 5 from 0: [0, 5), [5, 10), [10, 15)
    assert_eq!((b.start, b.size), (0.0, 5.0));
    assert_eq!(b.counts, vec![3.0, 1.0, 1.0]);
    // one value: one bin around it
    let b = histogram::bins(&[(4.0, 2.0)], 10, 0.0).unwrap();
    assert_eq!((b.start, b.size, b.counts), (3.5, 1.0, vec![2.0]));
    // too many bins of the size asked: at most MAX_BINS
    let b = histogram::bins(&[(0.0, 1.0), (1e9, 1.0)], 10, 1.0).unwrap();
    assert_eq!(b.counts.len(), histogram::MAX_BINS);
    assert_eq!(b.counts.iter().sum::<f64>(), 2.0);
    assert!(histogram::bins(&[], 10, 0.0).is_none());
}

#[test]
fn box_statistics_follow_the_interpolated_quartiles_and_1_5_iqr() {
    let s = box_plot::stats(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 100.0]).unwrap();
    assert_eq!((s.q1, s.median, s.q3), (3.0, 5.0, 7.0));
    // fences 3 − 6 = −3 and 7 + 6 = 13: 100 is an outlier, the whisker ends at 8
    assert_eq!((s.low, s.high), (1.0, 8.0));
    assert_eq!(s.outliers, vec![100.0]);
    assert_eq!(s.n, 9);
    let one = box_plot::stats(&[5.0]).unwrap();
    assert_eq!(
        (one.q1, one.median, one.q3, one.low, one.high),
        (5.0, 5.0, 5.0, 5.0, 5.0)
    );
    assert!(box_plot::stats(&[]).is_none());
    let even = box_plot::stats(&[4.0, 1.0, 3.0, 2.0]).unwrap();
    assert_eq!((even.q1, even.median, even.q3), (1.75, 2.5, 3.25));
}

#[test]
fn options_fall_back_to_their_defaults() {
    let o = json!({"bins": {"count": 1e9, "size": "x"}, "colors": {"fill": "red", "line": "#a1b2c3"}, "style": {"slope": "yes"}});
    let o = Opts(&o);
    assert_eq!(o.int("bins", "count", 10.0, 1.0, 200.0), 200.0);
    assert_eq!(o.num("bins", "size", 0.0, 0.0, 1e15), 0.0);
    assert_eq!(o.color("colors", "fill", "#123456"), "#123456");
    assert_eq!(o.color("colors", "line", "#123456"), "#A1B2C3");
    assert!(!o.flag("style", "slope", false));
    assert!(o.flag("labels", "show", true));
}

/// Hostile data never panics and draws within bounds: empty, blank, huge and odd values, every
/// size from nothing to large, every kind.
#[test]
fn hostile_data_never_panics() {
    let odd = json!([null, "x", 1e300, -1e300, 0, true, {"a": 1}, [1], -0.0, 5e-324]);
    let roles = [
        "values",
        "frequency",
        "category",
        "value",
        "target",
        "bands",
        "maximum",
        "sample",
        "start",
        "end",
    ];
    let cols: Vec<Value> = roles
        .iter()
        .map(|r| json!({"role": r, "name": "n".repeat(500), "values": odd}))
        .collect();
    let datasets = [
        data(json!([])),
        data(Value::Array(cols)),
        Data::from_json(&json!({"columns": 3})),
    ];
    let opts = [
        json!(null),
        json!({"bins": {"size": 1e-300}}),
        json!({"style": {"slope": true}, "bins": {"count": 1}}),
    ];
    for kind in KINDS {
        for d in &datasets {
            for o in &opts {
                for (w, h) in [
                    (0.0, 0.0),
                    (1.0, 1.0),
                    (40.0, 30.0),
                    (300.0, 200.0),
                    (f64::NAN, 10.0),
                    (1e6, 1e6),
                ] {
                    let p = layout(kind, d, o, w, h).unwrap();
                    assert!(p.len() < 20_000, "{kind}");
                }
            }
        }
    }
    assert!(layout("pie", &Data::default(), &Value::Null, 10.0, 10.0).is_err());
}
