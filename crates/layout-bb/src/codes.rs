//! The legacy layout stores enumerations as numbers; PBIR spells them. Each table lists only the
//! codes we have evidence for (black-box test ids `LO-*` and sources in
//! `docs/formats/legacy-layout-bb.sources.md`). A code outside a table is never guessed: the
//! property stays in the legacy unmapped map.

pub(crate) type Table = &'static [(i64, &'static str)];

pub(crate) fn name(table: Table, code: &serde_json::Value) -> Option<&'static str> {
    let c = code.as_i64().or_else(|| {
        code.as_f64()
            .filter(|f| f.fract() == 0.0 && f.abs() < 1.0e15)
            .map(|f| f as i64)
    })?;
    table.iter().find(|(k, _)| *k == c).map(|(_, n)| *n)
}

pub(crate) fn code(table: Table, name: &str) -> Option<i64> {
    table.iter().find(|(_, n)| *n == name).map(|(k, _)| *k)
}

/// Section `displayOption` (LO-pages: 0, 1, 2, 3 all observed).
pub(crate) const DISPLAY_OPTION: Table = &[
    (0, "DeprecatedDynamic"),
    (1, "FitToPage"),
    (2, "FitToWidth"),
    (3, "ActualSize"),
];

/// Section `config.visibility` (LO-pages: 1). 0 (always visible) is the absence of the PBIR
/// property and stays unmapped.
pub(crate) const VISIBILITY: Table = &[(1, "HiddenInViewMode")];

/// Section `config.type` (LO-pages: 1 tooltip, 2 drillthrough).
pub(crate) const PAGE_TYPE: Table = &[(1, "Tooltip"), (2, "Drillthrough")];

/// Section `config.relationships[].type` → `visualInteractions[].type` (LO-pages: 1, 2, 3). A
/// relationship of type 0 is not written by Desktop (LX-x1): it stays unmapped.
pub(crate) const INTERACTION: Table = &[(1, "DataFilter"), (2, "HighlightFilter"), (3, "NoFilter")];

/// Pod `type` → `pageBinding.type` (LO-pages: 1 drillthrough, 2 tooltip).
pub(crate) const BINDING_TYPE: Table = &[(1, "Drillthrough"), (2, "Tooltip")];

/// Pod `referenceScope` (LO-pages: 1).
pub(crate) const REFERENCE_SCOPE: Table = &[(1, "CrossReport")];

/// Pod `config.acceptsFilterContext` (LO-pages: 1).
pub(crate) const ACCEPTS_FILTER_CONTEXT: Table = &[(1, "None")];

/// Filter `howCreated` (LO-filters: 1, 2, 3, 4; LO-pages: 5). 0 (`Auto`) is written by Desktop as
/// no `howCreated` at all (LO-filters, `Filter_vagg`).
pub(crate) const HOW_CREATED: Table = &[
    (1, "User"),
    (2, "Drill"),
    (3, "Include"),
    (4, "Exclude"),
    (5, "Drillthrough"),
];

/// `filterSortOrder` of the report, a page or a visual (LO-filters: 1, 2, 3).
pub(crate) const FILTER_SORT_ORDER: Table = &[(1, "Ascending"), (2, "Descending"), (3, "Custom")];

/// `prototypeQuery.OrderBy[].Direction` (LO-visuals: 1, 2).
pub(crate) const SORT_DIRECTION: Table = &[(1, "Ascending"), (2, "Descending")];

/// `singleVisualGroup.groupMode` (LO-groups: 0, 1).
pub(crate) const GROUP_MODE: Table = &[(0, "ScaleMode"), (1, "ScrollMode")];

/// Resource package `type` (LO-report: 1, 2).
pub(crate) const PACKAGE_TYPE: Table = &[(1, "RegisteredResources"), (2, "SharedResources")];

/// Resource package item `type` (LO-report: 100, 201, 202).
pub(crate) const ITEM_TYPE: Table = &[(100, "Image"), (201, "CustomTheme"), (202, "BaseTheme")];

/// Theme `type` in `config.themeCollection` (LO-report: 1, 2).
pub(crate) const THEME_TYPE: Table = &[(1, "RegisteredResources"), (2, "SharedResources")];

/// `config.settings.exportDataMode` (LO-report: 1; LX-x1: 2). Desktop writes nothing for 0
/// (LX-x2), which stays unmapped.
pub(crate) const EXPORT_DATA_MODE: Table = &[(1, "AllowSummarized"), (2, "None")];

/// `config.settings.queryLimitOption` (LX-x1: 0; LX-x2: 3; both the public schema's order). Desktop
/// writes nothing for 6 (LO-report), which stays unmapped.
pub(crate) const QUERY_LIMIT_OPTION: Table = &[(0, "None"), (3, "SQLServerAS")];

/// `config.settings.pagesPosition` (LO-report: 1). Desktop writes nothing for 0 (LX-x1).
pub(crate) const PAGES_POSITION: Table = &[(1, "Bottom")];

/// Report-level measure `dataType` (`config.modelExtensions`): the `PrimitiveType` enumeration of
/// microsoft/powerbi-visuals-utils-typeutils (MIT), `src/valueType.ts`; 3 confirmed by LO-report.
pub(crate) const DATA_TYPE: Table = &[
    (0, "Null"),
    (1, "Text"),
    (2, "Decimal"),
    (3, "Double"),
    (4, "Integer"),
    (5, "Boolean"),
    (6, "Date"),
    (7, "DateTime"),
    (8, "DateTimeZone"),
    (9, "Time"),
    (10, "Duration"),
    (11, "Binary"),
    (12, "None"),
    (13, "Variant"),
];

/// The legacy theme `version` → PBIR `reportVersionAtImport` (`visual`, `report`, `page`):
/// LX-x1 (5.43, 5.80), LO-report (5.61), LX-x2 (5.70, 6.10), LO-groups (5.99). From 5.80 on, Desktop
/// writes the versions it supports itself: 2.158 wrote 2.13.0 for 5.99 (LO), 2.159.883.0 writes 2.14.0
/// (LX, and the committed `groups` oracle pair, 5.99); the table follows the current Desktop.
pub(crate) const THEME_VERSIONS: &[(&str, [&str; 3])] = &[
    ("5.43", ["1.8.79", "2.0.79", "1.3.79"]),
    ("5.61", ["1.8.97", "2.0.97", "1.3.97"]),
    ("5.70", ["2.6.0", "3.1.0", "2.3.0"]),
    ("5.80", ["2.14.0", "3.4.0", "2.3.1"]),
    ("5.99", ["2.14.0", "3.4.0", "2.3.1"]),
    ("6.10", ["2.14.0", "3.4.0", "2.3.1"]),
];

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tables_are_bijective() {
        for t in [
            DISPLAY_OPTION,
            VISIBILITY,
            PAGE_TYPE,
            INTERACTION,
            BINDING_TYPE,
            REFERENCE_SCOPE,
            ACCEPTS_FILTER_CONTEXT,
            HOW_CREATED,
            FILTER_SORT_ORDER,
            SORT_DIRECTION,
            GROUP_MODE,
            PACKAGE_TYPE,
            ITEM_TYPE,
            THEME_TYPE,
            EXPORT_DATA_MODE,
            PAGES_POSITION,
            QUERY_LIMIT_OPTION,
            DATA_TYPE,
        ] {
            for (c, n) in t {
                assert_eq!(code(t, n), Some(*c));
                assert_eq!(name(t, &json!(c)), Some(*n));
                assert_eq!(name(t, &json!(*c as f64)), Some(*n));
            }
        }
        assert_eq!(name(HOW_CREATED, &json!(0)), None);
        assert_eq!(name(HOW_CREATED, &json!("1")), None);
    }
}
