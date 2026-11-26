# DAX query view tabs in a project

Sources: Microsoft Learn, "Power BI Desktop project semantic model folder"
(`learn.microsoft.com/power-bi/developer/projects/projects-dataset`, page dated 2026-09-23,
sections "DAXQueries\ folder" and ".pbi\daxQueries.json"); the shape of `daxQueries.json` read
from Desktop-saved PBIP projects published in public GitHub repositories (black box: files Desktop
wrote, not BIcraft's). No JSON schema for the file is published in `microsoft/json-schemas`
(`fabric/item/semanticModel` has none, checked 2026-11-23).

## Files

| Path (relative to `<Name>.SemanticModel/`) | Content | Label |
|---|---|---|
| `DAXQueries/<tab name>.dax` | one file per query tab of the DAX query view, the tab's text | Documented (Learn) |
| `DAXQueries/.pbi/daxQueries.json` | the view's settings: the tab order and the tab shown (`defaultTab`) | Documented (Learn); shape observed |

`daxQueries.json` as Desktop-saved projects hold it (two-space indentation, no trailing line
break):

```json
{
  "version": "1.0.0",
  "tabOrder": [
    "Query 1",
    "Query 2"
  ],
  "defaultTab": "Query 2"
}
```

`tabOrder` lists the tab names (the `.dax` file names without the extension) in tab order;
`defaultTab` names the tab shown. The `.dax` files seen are UTF-8 without BOM. Only `1.0.0` has
been seen as `version`.

Learn lists the folder among those a project "can include": it is optional, and a project
without it opens with no saved query.

## What BIcraft does

- `file.open` of a project reads the `.dax` files directly in `DAXQueries/` (not its sub-folders),
  decoded as text (a BOM is dropped), in `tabOrder`'s order, then the files it does not name by
  name; `defaultTab` when it names one of them. A missing or non-object settings file is no
  error; a settings file that is not JSON, or a `.dax` file that is not text, is (the tabs are
  then empty and the files left as they are). Caps: 1000 files, 16 MiB each, 64 MiB in all.
- `file.save` writes the tabs when they changed (`dax.set_queries`): each tab's text as UTF-8
  without BOM exactly as given (its own line endings, so the bytes are the same on Windows and
  Linux), the `.dax` files of tabs no longer there removed, and `daxQueries.json` with `tabOrder`
  and `defaultTab` (the first tab when none is named; no `defaultTab` without tabs) and `version`
  `1.0.0` when the file had none. The other members of an existing settings file keep their
  values and order. A file is replaced through a temporary sibling and a rename.
- `file.save_as` and `project.new` write the tabs into the new project.
- A tab name becomes a file name, so it must be one on Windows and Linux: no `< > : " / \ | ? *`
  or control character, no leading or trailing dot or space, not `CON`, `PRN`, `AUX`, `NUL`,
  `COM1`…`LPT9`, at most 200 characters, unique ignoring case. Desktop's own rules for tab names
  are not known.

## Not done

- A PBIX or PBIT: no public description says where a package keeps the DAX query view's tabs,
  so BIcraft keeps them for the session and says so when it saves (docs/gaps.md G81).
- A thin report's `<Name>.Report/DAXQueries/` (queries over a remote model) is not read.
- Desktop's own rules for tab names are not known.

## Desktop answers (2026-11-23, Power BI Desktop 2.159.1231.0, Store edition)

Black box: a project BIcraft made (`project.new`, three tabs set with `dax.set_queries`, the
second one named as `defaultTab`, `file.save`) opened in Desktop with its WebView2 DevTools port
used only to read the rendered page and send mouse and key input.

- The DAX query view lists the three tabs in `tabOrder`'s order and shows `defaultTab` ("Query 2
  of 3" in the status bar), with that file's text; the `DEFINE MEASURE` line gets Desktop's
  "Update model: Add new measure" lens. LF line endings and a `.dax` file without a final line
  break are accepted.
- Save without a change to the queries writes nothing in `DAXQueries/`.
- After another tab is chosen and its text edited, Save rewrites every `.dax` file with the same
  bytes for the unchanged tabs and the new text (no line break added) for the edited one, and
  `daxQueries.json` with the same members in the same order, `defaultTab` naming the tab shown
  — with CRLF line endings and no final line break (BIcraft writes LF, which Desktop reads; the
  bytes stay the same on Windows and Linux by design).
