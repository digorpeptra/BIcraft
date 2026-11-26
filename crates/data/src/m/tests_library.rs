//! The library functions of ROADMAP M11.34. Each case is the example of the function's
//! Microsoft Learn page where it has one (learn.microsoft.com/powerquery-m/<function>), written
//! as an M equality that must be true, plus edge cases and hostile inputs.

use super::*;

/// Evaluates `actual = expected` in M; on failure shows `actual` as JSON.
fn eq(actual: &str, expected: &str) {
    let e = format!("#table({{\"OK\"}}, {{{{({actual}) = ({expected})}}}})");
    let ok = load_m(&e, &Limits::default())
        .map(|t| t.columns()[0].values.clone())
        .map_err(|e| e.to_string());
    if ok != Ok(vec![CellValue::Bool(true)]) {
        let shown = format!(
            "#table({{\"V\"}}, {{{{let r = try ({actual}), j = try Text.FromBinary(Json.FromValue(r[Value])) in if r[HasError] then \"error: \" & r[Error][Message] else j[Value]? ?? \"(no JSON)\"}}}})"
        );
        let got = load_m(&shown, &Limits::default()).map(|t| t.columns()[0].values.clone());
        panic!("{actual}\n  expected {expected}\n  check {ok:?}\n  got {got:?}");
    }
}

/// `expr` evaluates to an M error (caught by `try`).
fn fails(expr: &str) {
    eq(&format!("(try {expr})[HasError]"), "true");
}

/// `expr` is stopped by a limit.
fn limited(expr: &str, limits: &Limits) {
    let e = format!("#table({{\"V\"}}, {{{{Value.Type({expr}) = null}}}})");
    let r = load_m(&e, limits);
    assert!(matches!(r, Err(Error::LimitExceeded(_))), "{expr}: {r:?}");
}

#[test]
fn list_functions() {
    // List.Modes, List.Mode (the last value to reach the top count wins).
    eq("List.Modes({\"A\", 1, 2, 3, 3, 4, 5, 5})", "{3, 5}");
    eq("List.Mode({\"A\", 1, 2, 3, 3, 4, 5})", "3");
    eq("List.Mode({\"A\", 1, 2, 3, 3, 4, 5, 5})", "5");
    eq("List.Modes({})", "{}");
    fails("List.Mode({})");
    // List.Covariance: the population covariance.
    eq(
        "Number.Round(List.Covariance({1, 2, 3}, {1, 2, 3}), 6)",
        "0.666667",
    );
    fails("List.Covariance({1, 2}, {1})");
    // List.Percentile.
    eq("List.Percentile({5, 3, 1, 7, 9}, 0.25)", "3");
    eq(
        "List.Percentile({5, 3, 1, 7, 9}, {0.25, 0.5, 0.75}, [PercentileMode = PercentileMode.SqlDisc])",
        "{3, 5, 7}",
    );
    eq(
        "List.Percentile({1, 2, 3, 4}, 0.5, [PercentileMode = PercentileMode.SqlCont])",
        "2.5",
    );
    eq("List.Percentile({}, 0.5)", "null");
    // List.MaxN, List.MinN.
    eq("List.MaxN({3, 4, 5, -1, 7, 8, 2}, 5)", "{8, 7, 5, 4, 3}");
    eq(
        "List.MaxN({3, 4, 5, -1, 7, 8, 2}, each _ > 0)",
        "{8, 7, 5, 4, 3, 2}",
    );
    eq("List.MinN({3, 4, 5, -1, 7, 8, 2}, 5)", "{-1, 2, 3, 4, 5}");
    eq("List.MinN({3, 4, 5, -1, 7, 8, 2}, each _ < 0)", "{-1}");
    eq("List.MaxN({1, 2}, 10)", "{2, 1}");
    // List.PositionOfAny, List.Positions.
    eq("List.PositionOfAny({1, 2, 3}, {2, 3})", "1");
    eq("List.PositionOfAny({1, 2, 3, 2}, {2, 3}, Occurrence.Last)", "3");
    eq("List.PositionOfAny({1, 2, 3, 2}, {2}, Occurrence.All)", "{1, 3}");
    eq("List.PositionOfAny({1}, {9})", "-1");
    eq("List.Positions({1, 2, 3, 4, null, 5})", "{0, 1, 2, 3, 4, 5}");
    // List.IsDistinct.
    eq("List.IsDistinct({1, 2, 3})", "true");
    eq("List.IsDistinct({1, 2, 3, 3})", "false");
    eq(
        "List.IsDistinct({\"a\", \"A\"}, Comparer.OrdinalIgnoreCase)",
        "false",
    );
    // List.RemoveMatchingItems, List.RemoveRange.
    eq(
        "List.RemoveMatchingItems({1, 2, 3, 4, 5, 5}, {1, 5})",
        "{2, 3, 4}",
    );
    eq(
        "List.RemoveRange({1, 2, 3, 4, -6, -2, -1, 5}, 4, 3)",
        "{1, 2, 3, 4, 5}",
    );
    eq("List.RemoveRange({1, 2, 3}, 1)", "{1, 3}");
    fails("List.RemoveRange({1, 2, 3}, 5)");
    // List.ReplaceMatchingItems, List.ReplaceRange.
    eq(
        "List.ReplaceMatchingItems({1, 2, 3, 4, 5}, {{2, -2}, {3, -3}})",
        "{1, -2, -3, 4, 5}",
    );
    eq(
        "List.ReplaceRange({1, 2, 7, 8, 9, 5}, 2, 3, {3, 4})",
        "{1, 2, 3, 4, 5}",
    );
    // List.DateTimeZones.
    eq(
        "List.DateTimeZones(#datetimezone(2011, 12, 31, 23, 55, 0, -8, 0), 3, #duration(0, 0, 1, 0))",
        "{#datetimezone(2011, 12, 31, 23, 55, 0, -8, 0), #datetimezone(2011, 12, 31, 23, 56, 0, -8, 0), #datetimezone(2011, 12, 31, 23, 57, 0, -8, 0)}",
    );
}

const CUSTOMERS: &str = "Table.FromRecords({[CustomerID = 1, Name = \"Bob\", Revenue = 200], [CustomerID = 2, Name = \"Jim\", Revenue = 100], [CustomerID = 3, Name = \"Paul\", Revenue = 200], [CustomerID = 4, Name = \"Ringo\", Revenue = 50]})";

fn with_customers(e: &str) -> String {
    e.replace("$T", CUSTOMERS)
}

#[test]
fn table_functions() {
    // Table.AddRankColumn (Learn's example: competition ranking).
    eq(
        &with_customers(
            "Table.AddRankColumn($T, \"RevenueRank\", {\"Revenue\", Order.Descending}, [RankKind = RankKind.Competition])[RevenueRank]",
        ),
        "{1, 1, 3, 4}",
    );
    eq(
        &with_customers(
            "Table.AddRankColumn($T, \"R\", {\"Revenue\", Order.Descending}, [RankKind = RankKind.Dense])[R]",
        ),
        "{1, 1, 2, 3}",
    );
    eq(
        &with_customers(
            "Table.AddRankColumn($T, \"R\", {\"Revenue\", Order.Descending}, [RankKind = RankKind.Ordinal])[R]",
        ),
        "{1, 2, 3, 4}",
    );
    // Table.AlternateRows: keep 1, then skip 1 and keep 1 repeatedly.
    eq(
        "Table.AlternateRows(Table.FromRecords({[a = 1], [a = 2], [a = 3], [a = 4], [a = 5]}), 1, 1, 1)[a]",
        "{1, 3, 5}",
    );
    // Table.ColumnsOfType.
    eq(
        "Table.ColumnsOfType(Table.FromRecords({[a = 1, b = \"hello\"]}, type table [a = Number.Type, b = Text.Type]), {type number})",
        "{\"a\"}",
    );
    // Table.ContainsAll, Table.ContainsAny.
    eq(
        &with_customers("Table.ContainsAll($T, {[Name = \"Bob\"], [Name = \"Jim\"]})"),
        "true",
    );
    eq(
        &with_customers("Table.ContainsAll($T, {[Name = \"Bob\"], [Name = \"Zed\"]})"),
        "false",
    );
    eq(
        &with_customers("Table.ContainsAny($T, {[Name = \"Zed\"], [Name = \"Jim\"]})"),
        "true",
    );
    eq(
        &with_customers("Table.ContainsAny($T, {[Name = \"Zed\"]})"),
        "false",
    );
    // Table.FindText.
    eq(&with_customers("Table.FindText($T, \"Rin\")[CustomerID]"), "{4}");
    // Table.IsDistinct.
    eq(&with_customers("Table.IsDistinct($T)"), "true");
    eq(&with_customers("Table.IsDistinct($T, \"Revenue\")"), "false");
    // Table.MaxN, Table.MinN.
    eq(
        &with_customers("Table.MaxN($T, \"Revenue\", 2)[Revenue]"),
        "{200, 200}",
    );
    eq(
        &with_customers("Table.MinN($T, \"Revenue\", 2)[Name]"),
        "{\"Ringo\", \"Jim\"}",
    );
    eq(
        &with_customers("Table.MaxN($T, \"Revenue\", each [Revenue] > 150)[CustomerID]"),
        "{1, 3}",
    );
    // Table.Partition.
    eq(
        &with_customers("List.Transform(Table.Partition($T, \"CustomerID\", 2, each _), each _[CustomerID])"),
        "{{2, 4}, {1, 3}}",
    );
    // Table.PositionOfAny.
    eq(
        &with_customers(
            "Table.PositionOfAny($T, {[Name = \"Paul\"], [Name = \"Jim\"]}, Occurrence.First, \"Name\")",
        ),
        "1",
    );
    eq(
        &with_customers("Table.PositionOfAny($T, {[Revenue = 200]}, Occurrence.All, \"Revenue\")"),
        "{0, 2}",
    );
    // Table.RemoveMatchingRows, Table.RemoveRows.
    // Rows are compared whole unless equation criteria name the columns.
    eq(
        &with_customers("Table.PositionOfAny($T, {[Name = \"Paul\"]})"),
        "-1",
    );
    eq(
        "Table.RemoveMatchingRows(Table.FromRecords({[a = 1, b = 2], [a = 3, b = 4], [a = 1, b = 6]}), {[a = 1]}, \"a\")",
        "Table.FromRecords({[a = 3, b = 4]})",
    );
    eq(
        &with_customers("Table.RowCount(Table.RemoveMatchingRows($T, {[Revenue = 200]}))"),
        "4",
    );
    eq(
        &with_customers("Table.RemoveRows($T, 1, 2)[Name]"),
        "{\"Bob\", \"Ringo\"}",
    );
    eq(
        &with_customers("Table.RemoveRows($T, 3)[Name]"),
        "{\"Bob\", \"Jim\", \"Paul\"}",
    );
    // Table.ReplaceMatchingRows, Table.ReplaceRows.
    eq(
        "Table.ReplaceMatchingRows(Table.FromRecords({[a = 1, b = 2], [a = 2, b = 3], [a = 3, b = 4], [a = 1, b = 2]}), {[a = 1, b = 2], [a = -1, b = -2]})[a]",
        "{-1, 2, 3, -1}",
    );
    eq(
        "Table.ReplaceRows(Table.FromRecords({[a = 1], [a = 2], [a = 3]}), 0, 2, {[a = 7]})[a]",
        "{7, 3}",
    );
    // Table.Schema.
    eq(
        &with_customers("Table.Schema($T)[Name]"),
        "{\"CustomerID\", \"Name\", \"Revenue\"}",
    );
    eq(&with_customers("Table.ColumnCount(Table.Schema($T))"), "19");
    // Table.SingleRow.
    eq(
        "Table.SingleRow(Table.FromRecords({[a = 1, b = 2]}))",
        "[a = 1, b = 2]",
    );
    fails(&with_customers("Table.SingleRow($T)"));
    // Table.Split, Table.SplitAt.
    eq(
        &with_customers("List.Transform(Table.Split($T, 3), Table.RowCount)"),
        "{3, 1}",
    );
    eq(
        &with_customers("List.Transform(Table.SplitAt($T, 1), Table.RowCount)"),
        "{1, 3}",
    );
    // Table.ToList.
    eq(
        "Table.ToList(Table.FromRows({{\"a\", \"b\"}, {\"c\", \"d\"}}))",
        "{\"a,b\", \"c,d\"}",
    );
    eq(
        "Table.ToList(Table.FromRows({{\"a\", \"b\"}}), Combiner.CombineTextByDelimiter(\"-\"))",
        "{\"a-b\"}",
    );
    // Table.TransformColumnNames.
    eq(
        "Table.ColumnNames(Table.TransformColumnNames(Table.FromRecords({[#\"Col#(tab)umn\" = 1]}), Text.Clean))",
        "{\"Column\"}",
    );
    // Table.CombineColumnsToRecord.
    eq(
        "Table.CombineColumnsToRecord(Table.FromRecords({[a = 1, b = 2, c = 3]}), \"R\", {\"a\", \"b\"}){0}",
        "[c = 3, R = [a = 1, b = 2]]",
    );
    // Table.AggregateTableColumn.
    eq(
        "Table.AggregateTableColumn(#table({\"t\"}, {{#table({\"v\"}, {{1}, {2}})}}), \"t\", {{\"v\", List.Sum, \"sum\"}, {\"v\", List.Count, \"n\"}}){0}",
        "[sum = 3, n = 2]",
    );
    // Table.AddJoinColumn.
    eq(
        "Table.AddJoinColumn(#table({\"k\"}, {{1}, {2}}), \"k\", #table({\"id\", \"v\"}, {{1, \"x\"}, {1, \"y\"}}), \"id\", \"J\")[J]{0}[v]",
        "{\"x\", \"y\"}",
    );
    eq(
        "Table.RowCount(Table.AddJoinColumn(#table({\"k\"}, {{1}, {2}}), \"k\", #table({\"id\"}, {{1}}), \"id\", \"J\"){1}[J])",
        "0",
    );
    // Table.ApproximateRowCount.
    eq(&with_customers("Table.ApproximateRowCount($T)"), "4");
}

#[test]
fn number_binary_json() {
    eq("Number.BitwiseAnd(12, 10)", "8");
    eq("Number.BitwiseOr(12, 10)", "14");
    eq("Number.BitwiseXor(12, 10)", "6");
    eq("Number.BitwiseNot(0)", "-1");
    eq("Number.BitwiseShiftLeft(1, 4)", "16");
    eq("Number.BitwiseShiftRight(16, 4)", "1");
    eq("Number.BitwiseAnd(null, 1)", "null");
    fails("Number.BitwiseAnd(1.5, 1)");
    fails("Number.BitwiseOr(1e30, 1)");
    eq("Number.PI > 3.14159 and Number.PI < 3.1416", "true");
    eq("Number.Epsilon > 0 and Number.Epsilon < 1e-300", "true");
    eq("Number.IsNaN(Number.NaN)", "true");
    eq("Number.PositiveInfinity = -Number.NegativeInfinity", "true");
    // Binary.
    eq(
        "Binary.Combine({#binary({1}), #binary({2, 3})})",
        "#binary({1, 2, 3})",
    );
    eq(
        "Binary.Range(#binary({0, 1, 2, 3, 4, 5}), 2)",
        "#binary({2, 3, 4, 5})",
    );
    eq(
        "Binary.Range(#binary({0, 1, 2, 3, 4, 5}), 2, 2)",
        "#binary({2, 3})",
    );
    eq("Binary.Range(#binary({0, 1}), 5)", "#binary({})");
    eq(
        "Binary.Split(#binary({1, 2, 3, 4, 5}), 2)",
        "{#binary({1, 2}), #binary({3, 4}), #binary({5})}",
    );
    fails("Binary.Split(#binary({1}), 0)");
    eq(
        "Binary.ApproximateLength(Binary.FromText(\"i45WMlSKjQUA\", BinaryEncoding.Base64))",
        "9",
    );
    eq(
        "Binary.From(\"1011\")",
        "Binary.FromText(\"1011\", BinaryEncoding.Base64)",
    );
    eq("Binary.From(null)", "null");
    // Json.FromValue: Learn's example, then the other kinds of value.
    eq(
        "Text.FromBinary(Json.FromValue([A = {1, true, \"3\"}, B = #date(2012, 3, 25)]))",
        "\"{\"\"A\"\":[1,true,\"\"3\"\"],\"\"B\"\":\"\"2012-03-25\"\"}\"",
    );
    eq(
        "Text.FromBinary(Json.FromValue({null, 1.5, \"a/b\"\"é\", #time(1, 2, 3), #duration(1, 2, 3, 4)}))",
        "\"[null,1.5,\"\"a\\/b\\\"\"\\u00e9\"\",\"\"01:02:03\"\",\"\"P1DT2H3M4S\"\"]\"",
    );
    eq(
        "Text.FromBinary(Json.FromValue(#table({\"a\"}, {{1}, {2}})))",
        "\"[{\"\"a\"\":1},{\"\"a\"\":2}]\"",
    );
    fails("Json.FromValue(each _)");
}

#[test]
fn value_type_record_functions() {
    eq("Type.Is(Value.Type(1), type number)", "true");
    eq("Type.Is(Value.Type(\"a\"), type text)", "true");
    eq(
        "Table.ColumnNames(#table(Value.Type(#table({\"x\"}, {})), {}))",
        "{\"x\"}",
    );
    eq("Value.ReplaceMetadata(1, [a = 1])", "1");
    eq("Value.NullableEquals(1, 1)", "true");
    eq("Value.NullableEquals(null, 1)", "null");
    eq("Value.NullableEquals(1, 2)", "false");
    eq("Type.Is(type number, type nullable number)", "true");
    eq("Type.Is(type nullable number, type number)", "false");
    eq("Type.IsNullable(type nullable number)", "true");
    eq("Type.IsNullable(type number)", "false");
    eq("Type.IsNullable(Type.NonNullable(type nullable number))", "false");
    eq("Type.IsNullable(Type.NonNullable(type any))", "false");
    eq(
        "Type.Is(Type.TableColumn(type table [Name = text], \"Name\"), type text)",
        "true",
    );
    fails("Type.TableColumn(type table [Name = text], \"Other\")");
    // Record.ReorderFields: record equality ignores field order, so compare the names.
    eq(
        "Record.FieldNames(Record.ReorderFields([CustomerID = 1, OrderID = 1, Item = \"Fishing rod\", Price = 100.0], {\"OrderID\", \"CustomerID\"}))",
        "{\"OrderID\", \"CustomerID\", \"Item\", \"Price\"}",
    );
    eq(
        "Record.FieldNames(Record.ReorderFields([a = 1, b = 2, c = 3], {\"c\", \"a\"}))",
        "{\"c\", \"b\", \"a\"}",
    );
    fails("Record.ReorderFields([a = 1], {\"z\"})");
    eq(
        "Record.FieldNames(Record.ReorderFields([a = 1], {\"z\"}, MissingField.Ignore))",
        "{\"a\"}",
    );
    // Time.ToRecord, DateTimeZone.ToRecord.
    eq(
        "Time.ToRecord(#time(11, 56, 2))",
        "[Hour = 11, Minute = 56, Second = 2]",
    );
    eq(
        "DateTimeZone.ToRecord(#datetimezone(2011, 12, 31, 11, 56, 2, 8, 0))",
        "[Year = 2011, Month = 12, Day = 31, Hour = 11, Minute = 56, Second = 2, ZoneHours = 8, ZoneMinutes = 0]",
    );
}

#[test]
fn splitters() {
    eq(
        "Splitter.SplitTextByEachDelimiter({\";\", \",\"})(\"a;b,c,d\")",
        "{\"a\", \"b\", \"c,d\"}",
    );
    eq(
        "Splitter.SplitTextByEachDelimiter({\";\", \",\"}, null, true)(\"a;b,c;d\")",
        "{\"a;b\", \"c\", \"d\"}",
    );
    eq(
        "Splitter.SplitTextByEachDelimiter({\",\"})(\"\"\"a,b\"\",c\")",
        "{\"a,b\", \"c\"}",
    );
    eq(
        "Splitter.SplitTextByEachDelimiter({\",\"}, QuoteStyle.None)(\"\"\"a,b\"\",c\")",
        "{\"\"\"a\", \"b\"\",c\"}",
    );
    eq(
        "Splitter.SplitTextByRanges({{0, 4}, {5, 10}})(\"code-limiter\")",
        "{\"code\", \"limiter\"}",
    );
    eq(
        "Splitter.SplitTextByRanges({{0, 2}}, true)(\"abcde\")",
        "{\"de\"}",
    );
    eq(
        "Splitter.SplitTextByRepeatedLengths(3)(\"abcdefgh\")",
        "{\"abc\", \"def\", \"gh\"}",
    );
    eq(
        "Splitter.SplitTextByRepeatedLengths(3, true)(\"abcdefgh\")",
        "{\"ab\", \"cde\", \"fgh\"}",
    );
    fails("Splitter.SplitTextByRepeatedLengths(0)");
    eq(
        "Splitter.SplitTextByWhitespace()(\"a b \t c\")",
        "{\"a\", \"b\", \"c\"}",
    );
    eq(
        "Splitter.SplitTextByWhitespace()(\"a \"\"b c\"\" d\")",
        "{\"a\", \"b c\", \"d\"}",
    );
    eq(
        "Splitter.SplitTextByCharacterTransition({\"a\"..\"z\"}, {\"0\"..\"9\"})(\"ab12cd34\")",
        "{\"ab\", \"12cd\", \"34\"}",
    );
    eq(
        "Splitter.SplitTextByCharacterTransition((c) => c = \"-\", each true)(\"a-b-c\")",
        "{\"a-\", \"b-\", \"c\"}",
    );
    eq("Splitter.SplitTextByWhitespace()(null)", "{null}");
}

#[test]
fn hostile_inputs_are_capped() {
    let rows = Limits {
        max_rows: 1000,
        ..Limits::default()
    };
    limited(
        "List.DateTimeZones(#datetimezone(2011, 1, 1, 0, 0, 0, 0, 0), 1e12, #duration(0, 0, 1, 0))",
        &rows,
    );
    limited(
        "Binary.Split(Binary.FromText(Text.Repeat(\"AAAA\", 2000)), 1)",
        &rows,
    );
    limited(
        "Splitter.SplitTextByRepeatedLengths(1)(Text.Repeat(\"a\", 5000))",
        &rows,
    );
    limited(
        "Splitter.SplitTextByWhitespace()(Text.Repeat(\"a \", 5000))",
        &rows,
    );
    // Deep nesting in Json.FromValue is bounded by the evaluation depth.
    let deep = Limits {
        max_eval_depth: 50,
        ..Limits::default()
    };
    limited(
        "Json.FromValue(List.Accumulate({1..200}, {}, (s, _) => {s}))",
        &deep,
    );
    // Huge counts and offsets give errors or clamped results, never a crash.
    eq("List.MaxN({1, 2, 3}, 1e15)", "{3, 2, 1}");
    eq("Binary.Range(#binary({1}), 0, 1e300)", "#binary({1})");
    eq("Splitter.SplitTextByRanges({{1e300, 1e300}})(\"abc\")", "{\"\"}");
    fails("Number.BitwiseShiftLeft(1, 1e300)");
}

/// BIcraft's M catalogue (`bicraft-mlang`, the completion, hover and signature help of M11.35)
/// lists exactly the functions the evaluator runs, each with as many required and optional
/// parameters as it takes, and every constant it lists evaluates.
#[test]
fn the_language_services_catalogue_matches_the_evaluator() {
    let table = super::lib::table();
    let mut missing = Vec::new();
    for (name, b) in &table {
        match bicraft_mlang::function(name) {
            None => missing.push(*name),
            Some(f) => assert_eq!(
                (f.required(), f.params.len()),
                (b.min, b.max),
                "{name}: {}",
                f.signature()
            ),
        }
    }
    missing.sort_unstable();
    assert!(missing.is_empty(), "not in the catalogue: {missing:?}");
    for f in bicraft_mlang::functions() {
        assert!(table.contains_key(f.name), "{} is not evaluated", f.name);
    }
    for k in bicraft_mlang::constants() {
        eq(&format!("(try {})[HasError]", k.name), "false");
    }
}

/// `type type` is the primitive type of types, also inside a type (`nullable type`, a parameter).
#[test]
fn type_type_is_the_primitive_type_of_types() {
    eq("Type.Is(Value.Type(type text), type type)", "true");
    eq("Type.Is(type type, type text)", "false");
    eq("Type.Is(type nullable type, type any)", "true");
    eq("((t as type) => Type.Is(t, type text))(type text)", "true");
    eq("Type.Is(type {number}, type list)", "true");
}

/// Argument counts that Power BI Desktop's M engine reports for these functions (the signature
/// corpus of M11.40, `bicraft-mlang`'s tests/oracle/m_signatures.json): a count outside them is
/// an error, a count inside runs.
#[test]
fn argument_counts_follow_the_signature_corpus() {
    // the count or condition of List.LastN is optional (the last item)
    eq("List.LastN({1, 2, 3})", "{3}");
    // a culture comparer needs its culture
    fails("Comparer.FromCulture()");
    eq("Value.Is(Comparer.FromCulture(\"en-US\"), type function)", "true");
    // the FromText readers take a text and its options, nothing more
    fails("Date.FromText(\"2020-01-02\", null, \"en-US\")");
    fails("DateTime.FromText(\"2020-01-02\", null, \"en-US\")");
    fails("DateTimeZone.FromText(\"2020-01-02T00:00:00+01:00\", null, \"en-US\")");
    fails("Time.FromText(\"10:00\", null, \"en-US\")");
    eq(
        "Date.FromText(\"2020-01-02\", [Culture = \"en-US\"])",
        "#date(2020, 1, 2)",
    );
    // Lines.FromText takes no encoding (Lines.FromBinary does)
    fails("Lines.FromText(\"a\", null, false, 65001)");
    // Table.NestedJoin has no join algorithm: seven parameters at most
    let t = "#table({\"K\"}, {{1}})";
    fails(&format!(
        "Table.NestedJoin({t}, \"K\", {t}, \"K\", \"N\", JoinKind.Inner, null, null)"
    ));
    eq(
        &format!("Table.RowCount(Table.NestedJoin({t}, \"K\", {t}, \"K\", \"N\", JoinKind.Inner, null))"),
        "1",
    );
}
