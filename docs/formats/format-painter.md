# Format painter

Sources: Microsoft's Power BI blog post on the December 2015 Power BI Desktop update ("More Power
BI feature updates: Power BI Desktop December update", section "Format Painter"), read through
the Internet Archive copy because the live page refuses automated reads; Microsoft Learn's
monthly update archive (September 2026: Format Painter in the report ribbon on the web); the
published report theme schema 2.159 (`crates/visuals/schemas/`) for the objects each visual type
has. No Desktop run.

## What Microsoft documents

- Use: select the visual that has the formatting, click **Format Painter** on the **Home** tab,
  click the visual that should get it.
- Copied: all **Background** options; **Title** options except the text itself; **Data labels**
  except display units and decimal places; **Data colors**: the default colour and, when the same
  field drives the colour, the colour of each value; **X-axis** and **Y-axis** options except
  start and end; **Legend** options except its text; all **Border** options; all **Slicer**
  options; the waterfall's sentiment colours.
- Options the target visual does not have are dropped (the post's example: a scatter chart's
  X-axis options copied onto a column chart).

Nothing Microsoft publishes says that a double click keeps the painter on (a Fabric Ideas
request asks for that "lock-on" behaviour), so BIcraft does not do it.

## BIcraft's mapping (`visual.copy_format`)

The 2015 list names the cards of that time; BIcraft applies the same rule to the PBIR objects of
today. For each formatting object of the source (`visual.objects` and
`visual.visualContainerObjects`):

| Rule | PBIR |
|---|---|
| Not formatting: stays the target's | container objects `general` (position, size, alt text), `visualLink` (the action), `lockAspect` |
| Kept by the target | `title.text`, `subTitle.text`, `labels.labelDisplayUnits`, `labels.labelPrecision`, `categoryAxis.start` / `end`, `valueAxis.start` / `end` / `secStart` / `secEnd`, `legend.titleText` |
| Dropped when the target type lacks it | an object or property the target's catalogued type (report theme schema) does not have; for a type outside the catalog (custom visuals), only the container objects are copied |
| Field-scoped entries | an entry whose selector names data (`data`, `metadata`) is copied only when the target binds every field it names (the "same field drives the colour" rule, extended to per-series labels); the target keeps its own field-scoped entries unless the source has one for the same selector |
| Everything else | the source's value replaces the target's; a property the source leaves at its default is removed from the target, so the target looks like the source |

Fields, filters, sort, size and position are never touched. One call paints several targets
and is one undo step. The source may be on another page (`to_page`).

## In the app

Home › Clipboard › **Format painter** is a toggle: with a visual selected it turns on (the
pointer shows a brush), the next click on a visual applies the formatting and turns it off. A
click on the empty page, a right click, Esc or a second click on the button turn it off.

## Not verified

The PBIR mapping is not checked against a Desktop run: whether Desktop's painter also resets
properties the source leaves at default, and how it treats modern objects the 2015 list does
not name (shadow, header icons, tooltips, spacing), are BIcraft's reading of the rule.
