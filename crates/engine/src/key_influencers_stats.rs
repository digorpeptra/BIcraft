//! The arithmetic of BIcraft's key influencers estimate (`docs/key-influencers.md`): pure
//! functions over per-value summaries ([`Cell`]), so they are tested on planted data without a
//! session. Nothing here reproduces Power BI's own analysis: the method is BIcraft's own, written
//! from textbook statistics (Welch's t-test, least squares, a CART-style regression tree).
//!
//! A categorical target is analysed as a 0/1 indicator of the selected class: a cell's `sum` is
//! then the class count and its mean the class rate, so one code path serves both kinds.

/// The summary of the target over the rows sharing one value: count, sum, and the sum of squared
/// deviations from the cell's mean.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Cell {
    pub n: f64,
    pub sum: f64,
    pub ss: f64,
}

impl Cell {
    /// From a count, a mean and a population variance (the engine's `CountNonNull`, `Avg`,
    /// `Variance` aggregations).
    pub fn from_moments(n: f64, mean: f64, var_p: f64) -> Cell {
        let (n, mean, var) = (fin(n).max(0.0), fin(mean), fin(var_p).max(0.0));
        Cell {
            n,
            sum: n * mean,
            ss: n * var,
        }
    }

    /// `k` rows of the selected class out of `n` (an indicator's moments).
    pub fn from_counts(n: f64, k: f64) -> Cell {
        let n = fin(n).max(0.0);
        let k = fin(k).clamp(0.0, n);
        let ss = if n > 0.0 { k * (1.0 - k / n) } else { 0.0 };
        Cell { n, sum: k, ss }
    }

    /// One observation.
    pub fn one(y: f64) -> Cell {
        Cell {
            n: 1.0,
            sum: fin(y),
            ss: 0.0,
        }
    }

    pub fn mean(&self) -> f64 {
        if self.n > 0.0 { self.sum / self.n } else { 0.0 }
    }

    /// Both cells' rows (Chan's parallel formula).
    pub fn merge(&self, o: &Cell) -> Cell {
        let n = self.n + o.n;
        if n <= 0.0 {
            return Cell::default();
        }
        let d = o.mean() - self.mean();
        Cell {
            n,
            sum: self.sum + o.sum,
            ss: (self.ss + o.ss + d * d * self.n * o.n / n).max(0.0),
        }
    }

    /// The rows of `self` that are not in `part` (`part` ⊂ `self`).
    pub fn minus(&self, part: &Cell) -> Cell {
        let n = self.n - part.n;
        if n <= 0.0 {
            return Cell::default();
        }
        let sum = self.sum - part.sum;
        let (mp, mr) = (part.mean(), sum / n);
        let d = mp - mr;
        Cell {
            n,
            sum,
            ss: (self.ss - part.ss - d * d * part.n * n / self.n).max(0.0),
        }
    }

    /// Sample variance (0 under two rows).
    pub fn var_s(&self) -> f64 {
        if self.n > 1.0 {
            self.ss / (self.n - 1.0)
        } else {
            0.0
        }
    }
}

fn fin(x: f64) -> f64 {
    if x.is_finite() { x } else { 0.0 }
}

/// Thresholds of the estimate (documented in `docs/key-influencers.md`).
pub(crate) const MIN_COUNT: f64 = 10.0;
/// Family-wise significance level; each test uses `ALPHA / tests` (Bonferroni).
pub(crate) const ALPHA: f64 = 0.05;
/// Bins of a numeric explain-by field (equal-count, at most).
pub(crate) const BINS: usize = 5;
/// Deepest split of the top segments tree.
pub(crate) const TREE_DEPTH: usize = 3;
/// Smallest segment: this many rows and this share of the rows.
pub(crate) const MIN_SEGMENT: f64 = 20.0;
pub(crate) const MIN_SEGMENT_SHARE: f64 = 0.05;
/// Categories tried as a split of one field (the most frequent ones).
pub(crate) const SPLIT_CATEGORIES: usize = 30;
/// Segments returned.
pub(crate) const MAX_SEGMENTS: usize = 6;

// ---------------------------------------------------------------------------------------------
// distributions
// ---------------------------------------------------------------------------------------------

/// ln Γ(x), x > 0 (Lanczos approximation, g = 7, 9 terms).
#[allow(clippy::excessive_precision)]
fn ln_gamma(x: f64) -> f64 {
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        // reflection
        let s = (std::f64::consts::PI * x).sin();
        if s == 0.0 {
            return f64::INFINITY;
        }
        return std::f64::consts::PI.ln() - s.abs().ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + 7.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Continued fraction of the incomplete beta function (modified Lentz), at most 300 terms.
fn beta_cf(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < TINY {
        d = TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=300 {
        let m = m as f64;
        let m2 = 2.0 * m;
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < TINY {
            d = TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < TINY {
            c = TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < 1e-12 {
            break;
        }
    }
    h
}

/// The regularized incomplete beta function I_x(a, b).
pub(crate) fn inc_beta(a: f64, b: f64, x: f64) -> f64 {
    if a.is_nan() || b.is_nan() || a <= 0.0 || b <= 0.0 || x.is_nan() {
        return f64::NAN;
    }
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let ln_front = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln();
    let front = ln_front.exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        (front * beta_cf(a, b, x) / a).clamp(0.0, 1.0)
    } else {
        (1.0 - front * beta_cf(b, a, 1.0 - x) / b).clamp(0.0, 1.0)
    }
}

/// Two-sided p-value of Student's t with `df` degrees of freedom.
pub(crate) fn t_two_sided(t: f64, df: f64) -> f64 {
    if t.is_nan() || df.is_nan() || df <= 0.0 {
        return 1.0;
    }
    if t.is_infinite() {
        return 0.0;
    }
    inc_beta(df / 2.0, 0.5, df / (df + t * t)).clamp(0.0, 1.0)
}

/// Welch's t-test of the means of `a` and `b`: (t, two-sided p).
pub(crate) fn welch(a: &Cell, b: &Cell) -> (f64, f64) {
    if a.n < 2.0 || b.n < 2.0 {
        return (0.0, 1.0);
    }
    let (va, vb) = (a.var_s() / a.n, b.var_s() / b.n);
    let diff = a.mean() - b.mean();
    let se2 = va + vb;
    if se2 <= 0.0 {
        // no spread on either side: any difference is certain
        return if diff.abs() > 0.0 {
            (diff.signum() * f64::INFINITY, 0.0)
        } else {
            (0.0, 1.0)
        };
    }
    let t = diff / se2.sqrt();
    let den = va * va / (a.n - 1.0) + vb * vb / (b.n - 1.0);
    let df = if den > 0.0 {
        se2 * se2 / den
    } else {
        a.n + b.n - 2.0
    };
    (t, t_two_sided(t, df.max(1.0)))
}

// ---------------------------------------------------------------------------------------------
// influencers
// ---------------------------------------------------------------------------------------------

/// What one explain-by field looks like to the analysis.
#[derive(Debug, Clone)]
pub(crate) enum FieldCells {
    /// Category key (display order kept) → cell.
    Categories(Vec<(String, Cell)>),
    /// Numeric value → cell, any order.
    Numbers(Vec<(f64, Cell)>),
}

/// The goal: which direction of the target counts as an influence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Goal {
    Increase,
    Decrease,
    /// A categorical target: the selected class's rate increases.
    Class,
}

impl Goal {
    fn sign(self) -> f64 {
        match self {
            Goal::Decrease => -1.0,
            _ => 1.0,
        }
    }
}

/// How an influencer is defined.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum What {
    /// The field equals this category.
    Is(String),
    /// The field is in the bin [from, to] (a categorical target's numeric field).
    Bin { from: f64, to: f64 },
    /// The field goes up by `step` (a numeric target's numeric field).
    Up { step: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Influencer {
    pub field: usize,
    pub what: What,
    /// The target's mean (or the class rate) in the group; for `Up`, the change per step.
    pub value: f64,
    /// The mean (rate) of the other rows; for `Up`, the overall mean.
    pub rest: f64,
    /// Numeric target: value − rest (or the change per step); class target: value / rest.
    pub effect: f64,
    pub count: f64,
    pub p: f64,
}

/// One bar of a field's detail chart.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bar {
    pub label: BarLabel,
    pub mean: f64,
    pub count: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BarLabel {
    Category(String),
    Range(f64, f64),
}

/// Equal-count bins of numeric cells: (from, to, merged cell), at most `bins`, ascending.
pub(crate) fn bin_numbers(cells: &[(f64, Cell)], bins: usize) -> Vec<(f64, f64, Cell)> {
    let mut v: Vec<(f64, Cell)> = cells
        .iter()
        .filter(|(x, c)| x.is_finite() && c.n > 0.0)
        .copied()
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = v.iter().map(|(_, c)| c.n).sum();
    let bins = bins.max(1);
    if total <= 0.0 {
        return Vec::new();
    }
    let mut out: Vec<(f64, f64, Cell)> = Vec::new();
    let mut acc = Cell::default();
    let mut from = f64::NAN;
    let mut seen = 0.0;
    for (x, c) in &v {
        if acc.n == 0.0 {
            from = *x;
        }
        acc = acc.merge(c);
        seen += c.n;
        // close the bin when the running count passes the next quantile
        let k = out.len() + 1;
        if seen >= total * k as f64 / bins as f64 - 1e-9 && out.len() + 1 < bins {
            out.push((from, *x, acc));
            acc = Cell::default();
        }
    }
    if acc.n > 0.0 {
        out.push((from, v.last().map_or(from, |l| l.0), acc));
    }
    out
}

/// A round step near `x` (1, 2 or 5 × 10ⁿ).
pub(crate) fn nice_step(x: f64) -> f64 {
    if !(x.is_finite() && x > 0.0) {
        return 1.0;
    }
    let e = x.log10().floor();
    let base = 10f64.powf(e);
    let m = x / base;
    let k = if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    };
    k * base
}

/// Least squares of the target on a numeric field from its cells: (slope, t, df, sd of x,
/// overall cell).
fn slope(cells: &[(f64, Cell)]) -> Option<(f64, f64, f64, f64, Cell)> {
    let all = cells
        .iter()
        .filter(|(x, c)| x.is_finite() && c.n > 0.0)
        .fold(Cell::default(), |a, (_, c)| a.merge(c));
    let n = all.n;
    if n < 3.0 {
        return None;
    }
    let xbar = cells
        .iter()
        .filter(|(x, c)| x.is_finite() && c.n > 0.0)
        .map(|(x, c)| x * c.n)
        .sum::<f64>()
        / n;
    let ybar = all.mean();
    let (mut sxx, mut sxy) = (0.0, 0.0);
    for (x, c) in cells.iter().filter(|(x, c)| x.is_finite() && c.n > 0.0) {
        let dx = x - xbar;
        sxx += c.n * dx * dx;
        sxy += c.n * dx * (c.mean() - ybar);
    }
    if sxx.is_nan() || sxx <= 0.0 {
        return None;
    }
    let b = sxy / sxx;
    let a = ybar - b * xbar;
    let mut sse = 0.0;
    for (x, c) in cells.iter().filter(|(x, c)| x.is_finite() && c.n > 0.0) {
        let r = c.mean() - (a + b * x);
        sse += c.ss + c.n * r * r;
    }
    let df = n - 2.0;
    let s2 = sse / df;
    let t = if s2 > 0.0 {
        b / (s2 / sxx).sqrt()
    } else if b != 0.0 {
        b.signum() * f64::INFINITY
    } else {
        0.0
    };
    Some((b, t, df, (sxx / n).sqrt(), all))
}

/// The analysis of one field: candidate influencers (not yet filtered by significance) and the
/// field's detail bars.
#[derive(Debug, Clone, Default)]
pub(crate) struct FieldResult {
    pub candidates: Vec<Influencer>,
    pub bars: Vec<Bar>,
    /// Numeric fields: the bin edges the segments may split at.
    pub edges: Vec<f64>,
    /// Numbers of tests run (for the Bonferroni correction).
    pub tests: usize,
}

/// Analyses one explain-by field.
pub(crate) fn analyse_field(field: usize, cells: &FieldCells, goal: Goal) -> FieldResult {
    let mut out = FieldResult::default();
    let categories: Vec<(String, Cell)> = match cells {
        FieldCells::Categories(c) => c.clone(),
        FieldCells::Numbers(nums) => {
            let bins = bin_numbers(nums, BINS);
            out.edges = bins
                .iter()
                .take(bins.len().saturating_sub(1))
                .map(|b| b.1)
                .collect();
            if goal != Goal::Class {
                if let Some((b, t, df, sd, all)) = slope(nums) {
                    let step = nice_step(sd);
                    out.tests = 1;
                    let p = t_two_sided(t, df);
                    out.candidates.push(Influencer {
                        field,
                        what: What::Up { step },
                        value: b * step,
                        rest: all.mean(),
                        effect: b * step,
                        count: all.n,
                        p,
                    });
                }
                out.bars = bins
                    .iter()
                    .map(|(f, t, c)| Bar {
                        label: BarLabel::Range(*f, *t),
                        mean: c.mean(),
                        count: c.n,
                    })
                    .collect();
                return out;
            }
            // a class target: each bin is a category
            let mut cats = Vec::new();
            for (f, t, c) in &bins {
                out.bars.push(Bar {
                    label: BarLabel::Range(*f, *t),
                    mean: c.mean(),
                    count: c.n,
                });
                cats.push((*f, *t, *c));
            }
            let all = cats.iter().fold(Cell::default(), |a, (_, _, c)| a.merge(c));
            for (f, t, c) in cats {
                if let Some(i) = compare(field, What::Bin { from: f, to: t }, &c, &all, goal) {
                    out.candidates.push(i);
                }
                out.tests += 1;
            }
            return out;
        }
    };
    let all = categories.iter().fold(Cell::default(), |a, (_, c)| a.merge(c));
    for (k, c) in &categories {
        out.tests += 1;
        if let Some(i) = compare(field, What::Is(k.clone()), c, &all, goal) {
            out.candidates.push(i);
        }
    }
    let mut bars: Vec<Bar> = categories
        .iter()
        .filter(|(_, c)| c.n > 0.0)
        .map(|(k, c)| Bar {
            label: BarLabel::Category(k.clone()),
            mean: c.mean(),
            count: c.n,
        })
        .collect();
    // the goal's best first (ties: display order)
    let s = goal.sign();
    bars.sort_by(|a, b| (s * b.mean).total_cmp(&(s * a.mean)));
    out.bars = bars;
    out
}

/// A group against the other rows, when both have enough rows.
fn compare(field: usize, what: What, g: &Cell, all: &Cell, goal: Goal) -> Option<Influencer> {
    let rest = all.minus(g);
    if g.n < MIN_COUNT || rest.n < MIN_COUNT {
        return None;
    }
    let (_, p) = welch(g, &rest);
    let (value, other) = (g.mean(), rest.mean());
    let effect = match goal {
        Goal::Class => {
            // the class rate's ratio; a rest without the class counts half a row (no ∞)
            let floor = 0.5 / rest.n;
            value / other.max(floor)
        }
        _ => value - other,
    };
    Some(Influencer {
        field,
        what,
        value,
        rest: other,
        effect,
        count: g.n,
        p,
    })
}

/// Keeps the significant candidates that push the target the goal's way, best first (ties:
/// field, then input order).
pub(crate) fn rank(results: &[FieldResult], goal: Goal, max: usize) -> Vec<Influencer> {
    let tests = results.iter().map(|r| r.tests).sum::<usize>().max(1);
    let alpha = ALPHA / tests as f64;
    let mut kept: Vec<(usize, Influencer)> = Vec::new();
    for r in results {
        for c in &r.candidates {
            let good = match goal {
                Goal::Class => c.effect > 1.0,
                Goal::Increase => c.effect > 0.0,
                Goal::Decrease => c.effect < 0.0,
            };
            if good && c.p <= alpha && c.effect.is_finite() {
                kept.push((kept.len(), c.clone()));
            }
        }
    }
    // strength: the ratio for a class, else the size of the change (per step for numbers)
    let strength = |i: &Influencer| match goal {
        Goal::Class => i.effect,
        _ => i.effect.abs(),
    };
    kept.sort_by(|a, b| strength(&b.1).total_cmp(&strength(&a.1)).then(a.0.cmp(&b.0)));
    kept.into_iter().take(max).map(|(_, i)| i).collect()
}

// ---------------------------------------------------------------------------------------------
// top segments
// ---------------------------------------------------------------------------------------------

/// One key of a joint cell: a category or a number.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Key {
    Cat(String),
    Num(f64),
}

/// A condition of a segment.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Cond {
    Is(usize, String),
    IsNot(usize, String),
    AtMost(usize, f64),
    Above(usize, f64),
}

impl Cond {
    fn holds(&self, keys: &[Key]) -> bool {
        match self {
            Cond::Is(f, v) => matches!(keys.get(*f), Some(Key::Cat(k)) if k == v),
            Cond::IsNot(f, v) => !matches!(keys.get(*f), Some(Key::Cat(k)) if k == v),
            Cond::AtMost(f, t) => matches!(keys.get(*f), Some(Key::Num(x)) if x <= t),
            Cond::Above(f, t) => matches!(keys.get(*f), Some(Key::Num(x)) if x > t),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Segment {
    pub conds: Vec<Cond>,
    pub mean: f64,
    pub count: f64,
    pub share: f64,
}

/// The top segments: a regression tree over the joint cells (keys per field in `fields` order;
/// `edges[f]`: the thresholds a numeric field may split at), split on the largest drop of the
/// squared error, depth ≤ [`TREE_DEPTH`], segments ≥ [`MIN_SEGMENT`] rows and
/// [`MIN_SEGMENT_SHARE`] of them. The leaves the goal favours, best first.
pub(crate) fn segments(cells: &[(Vec<Key>, Cell)], edges: &[Vec<f64>], goal: Goal) -> Vec<Segment> {
    let all = cells.iter().fold(Cell::default(), |a, (_, c)| a.merge(c));
    if all.n <= 0.0 {
        return Vec::new();
    }
    let min_leaf = MIN_SEGMENT.max(all.n * MIN_SEGMENT_SHARE);
    let idx: Vec<usize> = (0..cells.len()).collect();
    let mut leaves: Vec<(Vec<Cond>, Cell)> = Vec::new();
    grow(cells, edges, &idx, Vec::new(), 0, min_leaf, &mut leaves);
    let s = goal.sign();
    let overall = all.mean();
    let mut out: Vec<Segment> = leaves
        .into_iter()
        .filter(|(conds, c)| !conds.is_empty() && s * (c.mean() - overall) > 0.0)
        .map(|(conds, c)| Segment {
            conds,
            mean: c.mean(),
            count: c.n,
            share: c.n / all.n,
        })
        .collect();
    out.sort_by(|a, b| (s * b.mean).total_cmp(&(s * a.mean)));
    out.truncate(MAX_SEGMENTS);
    out
}

fn grow(
    cells: &[(Vec<Key>, Cell)],
    edges: &[Vec<f64>],
    idx: &[usize],
    conds: Vec<Cond>,
    depth: usize,
    min_leaf: f64,
    leaves: &mut Vec<(Vec<Cond>, Cell)>,
) {
    let total = idx
        .iter()
        .filter_map(|i| cells.get(*i))
        .fold(Cell::default(), |a, (_, c)| a.merge(c));
    if depth >= TREE_DEPTH || leaves.len() >= 64 {
        leaves.push((conds, total));
        return;
    }
    let fields = cells.first().map_or(0, |(k, _)| k.len());
    let mut best: Option<(f64, Cond, Cond)> = None;
    let consider = |l: Cond, r: Cond, best: &mut Option<(f64, Cond, Cond)>| {
        let left = idx
            .iter()
            .filter_map(|i| cells.get(*i))
            .filter(|(k, _)| l.holds(k))
            .fold(Cell::default(), |a, (_, c)| a.merge(c));
        let right = total.minus(&left);
        if left.n < min_leaf || right.n < min_leaf {
            return;
        }
        let gain = total.ss - left.ss - right.ss;
        if gain > 1e-9 * total.ss.max(1e-300) && best.as_ref().is_none_or(|b| gain > b.0) {
            *best = Some((gain, l, r));
        }
    };
    for f in 0..fields {
        let numeric = cells.iter().any(|(k, _)| matches!(k.get(f), Some(Key::Num(_))));
        if numeric {
            for t in edges.get(f).map(Vec::as_slice).unwrap_or_default() {
                consider(Cond::AtMost(f, *t), Cond::Above(f, *t), &mut best);
            }
        } else {
            // the most frequent categories of this node, by count then name
            let mut counts: Vec<(String, f64)> = Vec::new();
            for (k, c) in idx.iter().filter_map(|i| cells.get(*i)) {
                if let Some(Key::Cat(v)) = k.get(f) {
                    if let Some(e) = counts.iter_mut().find(|(x, _)| x == v) {
                        e.1 += c.n;
                    } else if counts.len() < 10_000 {
                        counts.push((v.clone(), c.n));
                    }
                }
            }
            counts.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
            for (v, _) in counts.into_iter().take(SPLIT_CATEGORIES) {
                consider(Cond::Is(f, v.clone()), Cond::IsNot(f, v), &mut best);
            }
        }
    }
    match best {
        None => leaves.push((conds, total)),
        Some((_, l, r)) => {
            for side in [l, r] {
                let sub: Vec<usize> = idx
                    .iter()
                    .copied()
                    .filter(|i| cells.get(*i).is_some_and(|(k, _)| side.holds(k)))
                    .collect();
                let mut c = conds.clone();
                c.push(side);
                grow(cells, edges, &sub, c, depth + 1, min_leaf, leaves);
            }
        }
    }
}
