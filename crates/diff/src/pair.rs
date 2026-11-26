//! Matching the objects of two collections: by name, then lineage tag, then case-insensitive
//! name, then content signature. Each step only pairs what is still unmatched, and a tag or
//! signature only when it is unique on both sides, so the result never depends on order
//! beyond "first of duplicate names wins".

use std::collections::{BTreeMap, HashMap};

use crate::How;

/// What identifies one object.
pub(crate) struct Item<'a> {
    pub name: &'a str,
    pub tag: Option<&'a str>,
    /// Equal for objects with the same content (e.g. the normalised expression); `None`: no
    /// content matching.
    pub sig: Option<String>,
}

pub(crate) struct Pairing {
    /// `(index in a, index in b, how)`, in `a` order.
    pub pairs: Vec<(usize, usize, How)>,
    pub removed: Vec<usize>,
    pub added: Vec<usize>,
}

/// Gives a closure that lists a collection's [`Item`]s the signature it needs (items borrow from
/// the collection), so the same closure serves both sides.
pub(crate) fn items_fn<'a, T: ?Sized + 'a, F: Fn(&'a T) -> Vec<Item<'a>>>(f: F) -> F {
    f
}

/// [`items_fn`] for a closure of two arguments.
pub(crate) fn items_fn2<'a, T: ?Sized + 'a, U: ?Sized + 'a, F: Fn(&'a T, &'a U) -> Vec<Item<'a>>>(f: F) -> F {
    f
}

fn lower(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).collect()
}

pub(crate) fn pair(a: &[Item<'_>], b: &[Item<'_>]) -> Pairing {
    let mut a_used = vec![false; a.len()];
    let mut b_used = vec![false; b.len()];
    let mut pairs = Vec::new();
    let mut take = |ia: usize, ib: usize, how: How, a_used: &mut [bool], b_used: &mut [bool]| {
        if let (Some(x), Some(y)) = (a_used.get_mut(ia), b_used.get_mut(ib)) {
            *x = true;
            *y = true;
            pairs.push((ia, ib, how));
        }
    };
    // 1. exact name (first of duplicates)
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    for (i, it) in b.iter().enumerate() {
        by_name.entry(it.name).or_insert(i);
    }
    for (ia, it) in a.iter().enumerate() {
        if let Some(&ib) = by_name.get(it.name)
            && !b_used.get(ib).copied().unwrap_or(true)
        {
            take(ia, ib, How::Name, &mut a_used, &mut b_used);
        }
    }
    // 2. lineage tag, 3. case-insensitive name, 4. content: unique among the unmatched
    type Key<'k> = &'k dyn Fn(&Item<'_>) -> Option<String>;
    let steps: [(How, Key<'_>); 3] = [
        (How::LineageTag, &|it: &Item<'_>| {
            it.tag.filter(|t| !t.is_empty()).map(str::to_string)
        }),
        (How::Case, &|it: &Item<'_>| Some(lower(it.name))),
        (How::Content, &|it: &Item<'_>| {
            it.sig.clone().filter(|s| !s.is_empty())
        }),
    ];
    for (how, key) in steps {
        let unique = |items: &[Item<'_>], used: &[bool]| -> BTreeMap<String, Option<usize>> {
            let mut m: BTreeMap<String, Option<usize>> = BTreeMap::new();
            for (i, it) in items.iter().enumerate() {
                if used.get(i).copied().unwrap_or(true) {
                    continue;
                }
                if let Some(k) = key(it) {
                    m.entry(k).and_modify(|v| *v = None).or_insert(Some(i));
                }
            }
            m
        };
        let ua = unique(a, &a_used);
        let ub = unique(b, &b_used);
        for (k, ia) in &ua {
            if let (Some(ia), Some(Some(ib))) = (ia, ub.get(k)) {
                take(*ia, *ib, how, &mut a_used, &mut b_used);
            }
        }
    }
    pairs.sort_by_key(|p| p.0);
    Pairing {
        pairs,
        removed: (0..a.len())
            .filter(|&i| !a_used.get(i).copied().unwrap_or(true))
            .collect(),
        added: (0..b.len())
            .filter(|&i| !b_used.get(i).copied().unwrap_or(true))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn it<'a>(name: &'a str, tag: Option<&'a str>, sig: Option<&str>) -> Item<'a> {
        Item {
            name,
            tag,
            sig: sig.map(str::to_string),
        }
    }

    #[test]
    fn pairs_by_name_tag_case_and_content() {
        let a = [
            it("Keep", None, None),
            it("Old", Some("t1"), None),
            it("lower", None, None),
            it("Gone", None, Some("x")),
            it("Removed", None, None),
        ];
        let b = [
            it("New", Some("t1"), None),
            it("Keep", None, None),
            it("LOWER", None, None),
            it("Moved", None, Some("x")),
            it("Added", None, None),
        ];
        let p = pair(&a, &b);
        assert_eq!(
            p.pairs,
            [
                (0, 1, How::Name),
                (1, 0, How::LineageTag),
                (2, 2, How::Case),
                (3, 3, How::Content)
            ]
        );
        assert_eq!(p.removed, [4]);
        assert_eq!(p.added, [4]);
    }

    #[test]
    fn ambiguous_tags_and_signatures_do_not_pair() {
        let a = [it("A", Some("t"), Some("s")), it("B", Some("t"), Some("s"))];
        let b = [it("C", Some("t"), Some("s"))];
        let p = pair(&a, &b);
        assert!(p.pairs.is_empty());
        assert_eq!((p.removed.len(), p.added.len()), (2, 1));
    }
}
