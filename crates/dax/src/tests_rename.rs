//! Renaming references inside DAX text (G15).

use bicraft_model::{Column, DataType, Measure, SemanticModel, Table};

use crate::deps::ObjectRef;
use crate::rename::{Renamer, Rewrite, Site};

fn col(name: &str) -> Column {
    Column {
        name: name.into(),
        data_type: DataType::Double,
        ..Default::default()
    }
}

fn measure(name: &str, expression: &str) -> Measure {
    Measure {
        name: name.into(),
        expression: expression.into(),
        ..Default::default()
    }
}

fn table(name: &str, columns: &[&str], measures: Vec<Measure>) -> Table {
    Table {
        name: name.into(),
        columns: columns.iter().map(|c| col(c)).collect(),
        measures,
        ..Default::default()
    }
}

fn model() -> SemanticModel {
    SemanticModel {
        name: "m".into(),
        tables: vec![
            table(
                "Sales",
                &["Amount", "Qty", "Region", "Date"],
                vec![measure("Total", "SUM(Sales[Amount])"), measure("X]Y", "1")],
            ),
            table(
                "Order Lines",
                &["Qty", "Price"],
                vec![measure("Lines", "COUNTROWS('Order Lines')")],
            ),
            table("Product", &["Name", "Category"], vec![]),
            table("O'Brien", &["Id"], vec![]),
            table("Ventes été", &["Montant"], vec![]),
        ],
        ..Default::default()
    }
}

fn t(name: &str) -> ObjectRef {
    ObjectRef::Table { name: name.into() }
}

fn c(table: &str, name: &str) -> ObjectRef {
    ObjectRef::Column {
        table: table.into(),
        name: name.into(),
    }
}

fn m(table: &str, name: &str) -> ObjectRef {
    ObjectRef::Measure {
        table: table.into(),
        name: name.into(),
    }
}

const MEASURE: Site<'static> = Site {
    home: Some("Sales"),
    row_context: false,
    udf: false,
};

fn rw(text: &str, site: Site<'_>, target: &ObjectRef, new: &str) -> String {
    let model = model();
    match Renamer::new(&model).rewrite(text, site, target, new) {
        Rewrite::Rewritten { text, .. } => text,
        Rewrite::Unchanged => text.to_string(),
        Rewrite::Unparsable => "<unparsable>".into(),
    }
}

#[test]
fn measure_references_only_real_tokens() {
    let src = "[Total] * 2 // [Total] in a comment\n& \"[Total]\" /* [Total] */ -- [Total]\n+ [total]";
    assert_eq!(
        rw(src, MEASURE, &m("Sales", "Total"), "Revenue"),
        "[Revenue] * 2 // [Total] in a comment\n& \"[Total]\" /* [Total] */ -- [Total]\n+ [Revenue]"
    );
}

#[test]
fn qualified_measure_keeps_its_qualifier() {
    assert_eq!(
        rw(
            "Sales[Total] + 'Sales'[TOTAL]",
            MEASURE,
            &m("Sales", "Total"),
            "Revenue"
        ),
        "Sales[Revenue] + 'Sales'[Revenue]"
    );
}

#[test]
fn closing_brackets_are_escaped_both_ways() {
    assert_eq!(
        rw("[X]]Y] + 1", MEASURE, &m("Sales", "X]Y"), "A]]B"),
        "[A]]]]B] + 1"
    );
    assert_eq!(rw("[Total]", MEASURE, &m("Sales", "Total"), "a]b"), "[a]]b]");
}

#[test]
fn same_column_name_in_other_tables_is_left_alone() {
    let src = "SUM(Sales[Qty]) + SUM('Order Lines'[Qty]) + SUMX('Order Lines', [Qty]) + SUMX(Sales, [Qty])";
    assert_eq!(
        rw(src, MEASURE, &c("Sales", "Qty"), "Units"),
        "SUM(Sales[Units]) + SUM('Order Lines'[Qty]) + SUMX('Order Lines', [Qty]) + SUMX(Sales, [Units])"
    );
    assert_eq!(
        rw(src, MEASURE, &c("Order Lines", "Qty"), "Quantity"),
        "SUM(Sales[Qty]) + SUM('Order Lines'[Quantity]) + SUMX('Order Lines', [Quantity]) + SUMX(Sales, [Qty])"
    );
}

#[test]
fn row_context_of_calculated_columns_and_filters() {
    let on_sales = Site {
        home: Some("Sales"),
        row_context: true,
        udf: false,
    };
    let on_lines = Site {
        home: Some("Order Lines"),
        row_context: true,
        udf: false,
    };
    assert_eq!(
        rw("[Qty] * 2", on_sales, &c("Sales", "Qty"), "Units"),
        "[Units] * 2"
    );
    assert_eq!(
        rw("[Qty] * 2", on_lines, &c("Sales", "Qty"), "Units"),
        "[Qty] * 2"
    );
    assert_eq!(
        rw("[Region] = \"West\"", on_sales, &c("Sales", "Region"), "Area"),
        "[Area] = \"West\""
    );
    // FILTER over another table inside a calculated column: the inner row context wins.
    assert_eq!(
        rw(
            "COUNTROWS(FILTER('Order Lines', [Qty] > 1))",
            on_sales,
            &c("Sales", "Qty"),
            "Units"
        ),
        "COUNTROWS(FILTER('Order Lines', [Qty] > 1))"
    );
}

#[test]
fn table_rename_rewrites_quoted_bare_and_table_arguments() {
    let src = "SUM(Sales[Amount]) + COUNTROWS(Sales) + COUNTROWS('Sales') + CALCULATE([Total], RELATEDTABLE(sales)) + SUM('Order Lines'[Qty])";
    assert_eq!(
        rw(src, MEASURE, &t("Sales"), "Fact Sales"),
        "SUM('Fact Sales'[Amount]) + COUNTROWS('Fact Sales') + COUNTROWS('Fact Sales') + CALCULATE([Total], RELATEDTABLE('Fact Sales')) + SUM('Order Lines'[Qty])"
    );
    assert_eq!(
        rw(
            "Sales[Amount] + 'Sales'[Qty] + RELATED(Sales[Region])",
            MEASURE,
            &t("Sales"),
            "Fact"
        ),
        "Fact[Amount] + 'Fact'[Qty] + RELATED(Fact[Region])"
    );
    assert_eq!(
        rw("SUM('Order Lines'[Qty])", MEASURE, &t("Order Lines"), "Lines_2"),
        "SUM('Lines_2'[Qty])"
    );
}

#[test]
fn reserved_and_non_ascii_new_names_are_quoted() {
    assert_eq!(
        rw("Sales[Amount] + COUNTROWS(Sales)", MEASURE, &t("Sales"), "Date"),
        "'Date'[Amount] + COUNTROWS('Date')"
    );
    assert_eq!(
        rw("Sales[Amount] + COUNTROWS(Sales)", MEASURE, &t("Sales"), "Typed"),
        "'Typed'[Amount] + COUNTROWS('Typed')"
    );
    assert_eq!(
        rw("Sales[Amount]", MEASURE, &t("Sales"), "Status"),
        "'Status'[Amount]"
    );
    assert_eq!(rw("Sales[Amount]", MEASURE, &t("Sales"), "Été"), "'Été'[Amount]");
    assert_eq!(
        rw("Sales[Amount]", MEASURE, &t("Sales"), "Bob's"),
        "'Bob''s'[Amount]"
    );
    assert_eq!(
        rw("Sales[Amount]", MEASURE, &t("Sales"), "2024"),
        "'2024'[Amount]"
    );
}

#[test]
fn quotes_in_old_names_and_unicode_offsets() {
    assert_eq!(
        rw(
            "COUNTROWS('O''Brien') + 'o''brien'[Id]",
            MEASURE,
            &t("O'Brien"),
            "Irish"
        ),
        "COUNTROWS('Irish') + 'Irish'[Id]"
    );
    let src = "\"é€ — [Montant]\" & SUM('Ventes été'[Montant]) & \"€\" & [Total]";
    assert_eq!(
        rw(src, MEASURE, &c("Ventes été", "Montant"), "Montant €"),
        "\"é€ — [Montant]\" & SUM('Ventes été'[Montant €]) & \"€\" & [Total]"
    );
    assert_eq!(
        rw(src, MEASURE, &t("Ventes été"), "Ventes 2"),
        "\"é€ — [Montant]\" & SUM('Ventes 2'[Montant]) & \"€\" & [Total]"
    );
    assert_eq!(
        rw(src, MEASURE, &m("Sales", "Total"), "Totalité"),
        "\"é€ — [Montant]\" & SUM('Ventes été'[Montant]) & \"€\" & [Totalité]"
    );
}

#[test]
fn variables_shadow_table_names_even_quoted() {
    // Power BI Desktop 2.158: `VAR Product = 3 RETURN COUNTROWS('Product')` is an error, the
    // quoted name denotes the variable. A qualifier (`Sales[Qty]`) is always a table.
    let src =
        "VAR x = COUNTROWS(Sales)\nVAR Sales = 2\nRETURN x + Sales + COUNTROWS('Sales') + SUM(Sales[Qty])";
    assert_eq!(
        rw(src, MEASURE, &t("Sales"), "Fact"),
        "VAR x = COUNTROWS(Fact)\nVAR Sales = 2\nRETURN x + Sales + COUNTROWS('Sales') + SUM(Fact[Qty])"
    );
    // A new name that a variable in scope would capture is not written (counted as captured);
    // qualifiers are still rewritten.
    let model = model();
    let src = "VAR Fact = 1 RETURN COUNTROWS(Sales) + Fact + SUM(Sales[Qty])";
    assert_eq!(
        Renamer::new(&model).rewrite(src, MEASURE, &t("Sales"), "Fact"),
        Rewrite::Rewritten {
            text: "VAR Fact = 1 RETURN COUNTROWS(Sales) + Fact + SUM(Fact[Qty])".into(),
            references: 1,
            captured: 1
        }
    );
    assert_eq!(
        Renamer::new(&model).rewrite(
            "VAR fact = 1 RETURN COUNTROWS(Sales)",
            MEASURE,
            &t("Sales"),
            "Fact"
        ),
        Rewrite::Rewritten {
            text: "VAR fact = 1 RETURN COUNTROWS(Sales)".into(),
            references: 0,
            captured: 1
        }
    );
    // Outside the VAR block the variable is no longer in scope.
    let src = "(VAR Fact = 1 RETURN Fact) + COUNTROWS(Sales)";
    assert_eq!(
        rw(src, MEASURE, &t("Sales"), "Fact"),
        "(VAR Fact = 1 RETURN Fact) + COUNTROWS(Fact)"
    );
}

#[test]
fn whitespace_comments_and_casing_elsewhere_are_kept() {
    let src =
        "VAR  a =\r\n\tsum( Sales [Amount] ) -- note\r\nRETURN\r\n    a /* Sales[Amount] */ + Sales[amount]";
    assert_eq!(
        rw(src, MEASURE, &c("Sales", "Amount"), "Net"),
        "VAR  a =\r\n\tsum( Sales [Net] ) -- note\r\nRETURN\r\n    a /* Sales[Amount] */ + Sales[Net]"
    );
}

#[test]
fn field_parameters_variations_and_declared_columns() {
    let src = "{ (\"Amount\", NAMEOF('Sales'[Amount]), 0), (\"Qty\", NAMEOF('Sales'[Qty]), 1) }";
    let calc_table = Site::default();
    assert_eq!(
        rw(src, calc_table, &c("Sales", "Amount"), "Net Amount"),
        "{ (\"Amount\", NAMEOF('Sales'[Net Amount]), 0), (\"Qty\", NAMEOF('Sales'[Qty]), 1) }"
    );
    assert_eq!(
        rw(
            "COUNTROWS(VALUES(Sales[Date].[Year]))",
            MEASURE,
            &c("Sales", "Date"),
            "Order Date"
        ),
        "COUNTROWS(VALUES(Sales[Order Date].[Year]))"
    );
    // A column the expression declares itself is not the model column.
    let src = "SUMX(ADDCOLUMNS(VALUES(Product[Name]), \"Score\", 1), [Score])";
    assert_eq!(
        rw(src, MEASURE, &c("Product", "Name"), "Label"),
        src.replace("[Name]", "[Label]")
    );
}

#[test]
fn unparsable_and_untouched_expressions() {
    let model = model();
    let r = Renamer::new(&model);
    assert_eq!(
        r.rewrite("SUM(Sales[Amount]", MEASURE, &c("Sales", "Amount"), "Net"),
        Rewrite::Unparsable
    );
    assert_eq!(
        r.rewrite("\"Sales[Amount]\"", MEASURE, &c("Sales", "Amount"), "Net"),
        Rewrite::Unchanged
    );
    assert_eq!(
        r.rewrite("[Total] + [Total]", MEASURE, &m("Sales", "Total"), "T2"),
        Rewrite::Rewritten {
            text: "[T2] + [T2]".into(),
            references: 2,
            captured: 0
        }
    );
    // Hostile input never panics.
    for src in [
        "",
        "'",
        "[",
        "\u{0}",
        "((((((((",
        "VAR",
        "'a''",
        "[]]]",
        "€[",
        "Sales[Amount].[",
    ] {
        let _ = r.rewrite(src, MEASURE, &t("Sales"), "X");
    }
    let deep = format!("{}1{}", "(".repeat(5000), ")".repeat(5000));
    let _ = r.rewrite(&deep, MEASURE, &t("Sales"), "X");
}

#[test]
fn references_resolve_like_the_rewrite() {
    let model = model();
    let r = Renamer::new(&model);
    let refs = r
        .references(
            "SUMX('Order Lines', [Qty]) + [Total] + COUNTROWS(Product)",
            MEASURE,
        )
        .unwrap_or_default();
    assert_eq!(
        refs,
        vec![
            m("Sales", "Total"),
            c("Order Lines", "Qty"),
            t("Order Lines"),
            t("Product")
        ]
    );
    assert!(r.references("SUM(", MEASURE).is_none());
}

#[test]
fn moving_a_measure_requalifies_its_references() {
    let model = model();
    let r = Renamer::new(&model);
    let mv = |text: &str, to: &str| match r.move_measure(text, MEASURE, &m("Sales", "Total"), to) {
        Rewrite::Rewritten { text, .. } => text,
        Rewrite::Unchanged => text.to_string(),
        Rewrite::Unparsable => "<unparsable>".into(),
    };
    assert_eq!(
        mv(
            "Sales[Total] + 'Sales'[Total] + [Total] + Sales[Amount] // Sales[Total]",
            "Order Lines"
        ),
        "'Order Lines'[Total] + 'Order Lines'[Total] + [Total] + Sales[Amount] // Sales[Total]"
    );
    assert_eq!(mv("sales[TOTAL] * 2", "Product"), "Product[TOTAL] * 2");
    // A reserved word is quoted as a qualifier.
    assert_eq!(mv("Sales[Total]", "Date"), "'Date'[Total]");
    assert_eq!(mv("Sales[Lines]", "Product"), "Sales[Lines]");
    assert!(matches!(
        r.move_measure("Sales[Total", MEASURE, &m("Sales", "Total"), "Product"),
        Rewrite::Unparsable
    ));
    assert!(matches!(
        r.move_measure("1", MEASURE, &c("Sales", "Qty"), "Product"),
        Rewrite::Unchanged
    ));
}

#[test]
fn user_defined_functions_are_rewritten_with_parameters_as_variables() {
    let udf = Site {
        home: None,
        row_context: false,
        udf: true,
    };
    let src = "(Sales : TABLE, k : SCALAR NUMERIC VAL = COUNTROWS(Product)) =>\n    SUMX(Sales, [Qty]) + SUM(Sales[Amount]) * k + [Total] + COUNTROWS(Product)";
    assert_eq!(
        rw(src, udf, &t("Product"), "Goods"),
        "(Sales : TABLE, k : SCALAR NUMERIC VAL = COUNTROWS(Goods)) =>\n    SUMX(Sales, [Qty]) + SUM(Sales[Amount]) * k + [Total] + COUNTROWS(Goods)"
    );
    // `Sales` is the parameter, except as a column qualifier.
    assert_eq!(
        rw(src, udf, &t("Sales"), "Fact"),
        "(Sales : TABLE, k : SCALAR NUMERIC VAL = COUNTROWS(Product)) =>\n    SUMX(Sales, [Qty]) + SUM(Fact[Amount]) * k + [Total] + COUNTROWS(Product)"
    );
    assert_eq!(
        rw("(x) => [Total] * x", udf, &m("Sales", "Total"), "Revenue"),
        "(x) => [Revenue] * x"
    );
    assert_eq!(rw("(x) => x +", udf, &m("Sales", "Total"), "R"), "<unparsable>");
    let model = model();
    let refs = Renamer::new(&model)
        .references("() => SUM(Sales[Amount])", udf)
        .unwrap();
    assert_eq!(refs, vec![c("Sales", "Amount"), t("Sales")]);
}
