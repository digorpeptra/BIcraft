//! Names of the M standard library, for `m.validate`: which dotted names are library functions
//! or constants (so a reference to one is not an undefined name), which of them bicraft's
//! evaluator runs, and which are external connectors (never run).
//!
//! The documented list is the public Power Query M function reference
//! (learn.microsoft.com/powerquery-m/power-query-m-function-reference), grouped by namespace.
//! It does not need to be complete: a dotted name that is in no list is reported as an unknown
//! library function, a warning, never an error.

use std::collections::HashSet;
use std::sync::OnceLock;

/// What a dotted (or `#`) name is in the M standard library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryName {
    /// A function or constant bicraft's evaluator runs.
    Evaluated,
    /// A documented library function or constant bicraft's evaluator does not run.
    Documented,
    /// An external connector (database, web service, script): documented, never run.
    Connector,
    /// Not a name bicraft knows.
    Unknown,
}

/// Documented functions, one namespace per line: `Namespace: Name Name …`.
const DOCUMENTED: &str = "\
Access: Database
ActiveDirectory: Domains
AdobeAnalytics: Cubes
AdoDotNet: DataSource Query
AnalysisServices: Database Databases
Any: Type
AzureStorage: BlobContents Blobs DataLake DataLakeContents Tables
Binary: ApproximateLength Buffer Combine Compress Decompress From FromList FromText InferContentType Length Range Split ToList ToText View ViewError ViewFunction
BinaryFormat: 7BitEncodedSignedInteger 7BitEncodedUnsignedInteger Binary Byte ByteOrder Choice Decimal Double Group Length List Null Record SignedInteger16 SignedInteger32 SignedInteger64 Single Text Transform UnsignedInteger16 UnsignedInteger32 UnsignedInteger64
Byte: From
Character: FromNumber ToNumber
Combiner: CombineTextByDelimiter CombineTextByEachDelimiter CombineTextByLengths CombineTextByPositions CombineTextByRanges
Comparer: Equals FromCulture Ordinal OrdinalIgnoreCase
Csv: Document
Cube: AddAndExpandDimensionColumn AddMeasureColumn ApplyParameter AttributeMemberId AttributeMemberProperty CollapseAndRemoveColumns Dimensions DisplayFolders MeasureProperties MeasureProperty Measures Parameters Properties PropertyKey ReplaceDimensions Transform
Currency: From
Databricks: Catalogs Contents Query
DatabricksMultiCloud: Catalogs
Date: AddDays AddMonths AddQuarters AddWeeks AddYears Day DayOfWeek DayOfWeekName DayOfYear DaysInMonth EndOfDay EndOfMonth EndOfQuarter EndOfWeek EndOfYear From FromText IsInCurrentDay IsInCurrentMonth IsInCurrentQuarter IsInCurrentWeek IsInCurrentYear IsInNextDay IsInNextMonth IsInNextNDays IsInNextNMonths IsInNextNQuarters IsInNextNWeeks IsInNextNYears IsInNextQuarter IsInNextWeek IsInNextYear IsInPreviousDay IsInPreviousMonth IsInPreviousNDays IsInPreviousNMonths IsInPreviousNQuarters IsInPreviousNWeeks IsInPreviousNYears IsInPreviousQuarter IsInPreviousWeek IsInPreviousYear IsInYearToDate IsLeapYear Month MonthName QuarterOfYear StartOfDay StartOfMonth StartOfQuarter StartOfWeek StartOfYear ToRecord ToText WeekOfMonth WeekOfYear Year
DateTime: AddZone Date FixedLocalNow From FromFileTime FromText IsInCurrentHour IsInCurrentMinute IsInCurrentSecond IsInNextHour IsInNextMinute IsInNextNHours IsInNextNMinutes IsInNextNSeconds IsInNextSecond IsInPreviousHour IsInPreviousMinute IsInPreviousNHours IsInPreviousNMinutes IsInPreviousNSeconds IsInPreviousSecond LocalNow Time ToRecord ToText
DateTimeZone: FixedLocalNow FixedUtcNow From FromFileTime FromText LocalNow RemoveZone SwitchZone ToLocal ToRecord ToText ToUtc UtcNow ZoneHours ZoneMinutes
Decimal: From
Diagnostics: ActivityId Trace
Double: From
Duration: Days From FromText Hours Minutes Seconds ToRecord ToText TotalDays TotalHours TotalMinutes TotalSeconds
Embedded: Value
Error: Record
Excel: CurrentWorkbook ShapeTable Workbook
Exchange: Contents
Expression: Constant Evaluate Identifier
Facebook: Graph
File: Contents
Folder: Contents Files
Function: From Invoke InvokeAfter InvokeWithErrorContext IsDataSource ScalarVector
GoogleAnalytics: Accounts
Hdfs: Contents Files
HdInsight: Containers Contents Files
Html: Table
Identity: From IsMemberOf
IdentityProvider: Default
Int16: From
Int32: From
Int64: From
Int8: From
ItemExpression: From Item
Json: Document FromValue
Lines: FromBinary FromText ToBinary ToText
List: Accumulate AllTrue Alternate AnyTrue Average Buffer Combine ConformToPageReader Contains ContainsAll ContainsAny Count Covariance Dates DateTimes DateTimeZones Difference Distinct Durations FindText First FirstN Generate InsertRange Intersect IsDistinct IsEmpty Last LastN MatchesAll MatchesAny Max MaxN Median Min MinN Mode Modes NonNullCount Numbers Percentile PositionOf PositionOfAny Positions Product Random Range RemoveFirstN RemoveItems RemoveLastN RemoveMatchingItems RemoveNulls RemoveRange Repeat ReplaceMatchingItems ReplaceRange ReplaceValue Reverse Select Single SingleOrDefault Skip Sort Split StandardDeviation Sum Times Transform TransformMany Union Zip
Logical: From FromText ToText
MySQL: Database
Number: Abs Acos Asin Atan Atan2 BitwiseAnd BitwiseNot BitwiseOr BitwiseShiftLeft BitwiseShiftRight BitwiseXor Combinations Cos Cosh Exp Factorial From FromText IntegerDivide IsEven IsNaN IsOdd Ln Log Log10 Mod Permutations Power Random RandomBetween Round RoundAwayFromZero RoundDown RoundTowardZero RoundUp Sign Sin Sinh Sqrt Tan Tanh ToText
OData: Feed
Odbc: DataSource InferOptions Query
OleDb: DataSource Query
Oracle: Database
Parquet: Document
Pdf: Tables
Percentage: From
PostgreSQL: Database
Progress: DataSourceProgress
RData: FromBinary
Record: AddField Combine Field FieldCount FieldNames FieldOrDefault FieldValues FromList FromTable HasFields RemoveFields RenameFields ReorderFields SelectFields ToList ToTable TransformFields
Replacer: ReplaceText ReplaceValue
RowExpression: Column From Row
Salesforce: Data Reports
SapBusinessWarehouse: Cubes
SapHana: Database
SharePoint: Contents Files Tables
Single: From
Splitter: SplitByNothing SplitTextByAnyDelimiter SplitTextByCharacterTransition SplitTextByDelimiter SplitTextByEachDelimiter SplitTextByLengths SplitTextByPositions SplitTextByRanges SplitTextByRepeatedLengths SplitTextByWhitespace
Sql: Database Databases
Sybase: Database
Table: AddColumn AddFuzzyClusterColumn AddIndexColumn AddJoinColumn AddKey AddRankColumn AggregateTableColumn AlternateRows ApproximateRowCount Buffer Column ColumnCount ColumnNames ColumnsOfType Combine CombineColumns CombineColumnsToRecord ConformToPageReader Contains ContainsAll ContainsAny DemoteHeaders Distinct DuplicateColumn ExpandListColumn ExpandRecordColumn ExpandTableColumn FillDown FillUp FilterWithDataTable FindText First FirstN FirstValue FromColumns FromList FromPartitions FromRecords FromRows FromValue FuzzyGroup FuzzyJoin FuzzyNestedJoin Group HasColumns InsertRows IsDistinct IsEmpty Join Keys Last LastN MatchesAllRows MatchesAnyRows Max MaxN Min MinN NestedJoin Partition PartitionValues Pivot PositionOf PositionOfAny PrefixColumns Profile PromoteHeaders Range RemoveColumns RemoveFirstN RemoveLastN RemoveMatchingRows RemoveRows RemoveRowsWithErrors RenameColumns ReorderColumns Repeat ReplaceErrorValues ReplaceKeys ReplaceMatchingRows ReplaceRelationshipIdentity ReplaceRows ReplaceValue ReverseRows RowCount Schema SelectColumns SelectRows SelectRowsWithErrors SingleRow Skip Sort Split SplitAt SplitColumn StopFolding ToColumns ToList ToRecords ToRows TransformColumnNames TransformColumns TransformColumnTypes TransformRows Transpose Unpivot UnpivotOtherColumns View ViewError ViewFunction
Tables: GetRelationships
Teradata: Database
Text: AfterDelimiter At BeforeDelimiter BetweenDelimiters Clean Combine Contains End EndsWith Format From FromBinary InferNumberType Insert Length Lower Middle NewGuid PadEnd PadStart PositionOf PositionOfAny Proper Range Remove RemoveRange Repeat Replace ReplaceRange Reverse Select Split SplitAny Start StartsWith ToBinary ToList Trim TrimEnd TrimStart Upper
Time: EndOfHour From FromText Hour Minute Second StartOfHour ToRecord ToText
Type: AddTableKey ClosedRecord Facets ForFunction ForRecord FunctionParameters FunctionRequiredParameters FunctionReturn Is IsNullable IsOpenRecord ListItem NonNullable OpenRecord RecordFields ReplaceFacets ReplaceTableKeys TableColumn TableKeys TableRow TableSchema Union
Uri: BuildQueryString Combine EscapeDataString Parts
Value: Add Alternates As Compare Divide Equals Expression Firewall FromText Is Lineage Metadata Multiply NativeQuery NullableEquals Optimize RemoveMetadata ReplaceMetadata ReplaceType Subtract Traits Type VersionIdentity Versions ViewError ViewFunction
Variable: Value ValueOrDefault
Web: BrowserContents Contents Headers Page
Xml: Document Tables
";

/// Enumeration namespaces of the library (`Order.Ascending`, `JoinKind.Inner`, …): any member is
/// accepted as a documented constant.
const ENUMS: &[&str] = &[
    "AccessControlKind",
    "BinaryEncoding",
    "BinaryOccurrence",
    "BufferMode",
    "ByteOrder",
    "Compression",
    "CsvStyle",
    "Culture",
    "Day",
    "ExtraValues",
    "GroupKind",
    "JoinAlgorithm",
    "JoinKind",
    "JoinSide",
    "LimitClauseKind",
    "MissingField",
    "Occurrence",
    "ODataOmitValues",
    "Order",
    "PercentileMode",
    "Precision",
    "QuoteStyle",
    "RankKind",
    "RelativePosition",
    "RoundingMode",
    "SapBusinessWarehouseExecutionMode",
    "SapHanaDistribution",
    "SapHanaRangeOperator",
    "TextEncoding",
    "TraceLevel",
    "WebMethod",
];

/// `#` intrinsics.
const HASH: &[&str] = &[
    "#binary",
    "#date",
    "#datetime",
    "#datetimezone",
    "#duration",
    "#sections",
    "#shared",
    "#table",
    "#time",
];

fn documented() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        let mut set = HashSet::new();
        for line in DOCUMENTED.lines() {
            let Some((ns, names)) = line.split_once(':') else {
                continue;
            };
            for n in names.split_whitespace() {
                set.insert(format!("{ns}.{n}"));
            }
        }
        set
    })
}

fn evaluated() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| super::lib::table().into_keys().collect())
}

/// Classifies a library name (`Table.SelectRows`, `#date`, `Int64.Type`, `Sql.Database`).
pub fn library_name(name: &str) -> LibraryName {
    if evaluated().contains(name) {
        return LibraryName::Evaluated;
    }
    if super::lib::files::external(name).is_some() && name.contains('.') {
        return LibraryName::Connector;
    }
    if HASH.contains(&name) || documented().contains(name) {
        return LibraryName::Documented;
    }
    if let Some((ns, member)) = name.split_once('.') {
        // Type names (`Int64.Type`, `Text.Type`) and enumeration members.
        if (member == "Type" && !ns.is_empty()) || (ENUMS.contains(&ns) && !member.is_empty()) {
            return LibraryName::Documented;
        }
    }
    LibraryName::Unknown
}

/// The M keywords, which an identifier must quote (`#"let"`).
const KEYWORDS: &[&str] = &[
    "and",
    "as",
    "each",
    "else",
    "error",
    "false",
    "if",
    "in",
    "is",
    "let",
    "meta",
    "not",
    "null",
    "or",
    "otherwise",
    "section",
    "shared",
    "then",
    "true",
    "try",
    "type",
    "catch",
];

/// `name` as an M identifier: as written when it is a regular identifier (letters, digits and
/// `_`, not starting with a digit, dotted parts allowed, not a keyword), else `#"…"` with `"`
/// doubled.
pub fn identifier(name: &str) -> String {
    let regular = !name.is_empty()
        && !KEYWORDS.contains(&name)
        && name.split('.').all(|part| {
            let mut chars = part.chars();
            chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
                && chars.all(|c| c.is_alphanumeric() || c == '_')
        });
    if regular {
        name.to_string()
    } else {
        format!("#\"{}\"", name.replace('"', "\"\""))
    }
}
