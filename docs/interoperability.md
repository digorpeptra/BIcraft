# Interoperability

BIcraft is an interoperability implementation: it reads and writes the files that Power BI
Desktop opens and saves, so that people and agents can work with them outside Desktop. It is an
independent open-source project, not affiliated with, endorsed by or sponsored by Microsoft.

## Sources

BIcraft builds on:

- **Microsoft's public specifications**: `[MS-XLDM]` and the other Microsoft Open
  Specifications. Microsoft offers patent promises for the Open Specifications, the Open
  Specification Promise and the Microsoft Community Promise, each covering implementations of
  the specifications it lists, on the terms stated on Microsoft's pages.
- **Microsoft's published schemas and documentation**: the PBIR/PBIP JSON schemas (each file's
  `$schema` URL), the report theme schema, and the TMDL, TMSL and Power BI documentation on
  Microsoft Learn; also the SQLite file format.
- **Open-source code and format descriptions under their licences** (MIT, Apache-2.0,
  BSD-3-Clause, CC0, the SIL Open Font License and others), each use listed in
  [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) and [ATTRIBUTION.md](../ATTRIBUTION.md). The XPress9 codec (`crates/xpress9`) is a Rust port of
  the C source Microsoft published under the MIT licence in
  [microsoft/Extensible-Storage-Engine](https://github.com/microsoft/Extensible-Storage-Engine);
  each ported file names its upstream file and commit. The PBIR and report theme schemas, the
  best-practice rules and the Selawik fonts are Microsoft releases vendored unmodified with
  their licences; the CC0 pbixray Kaitai specs describe parts of the PBIX data model. The renderer's
  value-axis steps, category bands and treemap layout (`crates/render`) are attributed ports of
  d3 and d3-array (BSD-3-Clause).
- **Black-box testing** with Power BI Desktop (below).

What the project knows of each format is written down in [formats/](formats/), one note per
format, each with a line naming its sources.

## Black-box testing

BIcraft's own synthetic files are opened in Power BI Desktop, and the files Desktop writes from
our synthetic models are read back. Each observation records the Desktop version it was made
with. During development, DAX results and engine-written files are also checked against
Microsoft's Analysis Services engine on our own models.

- The scripts that drive Desktop as installed (`tools/layout-oracle`, `tools/render-oracle`,
  `tools/packaging-oracle`, the oracles under `xtask/`) drive it through UI Automation, input
  events, posted window messages, and the WebView2 DevTools protocol (DOM queries and input
  events), including the DOM of its dialogs.
- They observe what Desktop shows on its rendered page (text, DOM, styles, geometry and
  accessibility attributes, the drawn SVG, screenshots), what its engine answers (the queries
  Desktop sends to it, through a server trace with the public AMO client library, and the
  versions the server reports), and the files it writes: from our synthetic models, and the
  structure of Desktop-saved files held locally, which are never committed or quoted.
- The development oracles that use Microsoft's engine are kept by the maintainers outside this
  repository. Their results are stored here as our own data (verdict classes, expected values,
  fixtures made from our own models); public tests read that data and never need Windows,
  Desktop or the engine.
- BIcraft does not ship, embed or require any part of Power BI Desktop. It never loads a
  Power BI or Analysis Services library or connects to Desktop at run time, and this repository
  holds no Desktop file.

## Rules for contributors

- **No analysis of vendor binaries.** Do not disassemble or decompile Microsoft software, read
  the CIL or metadata of its assemblies, scan its binaries, call its non-public types by
  reflection, or read Desktop's script bundles as code. Desktop is used only as a black box.
- **Never paste Microsoft code or documentation prose** into the source. Interoperability
  constants (part names, property names, magic numbers, the XPress9 banner, error reason codes)
  are fine, and so are the error messages below.
- **Write each finding as a note** under [formats/](formats/) and implement from the note. Cite
  the public source (specification section, schema URL, open-source file and commit) or the
  black-box observation (Desktop version, what was opened or saved). Detailed provenance is kept
  privately by the maintainers and never committed.
- **Commit no Microsoft assets** and no Power BI files authored by others. Test fixtures are files
  the project generates itself; real Power BI files stay local.
- **Never describe BIcraft as a clean-room implementation**, nor as written from scratch.

## Performance figures

The Power BI Desktop licence does not allow disclosing its benchmark results. The project's docs
report BIcraft's own measurements and its agreement with Desktop (the same values, Desktop opens
the file), never Desktop timings or ratios against Desktop. `cargo xtask docs`, part of
`cargo xtask ci`, fails on a committed Markdown line that puts a duration, a ratio or a speed
comparison next to Desktop.

## Power Query error messages

BIcraft's Power Query evaluator (`crates/data/src/m`) returns errors with the same reason codes
(`Expression.Error`, `DataFormat.Error`, …) and the same short messages as Power Query, for
example "The key didn't match any rows in the table.". They are observed by running M queries
and reading the errors. They are reproduced on purpose: M code can test an error's `[Message]`,
so a different text would change what a query returns. Other strings that must match what
Desktop writes or shows follow [formats/locales.md](formats/locales.md); beyond these, BIcraft's
own source reproduces no Microsoft text.

## Trademarks

Power BI, Power BI Desktop, Analysis Services, Excel and Microsoft are trademarks of the
Microsoft group of companies. BIcraft names them only to say which products and file formats it
works with. It uses no Microsoft logos or icons and describes its own features in its own words.
