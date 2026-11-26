# INFO function probes (maintainers' checklist)

BIcraft implements the `INFO.*` functions from Microsoft's public DAX reference
(learn.microsoft.com/dax, "INFO functions" and each function's page): which roles may call them
and which columns they return. These were **documented, not measured**. To measure them, a
maintainer with Power BI Desktop runs the probes of `probes.json` with the development oracles
(`BICRAFT_ORACLES_DIR`, outside this repository; `docs/dax-oracle.md`) or by hand:

1. Open the oracle model (`crates/dax/tests/oracle/model.bim`, saved as a PBIX with data) in
   Desktop. Record the Desktop version in `probes.json` (`desktop_version`).
2. For each `measure`, `column`, `table` and `rls` probe: add the object, note whether Desktop
   accepts it (no error triangle, the object computes) or the error class
   (`docs/dax-oracle.md`, "Verdict semantics"). Never copy Desktop's message text.
3. For each `query` probe: run it in DAX query view; note the column names and types in order,
   and for `INFO.VIEW.*` the text values of `DataType`, `StorageMode`, `Type`, `SummarizeBy`,
   `Alignment`, `CrossFilteringBehavior`, `FromCardinality`, `State` and the shape of
   `Relationship`, `Model`, `TableStorage` and `ColumnStorage`.
4. Compare with `docs/formats/dax-functions.md`, "INFO functions": turn each cell measured into
   "measured (Desktop x.y)", fix the code (`crates/engine/src/preview_eval_info.rs`,
   `crates/dax/src/typecheck_rules.rs`) where Desktop differs, and add the verdicts to the
   tests (`refresh_table_tests.rs`, `tests_rules.rs`).

No Desktop timing is recorded.
