#!/usr/bin/env python3
"""Regenerate tests/fixtures/synthetic_metadata.sqlitedb for bicraft-metadata.

A tiny metadata database shaped like the TMSCHEMA tables of an Analysis Services
`metadata.sqlitedb`: bracket-quoted names, INTEGER/TEXT/REAL columns, and `ID` as an
`INTEGER` primary key declared as a table constraint. Only a subset of the columns is
created; the reader must cope with missing ones. All data is invented for these tests.

Usage:  python crates/metadata/tests/gen_fixture.py
"""

import os
import sqlite3

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "fixtures", "synthetic_metadata.sqlitedb")

SCHEMA = {
    "Model": "ID INTEGER, Name TEXT, Culture TEXT, DefaultMode INTEGER, "
    "DefaultPowerBIDataSourceVersion INTEGER, DataAccessOptions TEXT, ValueFilterBehavior INTEGER",
    "Table": "ID INTEGER, ModelID INTEGER, Name TEXT, IsHidden INTEGER, SystemFlags INTEGER, "
    "LineageTag TEXT, Description TEXT",
    "Column": "ID INTEGER, TableID INTEGER, ExplicitName TEXT, InferredName TEXT, "
    "ExplicitDataType INTEGER, InferredDataType INTEGER, IsHidden INTEGER, IsUnique INTEGER, "
    "IsKey INTEGER, IsNullable INTEGER, SummarizeBy INTEGER, Type INTEGER, SourceColumn TEXT, "
    "Expression TEXT, FormatString TEXT, SortByColumnID INTEGER, LineageTag TEXT",
    "Measure": "ID INTEGER, TableID INTEGER, Name TEXT, Description TEXT, DataType INTEGER, "
    "Expression TEXT, FormatString TEXT, IsHidden INTEGER, State INTEGER, ModifiedTime INTEGER, "
    "StructureModifiedTime INTEGER, KPIID INTEGER, IsSimpleMeasure INTEGER, ErrorMessage TEXT, "
    "DisplayFolder TEXT, DetailRowsDefinitionID INTEGER, DataCategory TEXT, "
    "FormatStringDefinitionID INTEGER, LineageTag TEXT, SourceLineageTag TEXT",
    "Partition": "ID INTEGER, TableID INTEGER, Name TEXT, QueryDefinition TEXT, Type INTEGER, "
    "Mode INTEGER, DataView INTEGER, RangeStart REAL",
    "Relationship": "ID INTEGER, ModelID INTEGER, Name TEXT, IsActive INTEGER, "
    "CrossFilteringBehavior INTEGER, FromTableID INTEGER, FromColumnID INTEGER, "
    "FromCardinality INTEGER, ToTableID INTEGER, ToColumnID INTEGER, ToCardinality INTEGER",
    "Expression": "ID INTEGER, ModelID INTEGER, Name TEXT, Kind INTEGER, Expression TEXT",
    "Annotation": "ID INTEGER, ObjectID INTEGER, ObjectType INTEGER, Name TEXT, Value TEXT, "
    "ModifiedTime INTEGER",
    "Role": "ID INTEGER, ModelID INTEGER, Name TEXT, Description TEXT, ModelPermission INTEGER, "
    "ModifiedTime INTEGER",
    "TablePermission": "ID INTEGER, RoleID INTEGER, TableID INTEGER, FilterExpression TEXT, "
    "ModifiedTime INTEGER, State INTEGER, ErrorMessage TEXT, MetadataPermission INTEGER",
    "RoleMembership": "ID INTEGER, RoleID INTEGER, MemberName TEXT, MemberID TEXT, "
    "IdentityProvider TEXT, MemberType INTEGER, ModifiedTime INTEGER",
    # Storage table: ignored by the reader.
    "StorageFile": "ID INTEGER, OwnerID INTEGER, OwnerType INTEGER, FileName TEXT",
}

# Windows FILETIME ticks (100 ns since 1601-01-01), as the engine stores ModifiedTime.
T0 = 133800000000000000
T1 = 133800000010000000

M_QUERY = 'let\n    Source = #table({"Amount"}, {{1.5}})\nin\n    Source'

ROWS = {
    "Model": [(1, "Model", "en-US", 0, 2, '{"legacyRedirects": true}', 1)],
    "Table": [
        (3, 1, "Sales", 0, 0, "lt-sales", None),
        (2, 1, "Calendar", 1, 2, None, "Days"),
        (50, 0, "H$Sales (3)$Amount (4)", 1, 1, None, None),
    ],
    "Column": [
        (4, 3, "Amount", None, 8, None, 0, 0, 0, 1, 3, 1, "Amount", None, "0.0", 0, "lt-amount"),
        (5, 3, "RowNumber-0", None, 6, 19, 1, 1, 1, 0, 1, 3, None, None, None, 0, None),
        (6, 3, "Day", None, 9, None, 0, 0, 0, 1, 1, 1, "Day", None, None, 0, None),
        (7, 2, None, "Date", 1, 9, 0, 0, 0, 1, 2, 4, "[Date]", None, None, 0, None),
        (8, 2, "Month", None, 2, 2, 0, 0, 0, 1, 2, 2, None, "FORMAT([Date], \"MMM\")", None, 7, None),
        (51, 50, "POS_TO_ID", None, 6, None, 0, 0, 0, 1, 1, 1, None, None, None, 0, None),
    ],
    "Measure": [
        (10, 3, "Total", None, 8, "SUM(Sales[Amount])", "#,0.0", 0, 1, T1, T0, 0, 0, None, "KPIs",
         0, None, 0, "lt-total", None),
        (9, 3, "Rows", "Row count", 6, "COUNTROWS(Sales)", None, 1, 1, T1, T0, 0, 0, None, None,
         0, None, 0, None, None),
    ],
    "Partition": [
        (11, 3, "Sales", M_QUERY, 4, 0, 3, 0.0),
        (12, 2, "Calendar", "CALENDAR(DATE(2020,1,1), DATE(2020,12,31))", 2, 0, 3, 0.0),
        (52, 50, "sys", None, 3, 2, 3, 0.0),
    ],
    "Relationship": [(13, 1, "rel-day", 1, 1, 3, 6, 2, 2, 7, 1)],
    "Expression": [(14, 1, "Rate", 0, "1.2 meta [IsParameterQuery=true]")],
    "Annotation": [
        (15, 1, 1, "PBI_QueryOrder", '["Sales"]', T0),
        (16, 3, 3, "PBI_ResultType", "Table", T0),
        (17, 7, 4, "SummarizationSetBy", "Automatic", T0),
        (19, 10, 8, "PBI_FormatHint", '{"isGeneralNumber":true}', T0),
    ],
    "Role": [(20, 1, "Readers", None, 2, T0)],
    "TablePermission": [(21, 20, 3, "Sales[Amount] > 0", T0, 1, None, 0)],
    "RoleMembership": [(22, 20, "someone@example.com", None, None, 2, T0)],
    "StorageFile": [(18, 4, 4, "Amount.idf")],
}

DBPROPERTIES = [("MAXID", 52), ("SCHEMAVERSION", 123)]


def main() -> None:
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    if os.path.exists(OUT):
        os.remove(OUT)
    con = sqlite3.connect(OUT)
    con.execute("PRAGMA page_size = 4096")
    for table, cols in SCHEMA.items():
        decl = " ,  ".join(f"[{c.split()[0]}] {c.split()[1]}" for c in cols.split(", "))
        con.execute(f'CREATE TABLE [{table}]( {decl} ,  PRIMARY KEY("ID" ASC) )')
        for row in ROWS.get(table, []):
            marks = ", ".join("?" for _ in row)
            con.execute(f"INSERT INTO [{table}] VALUES ({marks})", row)
    # The engine's ID counter: a name/value table without a rowid alias.
    con.execute("CREATE TABLE [DBPROPERTIES] ([NAME] STRING, [VALUE] INTEGER, PRIMARY KEY ([NAME] ASC))")
    for name, value in DBPROPERTIES:
        con.execute("INSERT INTO [DBPROPERTIES] VALUES (?, ?)", (name, value))
    con.commit()
    con.execute("VACUUM")
    con.close()


if __name__ == "__main__":
    main()
