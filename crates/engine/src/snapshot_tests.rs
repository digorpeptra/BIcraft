//! The snapshot test that needs the engine's private items (`refs`); the others are the `report`
//! integration test (`tests/report/snapshot_tests.rs`), under the same module path.

use serde_json::json;

#[test]
fn find_refs_sees_what_visit_sees() {
    let doc = json!({
        "query": {"From": [{"Name": "s", "Entity": "Sales"}],
                  "Select": [{"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Amount"}},
                             {"Measure": {"Expression": {"SourceRef": {"Entity": "Sales"}}, "Property": "Total"}},
                             {"Measure": {"Expression": {"SourceRef": {"Entity": "R", "Schema": "extension"}}, "Property": "X"}}]},
        "objects": [{"PropertyVariationSource": {"Expression": {"SourceRef": {"Entity": "Date"}}, "Property": "Day"}}]
    });
    let key = |site: &crate::refs::Site<'_>| {
        format!(
            "{:?} {} {} {} {}",
            site.kind, site.entity, site.property, site.direct, site.extension
        )
    };
    let mut seen_visit = Vec::new();
    crate::refs::visit(&mut doc.clone(), &mut |site| {
        seen_visit.push(key(site));
        crate::refs::Action::Keep
    });
    let mut seen_find = Vec::new();
    crate::refs::find_refs(&doc, &mut |site| seen_find.push(key(site)));
    assert_eq!(seen_find, seen_visit);
    assert_eq!(seen_find.len(), 8, "{seen_find:?}");
}
