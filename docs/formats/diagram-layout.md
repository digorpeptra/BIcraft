# Model diagram layout (`DiagramLayout`, `diagramLayout.json`)

Sources: Microsoft Learn's PBIP documentation (the `diagramLayout.json` file of the semantic model
folder), BIcraft's own PBIT writer (`crates/pbixwriter`, docs/formats/pbit.md), and the member
names and value types of `diagramLayout.json` files that Power BI Desktop saved in PBIP projects
kept locally (black-box reading of Desktop's output; the Desktop version of those saves was not
recorded).

## Where it lives

| Container | Location | Encoding |
|---|---|---|
| PBIX, PBIT | part `DiagramLayout` | UTF-16LE, no BOM (docs/formats/pbix-package.md) |
| PBIP | `<Name>.SemanticModel/diagramLayout.json` | UTF-8, no BOM |

The part and the file are optional: Desktop lays the tables out itself when they are missing or
empty (docs/formats/packaging-verification.md, PK-OPTIONAL).

## Shape

```json
{
  "version": "1.1.0",
  "diagrams": [
    {
      "ordinal": 0,
      "scrollPosition": {"x": 0, "y": 0},
      "nodes": [
        {
          "location": {"x": 412.5, "y": 96},
          "nodeIndex": "Sales",
          "nodeLineageTag": "…",
          "size": {"height": 300, "width": 234},
          "zIndex": 0
        }
      ],
      "name": "All tables",
      "zoomValue": 100,
      "pinKeyFieldsToTop": false,
      "showExtraHeaderInfo": false,
      "hideKeyFieldsWhenCollapsed": false,
      "tablesLocked": false
    }
  ],
  "selectedDiagram": "All tables",
  "defaultDiagram": "All tables"
}
```

| Member | Observed |
|---|---|
| `diagrams[]` | one entry per layout tab of the Model view; `name` is the tab's name (Desktop localizes the default one) |
| `selectedDiagram`, `defaultDiagram` | diagram names |
| `nodes[].nodeIndex` | the table's name |
| `nodes[].nodeLineageTag` | the table's `lineageTag` |
| `nodes[].location` | the card's top-left corner; `x` and `y` are integers or decimals |
| `nodes[].size` | integers; every observed `width` is 234 |
| `nodes[].zIndex` | integer (0 in the observed files) |
| `zoomValue` | integer or decimal (percent) |

## What BIcraft does

- The Model view places a table's card at its node's `location` in the selected diagram
  (`selectedDiagram`, else `defaultDiagram`, else the first); tables without a node keep the
  automatic layered layout, to the right of the placed cards.
- `diagram.move {table, x, y, diagram?}` sets the node's `location`, or adds a node
  (`location`, `nodeIndex`, `nodeLineageTag` when the table has one, `size` 234 × 300, `zIndex`
  0) when the diagram has none for the table. Every other member keeps its value and place. A
  file without a layout gets the PBIT writer's empty one (`All tables`) first.
- `file.save` writes the edited layout back: the `DiagramLayout` part of a PBIX/PBIT (UTF-16LE,
  its compression kept) or the project's `diagramLayout.json`. `file.save_as` and `file.convert`
  carry it over. A `model.bim` has no place for it.
- Layout tabs. BIcraft takes the diagram whose `ordinal` is 0 (else the first) as the layout of
  all tables: the Model view draws every table there, and only the tables with a node in the
  other diagrams. `diagram.add {name, tables?}` appends a diagram (`ordinal` one more than the
  highest, the members of the PBIT writer's empty diagram, the listed tables placed in rows of
  four) and makes it `selectedDiagram`; `diagram.rename {name, new_name}` renames one, and
  `selectedDiagram` / `defaultDiagram` follow; `diagram.remove {name}` deletes one other than the
  layout of all tables, and `selectedDiagram` / `defaultDiagram` that named it name that layout;
  `diagram.remove_tables {tables, diagram?}` takes tables' nodes out of a layout other than the
  layout of all tables (the tables stay in the model).
  Names are unique ignoring case. Which tab the Model view shows is view state: switching tabs
  writes nothing.
- `table.rename` renames the table's `nodeIndex` in every diagram, in the same undo step.

## Unchecked

- That Desktop's layout of all tables is the diagram with `ordinal` 0 (observed files have their
  `All tables` diagram first, with ordinal 0), whether Desktop lets that one be deleted or
  renamed (BIcraft refuses the deletion, allows the rename), and whether it stores the tab order
  in `ordinal` or in the list's order.
- Whether Desktop writes `selectedDiagram` when a tab is switched (BIcraft writes it only when a
  layout is added, renamed or deleted).
- The height of a new node (300): Desktop has not been seen opening a BIcraft-moved layout yet.
- Whether Desktop requires `nodeLineageTag` or a `size` matching the card's rows.
- What Desktop does with a node whose `nodeIndex` names no table (BIcraft leaves such nodes).
