# Model objects authored from the Model view

Sources: Microsoft Learn ("Create and manage relationships in Power BI Desktop", the Autodetect,
manual creation and edit sections; "Object-level security (OLS)"), the public TMSL and TMDL references (roles,
perspectives), and BIcraft's own synthetic models. Nothing here
was compared with a file Power BI Desktop saved; each section lists what is unchecked.

## Autodetect relationships

`relationship.autodetect` (in BIcraft, the Model explorer's Relationships menu). Microsoft Learn
says Desktop looks at the column names of the tables, sets cardinality, cross filter direction and "make this
relationship active" from the data, and creates no relationship it cannot match with a high
level of confidence; at least one side of a relationship must hold unique values. BIcraft's
rules, applied in model order (tables, then columns):

| Rule | BIcraft |
|---|---|
| Match | Same column name (ignoring case) and same data type, in two different tables not related yet; calculation group tables are left out |
| One per pair | The first match of a pair of tables; the others are not proposed (a second relationship would be inactive) |
| Cardinality | Read from the session's data: one unique, non-blank side gives many-to-one (`fromCardinality` absent, the "one" side as `to`); both unique gives one-to-one (`fromCardinality: one`) |
| Cross filter | Many-to-one single direction (Learn: "the most common, default direction"); one-to-one both (BIcraft only writes one-to-one in both directions, see `relationship.add`) |
| Ambiguity | A candidate that, added active after the earlier ones, gives a table two filter paths to another is passed over (`code: ambiguous`) |
| No data | Passed over (`code: no_data`): there is nothing to infer the cardinality from |
| Name | A new GUID, as Desktop names relationships |

The preview (no `apply`) changes nothing; `apply: true` adds all candidates, or those `only`
names, in one undo step. Saves write ordinary relationships (TMSL `relationships[]`, TMDL
`relationship` blocks, PBIX `Relationship` rows), as `relationship.add` does.

**Unchecked.** How Desktop matches names (it may compare more than equal names, e.g. a key
suffix), which candidate wins between two matches of one pair, whether it creates an ambiguous
candidate inactive instead of leaving it out, and whether it proposes many-to-many.

## Manage relationships

Sources: Microsoft Learn, "Create and manage relationships in Power BI Desktop" (Create a
relationship manually, Edit a relationship, Configure more options, Wrong active relationship,
cross filter ambiguity) and "Apply many-many relationships in Power BI Desktop"; the public TMSL
and TMDL references (`relationships[]`: `isActive`, `crossFilteringBehavior`,
`fromCardinality` / `toCardinality`, `securityFilteringBehavior`,
`relyOnReferentialIntegrity`).

Learn describes the dialog: the list of relationships (an Active box per row, From and To as
Table (Column)) with New, Autodetect, Edit and Delete; the Create / Edit window with a table
picker per end, a preview of each table's data, the column chosen in it, then Cardinality (many
to one, one to one, one to many, many to many), Cross filter direction (single, both), Make this
relationship active, Assume referential integrity and Apply security filter in both directions.
It validates the relationship as columns are picked and offers the cardinality and direction the
data suggests. BIcraft's review (`relationship.add` / `relationship.set` with `check: true`;
without it the same rules refuse or warn):

| Rule | BIcraft |
|---|---|
| Suggestion | From the session's data: both ends unique → one to one, both directions; the second end unique → many to one, single; the first end unique → one to many, single (written as many to one with the ends swapped); neither → many to many, single |
| One side | A "one" end with duplicate (without regard to case) or blank values is refused (`not_unique`); neither end unique and not many to many is refused with Learn's rule that one of the columns must have unique values (`no_unique_side`) |
| Many to many | Allowed, with a warning (`many_to_many`) |
| Active | One active relationship between two tables, and none that gives a table two active filter paths to another. A new relationship without `active` is added inactive with a note (`made_inactive`); `active: true` is refused (`active_exists`, `ambiguous`), as an edit that makes it active or cross filters both ways into an ambiguity |
| Security filter both | Needs cross filtering both ways (`security_needs_both`); written `securityFilteringBehavior: bothDirections` |
| Referential integrity | Both tables DirectQuery or Dual, not many to many (`referential_integrity`); written `relyOnReferentialIntegrity: true` (TMSL, TMDL, the PBIX `Relationship` row) |
| Edit an end | `new_from` / `new_to` keep the relationship's name; a PBIX rebuilds its storage under that name (as a cardinality change) |
| Preview | The first five rows of each table through `table.rows` (stored data or partitions; a DirectQuery table shows none) |

**Unchecked against Desktop.** The exact messages and when each shows, the many-to-many cross
filter Desktop offers, whether it unticks or greys "Make this relationship active" when the
relationship cannot be active, whether Dual tables allow referential integrity, the number of
preview rows, and its confirmation before Delete (BIcraft deletes at once; Undo restores).

## Roles

Sources: Microsoft Learn, "Row-level security (RLS) with Power BI" (Define roles in Power BI
Desktop); the TMSL Roles object.

The Manage security roles dialog follows Learn's steps: a list of roles (New, rename, delete),
"Select tables", then "Filter data" with a default editor and "Switch to DAX editor"; the DAX
editor has completion and a check mark that validates the expression and an X that reverts it;
"Switch to default editor" warns when the filter cannot be shown there (Cancel keeps it); Save.

| Rule (Learn) | BIcraft |
|---|---|
| A role name cannot contain a comma | `role.add` and `role.rename` refuse one |
| The filter returns True/False for each row | `role.check_filter` validates it as a row filter of the table (its row context); a result that is not True/False is a warning (`runtime-non-boolean-filter`) |
| The check mark validates without saving | `role.check_filter` changes nothing; Save runs `role.set_filter`, which refuses invalid DAX unless `force` |
| Some filters (e.g. `USERPRINCIPALNAME()`) need the DAX editor | Such a filter opens in the DAX editor; switching back asks first and drops it |

`role.rename {name, new_name}` renames `roles[].name` (unique, case ignored); its table
permissions stay, and View as follows the new name. TMDL, TMSL and PBIX keep it
(`crates/engine/tests/model/manage_roles_tests.rs`).

`role.set {name, description?, model_permission?}` writes `roles[].description` (TMSL) and
`roles[].modelPermission` (`none`, `read`, `readRefresh`, `refresh`, `administrator`, the TMSL
spellings; the case of the value given is ignored), for the Model view's Properties pane of a
role. TMDL and TMSL keep both. Which of these fields Desktop's own Properties pane offers for a
role was not checked.

The default editor's rows are column / condition / value joined by And or Or, written as DAX
read left to right: `[C] = "x"` (a number, `TRUE()`/`FALSE()` or `DATE(y, m, d)` when the column
is of that type and the text reads as one), `<>`, `>`, `>=`, `<`, `<=`, `CONTAINSSTRING([C], "x")`,
`ISBLANK([C])` and their `NOT`; `a || b && c` is written `(a || b) && c`
(`crates/ui/src/role_filter.rs`). A saved filter of that shape reads back into rows.

**Unchecked against Desktop.** The conditions Desktop's default editor offers and the DAX it
writes for them, how it joins And and Or, its messages, and whether a draft survives a change of
table (BIcraft drops an unsaved draft when another role or table is chosen).

## Object-level security

Sources: Microsoft Learn, "Object-level security (OLS)" (Fabric / Power BI) and "Analysis
Services tabular model object-level security"; the TMSL Roles object.

`role.set_permission {role, table, column?, metadata_permission}` writes
`roles[].tablePermissions[{name, metadataPermission}]` for a table and
`…columnPermissions[{name, metadataPermission}]` for a column, with `none` (hidden from the role)
or `read` (visible); `null` (or `default`) removes the setting, and a table permission left with
nothing (no filter, no permission) goes. TMDL writes them as `tablePermission T` /
`metadataPermission: none` and `columnPermission C` blocks; PBIX keeps them in its
`TablePermission` and `ColumnPermission` rows (round trips in `crates/engine/tests/report/ols_tests.rs`).

| Rule (Learn) | BIcraft |
|---|---|
| "Table-level security cannot be set for a model if it breaks a relationship chain" | `metadata_permission: none` on a table related to two or more other tables is refused |
| Measures that reference a secured table or column are restricted | Under View as, reading the object fails, so a measure that reads it fails too |
| A visual using a secured object shows the message of a missing object | Under View as the visual's error reads "cannot find column 'T'[C] (object-level security)" |
| RLS and OLS cannot be combined from different roles (an error at query time) | View as with a role that filters rows and another that secures objects fails every read |
| Relationships that reference a secured column work when its table is not secured | **Not followed**: under View as a relationship whose key column is secured fails (it fails closed) |

Several roles: an object is hidden only when every role chosen hides it (a role that does not
secure it grants read). **Unchecked**: that union, and which message Desktop's View as shows,
were not compared with Desktop. Desktop has no OLS editor of its own (Learn: TMDL view or
external tools); BIcraft's is the Manage roles dialog (the Model explorer's Roles menu).

## Perspectives

Sources: the TMSL Perspectives object and the TMDL `perspective` reference on Microsoft Learn.

`perspective.add {name, description?, tables?}`, `perspective.set {name, new_name?,
description?, tables?}` (the `tables` given replace the members) and `perspective.remove {name}`
write `perspectives[{name, description?, tables: [{name, columns: [{name}], measures: [{name}],
hierarchies: [{name}]}]}]`. A `tables` entry is a table name (every member), or `{table, all?,
columns?, measures?, hierarchies?}`; members are written one by one, in the order given (the
dialog: the model's order), never as `includeAll`. A perspective a file brought with
`includeAll: true` is read as every member the table has now. Renames follow and removals drop
members as before (`crates/engine/src/perspectives.rs`). TMDL, TMSL and PBIX round trips are in
`crates/engine/tests/model/perspective_edit_tests.rs`.

The Model view's Perspective list filters the diagram to the perspective's tables and their
members (view state, not saved). **Unchecked**: Desktop authors perspectives only through TMDL
view or external tools; how it filters its own diagram by perspective was not compared.
BIcraft's editor is the Manage perspectives dialog (the Model explorer's Perspectives menu).

## Properties of several objects

Sources: Microsoft Learn, "Use Model view in Power BI Desktop" (section "Set common
properties": Ctrl+click several tables, and a change in the Properties pane, such as Is hidden or
Storage mode, applies to all of them).

`model.set_properties {objects: [{table} | {table, column} | {table, measure}], props, force?}`
sets the same values on every object through its own setter (`table.set`, `column.set`,
`measure.set`, `table.set_storage_mode`), so their refusals apply; the first refusal puts the
session back as it was. It is one command, so one undo step. A property applies to some kinds
only: description, hidden and synonyms to all; display folder and format to columns and
measures; data category, summarize by and sort by column to columns; storage mode to tables. A
property that does not apply to one of the objects is refused. The written forms are those of the
single setters (TMDL, TMSL, PBIX round trips in
`crates/engine/tests/model/set_properties_tests.rs`).

In the Model view, Ctrl+click (or Shift+click) on the diagram adds an object to the selection or
takes it out; in the Data pane Ctrl+click does the same and Shift+click selects the rows from the
last one picked. The Properties pane then shows the properties the objects share; a value they
differ on is blank with "Multiple values" (a check box half ticked), and a change sets it on all.

**Unchecked**: Learn documents Ctrl+click on tables in the diagram only. Shift+click, selecting
columns and measures, selecting in the Data pane, which properties Desktop offers for a mixed
selection and how it shows differing values are BIcraft's choices. Switching several DirectQuery
tables to Import from the pane does not offer the Dual dialog a single table gets.

## Synonyms

Sources: Microsoft Learn, "Edit Q&A linguistic schema and add phrasings in Power BI Desktop"
(the schema lists entities with their bindings and words; `State: Generated` items are
regenerated, `State: Deleted` keeps a generated one from coming back; items written by hand have
no state); the TMSL `linguisticMetadata` of a culture (`content`, `contentType`).

A table's, column's or measure's synonyms are the terms of the entity bound to it in the Q&A
schema of the model's culture (en-US when the model names none): `cultures[].linguisticMetadata
.content.Entities.<key>` with `Definition.Binding` `{ConceptualEntity, ConceptualProperty?}` and
`Terms: [{term: {State?}}]`. The snapshot's `synonyms` lists the terms not `Suggested` or
`Deleted`. `model.set_properties` with `synonyms: [...]`:

| Case | Written |
|---|---|
| A listed term the entity has | kept as it is; a `Deleted` or `Suggested` one loses its state |
| A listed term it has not | `{term: {}}` |
| A `Generated` term left out | kept with `State: Deleted` |
| A `Suggested` or `Deleted` term left out | kept |
| Another term left out | removed |
| No entity bound to the object | a new one, keyed `table` or `table.column` (lower case, `_` for spaces), as `rename_refs.rs` assumes Desktop keys |
| No schema in the model's culture | `{"Version": "1.0.0", "Language": culture, "Entities": {…}}`, `contentType: json`; the culture is added when the model has none |
| The schema is JSON text | written back as text |
| A legacy XML schema | refused |

**Unchecked**: no file Desktop saved after editing synonyms was compared: the state Desktop gives
a synonym typed in its Properties pane (none, as written here, is inferred from Learn), the new
entity's `State`, the schema `Version` and other root keys Desktop writes (`DynamicImprovement`),
and whether Desktop lists suggested terms as synonyms.

## Storage mode

Sources: Microsoft Learn, "Table storage mode in Power BI semantic models", "Use composite models
in Power BI Desktop" and "Model relationships in Power BI Desktop" (source groups, limited
relationships); the TMSL Partition object (`mode`).

`table.set_storage_mode {table, mode: import|directQuery|dual, dual_related?}` writes the `mode`
of every partition of the table (TMDL `mode: directQuery` under each `partition`, TMSL
`partitions[].mode`, the PBIX `Partition.Mode` code: 0 import, 1 directQuery, 4 dual). A table's
mode, in the snapshot's `model.tables[].storage_mode`, is its partitions' (`default` or none is
the model's `defaultMode`, Import when absent), `hybrid` when they differ (an incremental refresh
policy with real time). TMDL, TMSL and PBIX round trips are in
`crates/engine/tests/model/storage_mode_tests.rs`.

| Rule (Learn) | BIcraft |
|---|---|
| A calculated table is computed in the model (always Import) | DirectQuery or Dual is refused for a calculated table, a calculation group and a field parameter |
| DirectQuery and Dual need a source that supports them | Refused for a non-M partition and for a query that reaches no connector (Enter data, `#table`) |
| "You can change a DirectQuery table to an Import or Dual table"; Import cannot go back | An Import table is changed with a warning (Desktop would not), except in a PBIX that stores its rows: refused, with the advice to edit a PBIP |
| Switching a DirectQuery table to Import shows a warning and offers to set dimension tables to Dual; the propagation follows only the one side of one-to-many relationships | The Properties pane asks first; `dual_related: true` sets the DirectQuery tables reached that way to Dual in the same undo step; without it they are returned as `dual_candidates` |
| A relationship between tables of different source groups is limited; Dual belongs to both | A relationship of the table between Import and DirectQuery is listed in `limited_relationships`, with a warning. BIcraft counts one DirectQuery source group (the connector or server is not compared) |
| Dual is for dimension tables | A warning when the table is on the many side of a relationship |
| A table with an incremental refresh policy is an Import table (hybrid: a DirectQuery partition added by the policy) | Refused while the table has a policy (`table.remove_refresh_policy` first) |

BIcraft evaluates imported data only: a DirectQuery table has no rows in visuals, which are
`unavailable` with the reason "not evaluated: DirectQuery" (`dax.evaluate` fails with it too). A
new PBIX (`file.save_as` from a model without data) writes a DirectQuery table without rows.

**Unchecked**: the files were not opened in Power BI Desktop; Desktop's dialog text and its exact
choice of tables to set to Dual (its "minimum set") were not compared; whether Desktop opens a
PBIP whose Import table BIcraft turned into DirectQuery was not tried.

## Incremental refresh policy

Sources: Microsoft Learn, "Incremental refresh and real-time data for semantic models",
"Configure incremental refresh and real-time data" and "Advanced incremental refresh and
real-time data with the XMLA endpoint"; the TMSL `refreshPolicy` object (BasicRefreshPolicy).

`table.set_refresh_policy {table, store: {n, unit}, refresh: {n, unit}, detect_changes?,
only_complete_periods?, realtime?}` writes the table's `refreshPolicy`; `table.remove_refresh_policy
{table}` removes it. Units are `day`, `month`, `quarter` and `year`.

| Dialog (Learn) | `refreshPolicy` |
|---|---|
| Archive data starting *n units* before refresh date | `rollingWindowPeriods` *n*, `rollingWindowGranularity` *unit* |
| Incrementally refresh data starting *n units* before refresh date | `incrementalPeriods` *n*, `incrementalGranularity` *unit* |
| Only refresh complete days (or periods) | `incrementalPeriodsOffset: -1` |
| Get the latest data in real time with DirectQuery (Premium) | `mode: hybrid`; it turns complete periods on |
| Detect data changes (a date/time column) | `pollingExpression`: BIcraft writes its own M, the column's `List.Max` (a fixed date when it is null) |
| — | `policyType: basic`; `sourceExpression`: the table's M query |

The rules BIcraft checks, each named in the error with what to do:

- the model has the `RangeStart` and `RangeEnd` parameters, names case-sensitive, of type
  Date/Time (`meta [IsParameterQuery=true, Type="DateTime"]`);
- the table is Import (or already hybrid by its policy) and has one M partition;
- the query references `RangeStart` and `RangeEnd` in a `Table.SelectRows` step (Learn: an equal
  sign on one bound only; BIcraft does not check which);
- the change detection column exists, is date/time and is not calculated.

Warnings: a refresh period longer than the archive period, a change detection column the range
filter also uses, real time needing a Premium, Premium Per User or Fabric capacity. The
compatibility level is raised to 1550 (1565 for real time) when lower. The policy applies when the
service refreshes the published model: Desktop and BIcraft keep the table's partition as it is.
The snapshot's `model.tables[].refresh_policy` and `range_filtered`, and
`model.incremental_refresh`, feed the dialog (a table's menu › Incremental refresh). TMDL, TMSL and
PBIX round trips are in `crates/engine/tests/model/refresh_policy_tests.rs`.

**Unchecked**: the files were not opened in Power BI Desktop nor published; Desktop's own
`pollingExpression` text was not compared; the dialog's exact wording in Desktop was not compared.
