# Decomposition tree (black box): source log

Every source read or run for [decomposition-tree-bb.md](decomposition-tree-bb.md) (2026-11-04).
Power BI Desktop opens reports BIcraft generates; the rendered page is observed through its
WebView2 DevTools port (rendered DOM: text, element classes, ARIA labels, SVG attributes and
boxes) and driven with mouse events (`Input.dispatchMouseEvent`) on its own controls.

## 2026-11-04

| Kind | What | Use |
|---|---|---|
| public schema | `crates/pbir/schemas/visualConfiguration/2.7.0/schema.json` (vendored, MIT): `ExpansionState`, `RootExpansionState`, `NodeExpansionState`, `LevelExpansionState` (`isPinned`, `isLocked`, `AIInformation`), `AILevelInformation`, `AIDecompositionMethod` (`BestSplit`, `MaxSplit`, `MinSplit`) | the shape of the saved state |
| public schema | `crates/visuals/schemas/reportThemeSchema-2.157.json` (vendored, MIT), `visual-decompositionTreeVisual`: objects `tree`, `dataBars`, `dataLabels`, `categoryLabels`, `levelHeader`, `analysis`, `insights` and their enumerations (`density`, `connectorType`, `dataBarScalingType`, `aiMode`, `defaultClickAction`) | the formatting options |
| black-box | test `save`: a PBIX BIcraft generated (one tree, one column chart), Desktop 2.159.1231.0 (Store); the tree's "+" clicked, menu items clicked ("High value", a field, "Low value"), members clicked, Desktop's Save button clicked after each step; the PBIX Desktop saved unzipped and its `visual.json` read | the saved state, the AI re-choice on a path change, selection and highlighting of the other visual |
| black-box | tests D01–D14, then D15–D23 (`cargo xtask oracle-gen decomp`), one recording each with `xtask/decomposition-tree-oracle/record.ps1`; page captures kept outside the repository | values, fields, windows, geometry, AI choices, options |
| process | Desktop's version read from its package (`Get-AppxPackage`): 2.159.1231.0 | the version of every test above |
