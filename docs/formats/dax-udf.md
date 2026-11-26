# DAX user-defined functions

Sources: Microsoft Learn, [DAX user-defined functions (preview)](https://learn.microsoft.com/en-us/dax/best-practices/dax-user-defined-functions)
(syntax, parameter types and pass modes, naming, limits, the Model explorer's Functions node, TMDL
`function`), [DEFINE](https://learn.microsoft.com/en-us/dax/define-statement-dax) (`DEFINE
FUNCTION` in a query), the TMDL and TMSL object references on Learn (`function` objects:
`expression`, `description`, `isHidden`, `lineageTag`).
The round trips are BIcraft's writers read back by BIcraft's readers.

## Storage

A function is a model object of the `functions` collection (TMSL `model.functions[]`, TMDL
`function Name = …` in `definition/functions.tmdl`, the PBIX metadata's functions). BIcraft keeps
the collection as TMSL JSON in `SemanticModel.extra["functions"]`, so TMDL, TMSL and PBIX carry
it unchanged:

```
{"name": "Sales.AddTax", "expression": "(amount : NUMERIC, rate = 0.1) => amount * (1 + rate)",
 "description": "Adds the tax", "isHidden": false, "lineageTag": "…"}
```

The model needs compatibility level **1702** or higher: `function.add`, `function.set` and a
`tmdl.apply` that leaves functions raise a lower level to 1702. Learn says functions are not
supported in a model without tables; `function.add` refuses one.

## Definition

`(parameters) => body`, every parameter `name [: type [subtype] [mode]] [= default]`:

| Part | Values | Notes |
|---|---|---|
| type | `AnyVal` (default), `Scalar`, `Table`, `AnyRef`, `CalendarRef`, `ColumnRef`, `MeasureRef`, `TableRef` | case-insensitive |
| subtype (Scalar) | `Variant`, `Int64`, `Decimal`, `Double`, `String`, `DateTime`, `Boolean`, `Numeric` | |
| mode | `val` (default: evaluated once, before the body) or `expr` (lazy: evaluated where the body uses it) | the `…Ref` types are `expr` |
| default | a DAX expression | a parameter with a default is optional |

Rules Learn states and BIcraft checks: 0 to 256 parameters; a name has letters, digits,
underscores and periods (namespaces; not first or last, no two in a row), no spaces, and is not
a DAX function or a reserved word; names are unique ignoring case; no recursion, direct or
mutual; no overloading.

## Commands

- `function.add {name, expression | body + parameters, description?, hidden?, force?}`,
  `function.set` (the part left out is kept: a new body keeps the parameters), `function.rename
  {name, new_name}` (calls in measures, columns, tables, row filters, calculation items and other
  functions follow: `calls_rewritten`), `function.remove {name, force?}` (refused while called).
  Each is one undo step.
- The definition is validated as the query `DEFINE FUNCTION Name = … EVALUATE {0}` against the
  model; its diagnostics are shifted back onto the definition (line and column).
- Call sites: the DAX binder knows the model's functions. A call with fewer arguments than the
  rightmost parameter without a default, or more than the parameters, is
  `wrong-argument-count`; a table where a scalar parameter is declared, or a scalar where a
  `Table`/`…Ref` parameter is, is `argument-type`.

## Evaluation

BIcraft's evaluator expands calls before it compiles a measure or a query (`dax.evaluate`
accepts `DEFINE FUNCTION` too). A `val` argument becomes a variable bound once, before the
body (`VAR __udf1_0 = (arg) RETURN (body)`); an `expr` argument replaces the parameter's name in
the body, so it is evaluated in the body's context, as Learn describes the two modes. A missing
optional argument is its default (`BLANK()` without one). Expansion stops after 32 nested rounds
and at 4 MiB of text.

## UI

The Model explorer's Model tab lists **Functions (n)**: New function, then per function Edit
function (the form with its definition in the DAX editor), Rename function, Delete function and
Script TMDL (to the TMDL view, [tmdl-view.md](tmdl-view.md)). Learn says Desktop cannot hide or
unhide a function in the UI; BIcraft keeps the TOM `isHidden` property through `function.set
{hidden}`.
