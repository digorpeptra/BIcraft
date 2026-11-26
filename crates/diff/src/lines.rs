//! Line diff of two texts (expressions before and after), Myers' O((n+m)·D) algorithm with a
//! cap on D: past [`MAX_EDITS`] edits, or past [`MAX_LINES`] lines, the middle is shown as
//! removed then added. The common prefix and suffix are stripped first, so a small edit in a
//! long expression costs little.

/// Most lines diffed per side (after stripping the common prefix and suffix).
pub const MAX_LINES: usize = 50_000;
/// Most edits searched for; the trace kept for backtracking is O(MAX_EDITS²).
pub const MAX_EDITS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Line<'a> {
    Same(&'a str),
    Del(&'a str),
    Ins(&'a str),
}

/// The lines of `a` turned into the lines of `b`.
pub fn diff<'a>(a: &'a str, b: &'a str) -> Vec<Line<'a>> {
    let la: Vec<&str> = a.lines().collect();
    let lb: Vec<&str> = b.lines().collect();
    let prefix = la.iter().zip(&lb).take_while(|(x, y)| x == y).count();
    let rest_a = la.get(prefix..).unwrap_or_default();
    let rest_b = lb.get(prefix..).unwrap_or_default();
    let suffix = rest_a
        .iter()
        .rev()
        .zip(rest_b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let mid_a = rest_a
        .get(..rest_a.len().saturating_sub(suffix))
        .unwrap_or_default();
    let mid_b = rest_b
        .get(..rest_b.len().saturating_sub(suffix))
        .unwrap_or_default();
    let mut out: Vec<Line<'a>> = la.iter().take(prefix).map(|l| Line::Same(l)).collect();
    let middle = if mid_a.len() <= MAX_LINES && mid_b.len() <= MAX_LINES {
        myers(mid_a, mid_b, MAX_EDITS)
    } else {
        None
    };
    match middle {
        Some(m) => out.extend(m),
        None => {
            out.extend(mid_a.iter().map(|l| Line::Del(l)));
            out.extend(mid_b.iter().map(|l| Line::Ins(l)));
        }
    }
    out.extend(rest_a.iter().skip(mid_a.len()).map(|l| Line::Same(l)));
    out
}

/// One row of the trace: `v[k]` for `k` in `-(d+1)..=d+1`.
struct Row {
    d: i64,
    v: Vec<i32>,
}

impl Row {
    fn get(&self, k: i64) -> i64 {
        usize::try_from(k + self.d + 1)
            .ok()
            .and_then(|i| self.v.get(i))
            .map_or(0, |&x| i64::from(x))
    }
}

fn myers<'a>(a: &[&'a str], b: &[&'a str], max_d: usize) -> Option<Vec<Line<'a>>> {
    let n = i64::try_from(a.len()).ok()?;
    let m = i64::try_from(b.len()).ok()?;
    let max = i64::try_from(max_d).ok()?.min(n + m);
    let off = max + 1;
    let width = usize::try_from(2 * max + 3).ok()?;
    let mut v = vec![0i64; width];
    let at = |k: i64| usize::try_from(k + off).ok();
    let get = |v: &[i64], k: i64| at(k).and_then(|i| v.get(i)).copied().unwrap_or(0);
    let mut trace: Vec<Row> = Vec::new();
    for d in 0..=max {
        // v as it was after step d - 1, on the diagonals step d can read
        trace.push(Row {
            d,
            v: (-(d + 1)..=d + 1)
                .map(|k| i32::try_from(get(&v, k)).unwrap_or(i32::MAX))
                .collect(),
        });
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && get(&v, k - 1) < get(&v, k + 1)) {
                get(&v, k + 1)
            } else {
                get(&v, k - 1) + 1
            };
            let mut y = x - k;
            while x < n && y < m && a.get(x as usize) == b.get(y as usize) {
                x += 1;
                y += 1;
            }
            if let Some(slot) = at(k).and_then(|i| v.get_mut(i)) {
                *slot = x;
            }
            if x >= n && y >= m {
                return Some(backtrack(&trace, a, b, n, m));
            }
            k += 2;
        }
    }
    None
}

fn backtrack<'a>(trace: &[Row], a: &[&'a str], b: &[&'a str], n: i64, m: i64) -> Vec<Line<'a>> {
    let (mut x, mut y) = (n, m);
    let mut rev = Vec::new();
    let line = |s: &[&'a str], i: i64| {
        usize::try_from(i)
            .ok()
            .and_then(|i| s.get(i))
            .copied()
            .unwrap_or("")
    };
    for row in trace.iter().rev() {
        let d = row.d;
        let k = x - y;
        let prev_k = if k == -d || (k != d && row.get(k - 1) < row.get(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = row.get(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y && x > 0 && y > 0 {
            rev.push(Line::Same(line(a, x - 1)));
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            if x == prev_x && y > 0 {
                rev.push(Line::Ins(line(b, y - 1)));
            } else if x > 0 {
                rev.push(Line::Del(line(a, x - 1)));
            }
        }
        x = prev_x;
        y = prev_y;
    }
    rev.reverse();
    rev
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply<'a>(d: &[Line<'a>]) -> (Vec<&'a str>, Vec<&'a str>) {
        let mut a = Vec::new();
        let mut b = Vec::new();
        for l in d {
            match *l {
                Line::Same(s) => {
                    a.push(s);
                    b.push(s);
                }
                Line::Del(s) => a.push(s),
                Line::Ins(s) => b.push(s),
            }
        }
        (a, b)
    }

    #[test]
    fn small_edits() {
        let a = "VAR x =\n  1\nVAR y = 2\nRETURN\n  x + y";
        let b = "VAR x =\n  10\nVAR y = 2\nVAR z = 3\nRETURN\n  x + y + z";
        let d = diff(a, b);
        let (ra, rb) = apply(&d);
        assert_eq!(ra, a.lines().collect::<Vec<_>>());
        assert_eq!(rb, b.lines().collect::<Vec<_>>());
        let edits = d.iter().filter(|l| !matches!(l, Line::Same(_))).count();
        assert_eq!(edits, 5, "{d:?}");
    }

    #[test]
    fn reconstructs_both_sides_on_pseudo_random_texts() {
        // a small deterministic generator: no external crate needed
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..200 {
            let gen_text = |next: &mut dyn FnMut() -> u64| -> String {
                let n = next() % 30;
                (0..n)
                    .map(|_| format!("l{}", next() % 5))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            let a = gen_text(&mut next);
            let b = gen_text(&mut next);
            let d = diff(&a, &b);
            let (ra, rb) = apply(&d);
            assert_eq!(ra, a.lines().collect::<Vec<_>>(), "{a:?} {b:?}");
            assert_eq!(rb, b.lines().collect::<Vec<_>>(), "{a:?} {b:?}");
        }
    }

    #[test]
    fn huge_inputs_fall_back_quickly() {
        let a: String = (0..200_000).map(|i| format!("a{i}\n")).collect();
        let b: String = (0..200_000).map(|i| format!("b{i}\n")).collect();
        let start = std::time::Instant::now();
        let d = diff(&a, &b);
        assert_eq!(d.len(), 400_000);
        assert!(start.elapsed().as_secs() < 20);
        // many scattered edits: past MAX_EDITS, removed then added
        let a: String = (0..5_000).map(|i| format!("x{i}\n")).collect();
        let b: String = (0..5_000)
            .map(|i| {
                if i % 2 == 0 {
                    format!("x{i}\n")
                } else {
                    format!("y{i}\n")
                }
            })
            .collect();
        let d = diff(&a, &b);
        let (ra, rb) = apply(&d);
        assert_eq!((ra.len(), rb.len()), (5_000, 5_000));
    }
}
