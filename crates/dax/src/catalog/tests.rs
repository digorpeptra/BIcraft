use super::*;
use crate::functions::{Arity, arity, declares_columns, is_keyword_argument, lookup, signature};
use crate::types::{Param, ParamKind, Returns};

/// Argument counts reported by Power BI Desktop's engine (2.158.1304.0, `Too few/many arguments`
/// errors for 0 and 40 arguments), and whether `$SYSTEM.MDSCHEMA_FUNCTIONS` lists the function.
/// `None` max: the 40-argument call raised no count error.
#[rustfmt::skip]
const ENGINE: &[(&str, u8, Option<u8>, bool)] = &[
    ("ABS", 1, Some(1), true), ("ACCRINT", 6, Some(8), true), ("ACCRINTM", 4, Some(5), true),
    ("ACOS", 1, Some(1), true), ("ACOSH", 1, Some(1), true), ("ACOT", 1, Some(1), true),
    ("ACOTH", 1, Some(1), true), ("ADDCOLUMNS", 3, None, true), ("ADDMISSINGITEMS", 1, None, true),
    ("ALL", 0, None, true), ("ALLCROSSFILTERED", 1, Some(1), true), ("ALLEXCEPT", 2, None, true),
    ("ALLNOBLANKROW", 1, None, true), ("ALLSELECTED", 0, None, true),
    ("AMORDEGRC", 6, Some(7), true), ("AMORLINC", 6, Some(7), true), ("AND", 2, Some(2), true),
    ("APPROXIMATEDISTINCTCOUNT", 1, Some(1), true), ("ASIN", 1, Some(1), true),
    ("ASINH", 1, Some(1), true), ("ATAN", 1, Some(1), true), ("ATANH", 1, Some(1), true),
    ("AVERAGE", 1, Some(1), true), ("AVERAGEA", 1, Some(1), true), ("AVERAGEX", 2, Some(2), true),
    ("BETA.DIST", 4, Some(6), true), ("BETA.INV", 3, Some(5), true), ("BITAND", 2, Some(2), true),
    ("BITLSHIFT", 2, Some(2), true), ("BITOR", 2, Some(2), true), ("BITRSHIFT", 2, Some(2), true),
    ("BITXOR", 2, Some(2), true), ("BLANK", 0, Some(0), true), ("CALCULATE", 1, None, true),
    ("CALCULATETABLE", 1, None, true), ("CALENDAR", 2, Some(2), true),
    ("CALENDARAUTO", 0, Some(1), true), ("CEILING", 2, Some(2), true),
    ("CHISQ.DIST", 3, Some(3), true), ("CHISQ.DIST.RT", 2, Some(2), true),
    ("CHISQ.INV", 2, Some(2), true), ("CHISQ.INV.RT", 2, Some(2), true),
    ("CLOSINGBALANCEMONTH", 2, Some(3), true), ("CLOSINGBALANCEQUARTER", 2, Some(3), true),
    ("CLOSINGBALANCEWEEK", 2, Some(3), true), ("CLOSINGBALANCEYEAR", 2, Some(4), true),
    ("COALESCE", 2, None, true), ("COLLAPSE", 1, None, true), ("COLLAPSEALL", 1, Some(2), true),
    ("COLUMNSTATISTICS", 0, Some(0), true), ("COMBIN", 2, Some(2), true),
    ("COMBINA", 2, Some(2), true), ("COMBINEVALUES", 3, None, true),
    ("CONCATENATE", 2, Some(2), true), ("CONCATENATEX", 2, None, true),
    ("CONFIDENCE.NORM", 3, Some(3), true), ("CONFIDENCE.T", 3, Some(3), true),
    ("CONTAINS", 3, None, true), ("CONTAINSROW", 2, None, true),
    ("CONTAINSSTRING", 2, Some(2), true), ("CONTAINSSTRINGEXACT", 2, Some(2), true),
    ("CONVERT", 2, Some(2), true), ("COS", 1, Some(1), true), ("COSH", 1, Some(1), true),
    ("COT", 1, Some(1), true), ("COTH", 1, Some(1), true), ("COUNT", 1, Some(1), true),
    ("COUNTA", 1, Some(1), true), ("COUNTAX", 2, Some(2), true), ("COUNTBLANK", 1, Some(1), true),
    ("COUNTROWS", 0, Some(1), true), ("COUNTX", 2, Some(2), true), ("COUPDAYBS", 3, Some(4), true),
    ("COUPDAYS", 3, Some(4), true), ("COUPDAYSNC", 3, Some(4), true), ("COUPNCD", 3, Some(4), true),
    ("COUPNUM", 3, Some(4), true), ("COUPPCD", 3, Some(4), true), ("CROSSFILTER", 3, Some(3), true),
    ("CROSSJOIN", 2, None, true), ("CUMIPMT", 6, Some(6), true), ("CUMPRINC", 6, Some(6), true),
    ("CURRENCY", 1, Some(1), true), ("CURRENTGROUP", 0, Some(0), true),
    ("CUSTOMDATA", 0, Some(0), true), ("DATATABLE", 3, None, true), ("DATE", 3, Some(3), true),
    ("DATEADD", 3, Some(5), true), ("DATEDIFF", 3, Some(3), true),
    ("DATESBETWEEN", 3, Some(3), true), ("DATESINPERIOD", 4, Some(5), true),
    ("DATESMTD", 1, Some(1), true), ("DATESQTD", 1, Some(1), true), ("DATESWTD", 1, Some(1), true),
    ("DATESYTD", 1, Some(2), true), ("DATEVALUE", 1, Some(1), true), ("DAY", 1, Some(1), true),
    ("DB", 4, Some(5), true), ("DDB", 4, Some(5), true), ("DEGREES", 1, Some(1), true),
    ("DETAILROWS", 1, Some(1), true), ("DISC", 4, Some(5), true), ("DISTINCT", 1, Some(1), true),
    ("DISTINCTCOUNT", 1, Some(1), true), ("DISTINCTCOUNTNOBLANK", 1, Some(1), true),
    ("DIVIDE", 2, Some(3), true), ("DOLLARDE", 2, Some(2), true), ("DOLLARFR", 2, Some(2), true),
    ("DURATION", 5, Some(6), true), ("EARLIER", 1, Some(2), true), ("EARLIEST", 1, Some(1), true),
    ("EDATE", 2, Some(2), true), ("EFFECT", 2, Some(2), true), ("ENDOFMONTH", 1, Some(1), true),
    ("ENDOFQUARTER", 1, Some(1), true), ("ENDOFWEEK", 1, Some(1), true),
    ("ENDOFYEAR", 1, Some(2), true), ("EOMONTH", 2, Some(2), true), ("ERROR", 1, Some(1), true),
    ("EVALUATEANDLOG", 1, Some(3), true), ("EVEN", 1, Some(1), true), ("EXACT", 2, Some(2), true),
    ("EXCEPT", 2, Some(2), true), ("EXP", 1, Some(1), true), ("EXPAND", 1, None, true),
    ("EXPANDALL", 1, Some(2), true), ("EXPON.DIST", 3, Some(3), true),
    ("EXTERNALMEASURE", 1, Some(3), true), ("FACT", 1, Some(1), true), ("FALSE", 0, None, true),
    ("FILTER", 2, Some(2), true), ("FILTERS", 1, Some(1), true), ("FIND", 2, Some(4), true),
    ("FIRST", 1, Some(5), true), ("FIRSTDATE", 1, Some(1), true),
    ("FIRSTNONBLANK", 2, Some(2), true), ("FIRSTNONBLANKVALUE", 2, Some(2), true),
    ("FIXED", 1, Some(3), true), ("FLOOR", 2, Some(2), true), ("FORMAT", 2, Some(3), true),
    ("FV", 3, Some(5), true), ("GCD", 2, Some(2), true), ("GENERATE", 2, Some(2), true),
    ("GENERATEALL", 2, Some(2), true), ("GENERATESERIES", 2, Some(3), true),
    ("GEOMEAN", 1, Some(1), true), ("GEOMEANX", 2, Some(2), true), ("GROUPBY", 2, None, true),
    ("HASONEFILTER", 1, Some(1), true), ("HASONEVALUE", 1, Some(1), true),
    ("HOUR", 1, Some(1), true), ("IF", 2, Some(3), true), ("IF.EAGER", 2, Some(3), true),
    ("IFERROR", 2, Some(2), true), ("IGNORE", 1, Some(1), true), ("INDEX", 1, Some(7), true),
    ("INFO.ALTERNATEOFDEFINITIONS", 0, None, true), ("INFO.ANNOTATIONS", 0, None, true),
    ("INFO.ATTRIBUTEHIERARCHIES", 0, None, true),
    ("INFO.ATTRIBUTEHIERARCHYSTORAGES", 0, None, true), ("INFO.CALCDEPENDENCY", 0, None, true),
    ("INFO.CALCULATIONGROUPS", 0, None, true), ("INFO.CALCULATIONITEMS", 0, None, true),
    ("INFO.CALENDARCOLUMNGROUPS", 0, None, true), ("INFO.CALENDARCOLUMNREFERENCES", 0, None, true),
    ("INFO.CALENDARS", 0, None, true), ("INFO.CATALOGS", 0, None, true),
    ("INFO.CHANGEDPROPERTIES", 0, None, true), ("INFO.COLUMNPARTITIONSTORAGES", 0, None, true),
    ("INFO.COLUMNPERMISSIONS", 0, None, true), ("INFO.COLUMNS", 0, None, true),
    ("INFO.COLUMNSTORAGES", 0, None, true), ("INFO.CSDLMETADATA", 0, None, true),
    ("INFO.CULTURES", 0, None, true), ("INFO.DATACOVERAGEDEFINITIONS", 0, None, true),
    ("INFO.DATASOURCES", 0, None, true), ("INFO.DELTATABLEMETADATASTORAGES", 0, None, true),
    ("INFO.DEPENDENCIES", 0, None, true), ("INFO.DETAILROWSDEFINITIONS", 0, None, true),
    ("INFO.DICTIONARYSTORAGES", 0, None, true), ("INFO.EXCLUDEDARTIFACTS", 0, None, true),
    ("INFO.EXPRESSIONS", 0, None, true), ("INFO.EXTENDEDPROPERTIES", 0, None, true),
    ("INFO.FORMATSTRINGDEFINITIONS", 0, None, true), ("INFO.FUNCTIONS", 0, None, true),
    ("INFO.GENERALSEGMENTMAPSEGMENTMETADATASTORAGES", 0, None, true),
    ("INFO.GROUPBYCOLUMNS", 0, None, true), ("INFO.HIERARCHIES", 0, None, true),
    ("INFO.HIERARCHYSTORAGES", 0, None, true), ("INFO.KPIS", 0, None, true),
    ("INFO.LEVELS", 0, None, true), ("INFO.LINGUISTICMETADATA", 0, None, true),
    ("INFO.MEASURES", 0, None, true), ("INFO.MODEL", 0, None, true),
    ("INFO.OBJECTTRANSLATIONS", 0, None, true), ("INFO.PARQUETFILESTORAGES", 0, None, true),
    ("INFO.PARTITIONS", 0, None, true), ("INFO.PARTITIONSTORAGES", 0, None, true),
    ("INFO.PERSPECTIVECOLUMNS", 0, None, true), ("INFO.PERSPECTIVEHIERARCHIES", 0, None, true),
    ("INFO.PERSPECTIVEMEASURES", 0, None, true), ("INFO.PERSPECTIVES", 0, None, true),
    ("INFO.PERSPECTIVETABLES", 0, None, true), ("INFO.PROPERTIES", 0, None, true),
    ("INFO.QUERYGROUPS", 0, None, true), ("INFO.REFRESHPOLICIES", 0, None, true),
    ("INFO.RELATEDCOLUMNDETAILS", 0, None, true), ("INFO.RELATIONSHIPINDEXSTORAGES", 0, None, true),
    ("INFO.RELATIONSHIPS", 0, None, true), ("INFO.RELATIONSHIPSTORAGES", 0, None, true),
    ("INFO.ROLEMEMBERSHIPS", 0, None, true), ("INFO.ROLES", 0, None, true),
    ("INFO.SEGMENTMAPSTORAGES", 0, None, true), ("INFO.SEGMENTSTORAGES", 0, None, true),
    ("INFO.STORAGEFILES", 0, None, true), ("INFO.STORAGEFOLDERS", 0, None, true),
    ("INFO.STORAGETABLECOLUMNS", 0, None, true), ("INFO.STORAGETABLECOLUMNSEGMENTS", 0, None, true),
    ("INFO.STORAGETABLES", 0, None, true), ("INFO.TABLEPERMISSIONS", 0, None, true),
    ("INFO.TABLES", 0, None, true), ("INFO.TABLESTORAGES", 0, None, true),
    ("INFO.USERDEFINEDFUNCTIONS", 0, None, true), ("INFO.VARIATIONS", 0, None, true),
    ("INFO.VIEW.COLUMNS", 0, Some(0), true), ("INFO.VIEW.MEASURES", 0, Some(0), true),
    ("INFO.VIEW.RELATIONSHIPS", 0, Some(0), true), ("INFO.VIEW.TABLES", 0, Some(0), true),
    ("INT", 1, Some(1), true), ("INTERSECT", 2, Some(2), true), ("INTRATE", 4, Some(5), true),
    ("IPMT", 4, Some(6), true), ("ISAFTER", 2, None, true), ("ISATLEVEL", 1, Some(1), true),
    ("ISBLANK", 1, Some(1), true), ("ISBOOLEAN", 1, Some(1), true),
    ("ISCROSSFILTERED", 1, Some(1), true), ("ISCURRENCY", 1, Some(1), true),
    ("ISDATETIME", 1, Some(1), true), ("ISDECIMAL", 1, Some(1), true),
    ("ISDOUBLE", 1, Some(1), true), ("ISEMPTY", 1, Some(1), true), ("ISERROR", 1, Some(1), true),
    ("ISEVEN", 1, Some(1), true), ("ISFILTERED", 1, Some(1), true), ("ISINSCOPE", 1, Some(1), true),
    ("ISINT64", 1, Some(1), true), ("ISINTEGER", 1, Some(1), true), ("ISLOGICAL", 1, Some(1), true),
    ("ISNONTEXT", 1, Some(1), true), ("ISNUMBER", 1, Some(1), true),
    ("ISNUMERIC", 1, Some(1), true), ("ISO.CEILING", 1, Some(2), true), ("ISODD", 1, Some(1), true),
    ("ISONORAFTER", 2, None, true), ("ISPMT", 4, Some(4), true),
    ("ISSELECTEDMEASURE", 1, None, true), ("ISSTRING", 1, Some(1), true),
    ("ISSUBTOTAL", 1, Some(1), true), ("ISTEXT", 1, Some(1), true),
    ("KEEPFILTERS", 1, Some(1), true), ("LAST", 1, Some(5), true), ("LASTDATE", 1, Some(1), true),
    ("LASTNONBLANK", 2, Some(2), true), ("LASTNONBLANKVALUE", 2, Some(2), true),
    ("LCM", 2, Some(2), true), ("LEFT", 1, Some(2), true), ("LEN", 1, Some(1), true),
    ("LINEST", 2, None, true), ("LINESTX", 3, None, true), ("LN", 1, Some(1), true),
    ("LOG", 1, Some(2), true), ("LOG10", 1, Some(1), true), ("LOOKUP", 1, None, true),
    ("LOOKUPVALUE", 3, None, true), ("LOOKUPWITHTOTALS", 1, None, true),
    ("LOWER", 1, Some(1), true), ("MATCHBY", 0, None, true), ("MAX", 1, Some(2), true),
    ("MAXA", 1, Some(1), true), ("MAXX", 2, Some(3), true), ("MDURATION", 5, Some(6), true),
    ("MEDIAN", 1, Some(1), true), ("MEDIANX", 2, Some(2), true), ("MID", 3, Some(3), true),
    ("MIN", 1, Some(2), true), ("MINA", 1, Some(1), true), ("MINUTE", 1, Some(1), true),
    ("MINX", 2, Some(3), true), ("MOD", 2, Some(2), true), ("MONTH", 1, Some(1), true),
    ("MOVINGAVERAGE", 2, Some(7), true), ("MROUND", 2, Some(2), true), ("NAMEOF", 1, Some(3), true),
    ("NATURALINNERJOIN", 2, Some(2), true), ("NATURALJOINUSAGE", 1, Some(1), true),
    ("NATURALLEFTOUTERJOIN", 2, Some(2), true), ("NETWORKDAYS", 2, Some(4), true),
    ("NEXT", 1, Some(6), true), ("NEXTDAY", 1, Some(1), true), ("NEXTMONTH", 1, Some(1), true),
    ("NEXTQUARTER", 1, Some(1), true), ("NEXTWEEK", 1, Some(1), true),
    ("NEXTYEAR", 1, Some(2), true), ("NOMINAL", 2, Some(2), true), ("NONVISUAL", 1, Some(1), true),
    ("NORM.DIST", 4, Some(4), true), ("NORM.INV", 3, Some(3), true),
    ("NORM.S.DIST", 2, Some(2), true), ("NORM.S.INV", 1, Some(1), true), ("NOT", 0, None, true),
    ("NOW", 0, Some(0), true), ("NPER", 3, Some(5), true), ("ODD", 1, Some(1), true),
    ("ODDFPRICE", 8, Some(9), true), ("ODDFYIELD", 8, Some(9), true),
    ("ODDLPRICE", 7, Some(8), true), ("ODDLYIELD", 7, Some(8), true), ("OFFSET", 1, Some(7), true),
    ("OPENINGBALANCEMONTH", 2, Some(3), true), ("OPENINGBALANCEQUARTER", 2, Some(3), true),
    ("OPENINGBALANCEWEEK", 2, Some(3), true), ("OPENINGBALANCEYEAR", 2, Some(4), true),
    ("OR", 2, Some(2), true), ("ORDERBY", 0, None, true), ("PARALLELPERIOD", 3, Some(3), true),
    ("PARTITIONBY", 0, None, true), ("PATH", 2, Some(2), true), ("PATHCONTAINS", 2, Some(2), true),
    ("PATHITEM", 2, Some(3), true), ("PATHITEMREVERSE", 2, Some(3), true),
    ("PATHLENGTH", 1, Some(1), true), ("PDURATION", 3, Some(3), true),
    ("PERCENTILE.EXC", 2, Some(2), true), ("PERCENTILE.INC", 2, Some(2), true),
    ("PERCENTILEX.EXC", 3, Some(3), true), ("PERCENTILEX.INC", 3, Some(3), true),
    ("PERMUT", 2, Some(2), true), ("PI", 0, Some(0), true), ("PMT", 3, Some(5), true),
    ("POISSON.DIST", 3, Some(3), true), ("POWER", 2, Some(2), true), ("PPMT", 4, Some(6), true),
    ("PREVIOUS", 1, Some(6), true), ("PREVIOUSDAY", 1, Some(1), true),
    ("PREVIOUSMONTH", 1, Some(1), true), ("PREVIOUSQUARTER", 1, Some(1), true),
    ("PREVIOUSWEEK", 1, Some(1), true), ("PREVIOUSYEAR", 1, Some(2), true),
    ("PRICE", 6, Some(7), true), ("PRICEDISC", 4, Some(5), true), ("PRICEMAT", 5, Some(6), true),
    ("PRODUCT", 1, Some(1), true), ("PRODUCTX", 2, Some(2), true), ("PV", 3, Some(5), true),
    ("QUARTER", 1, Some(1), true), ("QUOTIENT", 2, Some(2), true), ("RADIANS", 1, Some(1), true),
    ("RAND", 0, Some(0), true), ("RANDBETWEEN", 2, Some(2), true), ("RANGE", 1, Some(6), true),
    ("RANK", 0, None, true), ("RANK.EQ", 2, Some(3), true), ("RANKX", 2, Some(5), true),
    ("RATE", 3, Some(6), true), ("RECEIVED", 4, Some(5), true), ("RELATED", 1, Some(1), true),
    ("RELATEDTABLE", 1, Some(1), true), ("REMOVEFILTERS", 0, None, true),
    ("REPLACE", 4, Some(4), true), ("REPT", 2, Some(2), true), ("RIGHT", 1, Some(2), true),
    ("ROLLUP", 1, None, true), ("ROLLUPADDISSUBTOTAL", 2, None, true),
    ("ROLLUPGROUP", 1, None, true), ("ROLLUPISSUBTOTAL", 2, None, true),
    ("ROUND", 2, Some(2), true), ("ROUNDDOWN", 2, Some(2), true), ("ROUNDUP", 2, Some(2), true),
    ("ROW", 2, None, true), ("ROWNUMBER", 0, Some(6), true), ("RRI", 3, Some(3), true),
    ("RUNNINGSUM", 1, Some(5), true), ("SAMEPERIODLASTYEAR", 1, Some(1), true),
    ("SAMPLE", 3, None, true), ("SAMPLEAXISWITHLOCALMINMAX", 5, None, true),
    ("SAMPLECARTESIANPOINTSBYCOVER", 4, Some(7), true), ("SEARCH", 2, Some(4), true),
    ("SECOND", 1, Some(1), true), ("SELECTCOLUMNS", 2, None, true),
    ("SELECTEDMEASURE", 0, Some(0), true), ("SELECTEDMEASUREFORMATSTRING", 0, Some(0), true),
    ("SELECTEDMEASURENAME", 0, Some(0), true), ("SELECTEDVALUE", 1, Some(2), true),
    ("SIGN", 1, Some(1), true), ("SIN", 1, Some(1), true), ("SINH", 1, Some(1), true),
    ("SLN", 3, Some(3), true), ("SQRT", 1, Some(1), true), ("SQRTPI", 1, Some(1), true),
    ("STARTOFMONTH", 1, Some(1), true), ("STARTOFQUARTER", 1, Some(1), true),
    ("STARTOFWEEK", 1, Some(1), true), ("STARTOFYEAR", 1, Some(2), true),
    ("STDEV.P", 1, Some(1), true), ("STDEV.S", 1, Some(1), true), ("STDEVX.P", 2, Some(2), true),
    ("STDEVX.S", 2, Some(2), true), ("SUBSTITUTE", 3, Some(4), true),
    ("SUBSTITUTEWITHINDEX", 4, None, true), ("SUM", 1, Some(1), true), ("SUMMARIZE", 2, None, true),
    ("SUMMARIZECOLUMNS", 1, None, true), ("SUMX", 2, Some(2), true), ("SWITCH", 3, None, true),
    ("SYD", 4, Some(4), true), ("T.DIST", 3, Some(3), true), ("T.DIST.2T", 2, Some(2), true),
    ("T.DIST.RT", 2, Some(2), true), ("T.INV", 2, Some(2), true), ("T.INV.2T", 2, Some(2), true),
    ("TABLEOF", 1, Some(1), true), ("TAN", 1, Some(1), true), ("TANH", 1, Some(1), true),
    ("TBILLEQ", 3, Some(3), true), ("TBILLPRICE", 3, Some(3), true),
    ("TBILLYIELD", 3, Some(3), true), ("TEXTCONTAINS", 2, Some(3), false),
    ("TEXTSIMILARITY", 2, Some(3), false), ("TIME", 3, Some(3), true),
    ("TIMEVALUE", 1, Some(1), true), ("TOCSV", 1, Some(4), true), ("TODAY", 0, Some(0), true),
    ("TOJSON", 1, Some(2), true), ("TOPN", 2, None, true), ("TOPNSKIP", 3, None, true),
    ("TOTALMTD", 2, Some(3), true), ("TOTALQTD", 2, Some(3), true), ("TOTALWTD", 2, Some(3), true),
    ("TOTALYTD", 2, Some(4), true), ("TREATAS", 2, None, true), ("TRIM", 1, Some(1), true),
    ("TRUE", 0, None, true), ("TRUNC", 1, Some(2), true), ("UNICHAR", 1, Some(1), true),
    ("UNICODE", 1, Some(1), true), ("UNION", 2, None, true), ("UPPER", 1, Some(1), true),
    ("USERCULTURE", 0, Some(0), true), ("USERELATIONSHIP", 2, Some(2), true),
    ("USERNAME", 0, None, true), ("USEROBJECTID", 0, Some(0), true),
    ("USERPRINCIPALNAME", 0, Some(0), true), ("UTCNOW", 0, Some(0), true),
    ("UTCTODAY", 0, Some(0), true), ("VALUE", 1, Some(1), true), ("VALUES", 1, Some(1), true),
    ("VAR.P", 1, Some(1), true), ("VAR.S", 1, Some(1), true), ("VARX.P", 2, Some(2), true),
    ("VARX.S", 2, Some(2), true), ("VDB", 5, Some(7), true), ("WEEKDAY", 1, Some(2), true),
    ("WEEKNUM", 1, Some(2), true), ("WINDOW", 2, Some(10), true), ("XIRR", 3, Some(5), true),
    ("XNPV", 4, Some(4), true), ("YEAR", 1, Some(1), true), ("YEARFRAC", 2, Some(3), true),
    ("YIELD", 6, Some(7), true), ("YIELDDISC", 4, Some(5), true), ("YIELDMAT", 5, Some(6), true),
];

/// Where the catalog deliberately differs from the raw engine probe, and why.
const DEVIATIONS: &[(&str, &str)] = &[
    // Probe artifacts: `TRUE(1, …)`/`FALSE(1, …)` are syntax errors and `NOT` is an operator, so
    // the probe saw no argument-count error; the reference arity is used.
    ("TRUE", "keyword function, 0 arguments"),
    ("FALSE", "keyword function, 0 arguments"),
    ("NOT", "operator form, 1 argument"),
    // The engine silently ignores extra arguments; the reference documents none.
    ("USERNAME", "documented with no argument"),
    // The engine resolves RANK's arguments by shape and reports unused ones instead of a count.
    ("RANK", "7 documented parameters"),
];

/// Functions whose prefix has an optional param before a required one; the optional one is
/// recognised by shape (a keyword, or a table before a column reference).
const SHAPE_MATCHED: &[&str] = &[
    "WINDOW",
    "COLLAPSEALL",
    "EXPANDALL",
    "ROLLUPADDISSUBTOTAL",
    "ROLLUPISSUBTOTAL",
];

fn sig(name: &str) -> &'static Signature {
    match signature(name) {
        Some(s) => s,
        None => panic!("{name} missing from the catalog"),
    }
}

fn names(params: &[Param]) -> Vec<&'static str> {
    params.iter().map(|p| p.name).collect()
}

fn arity_of(name: &str) -> (u8, Option<u8>) {
    let a = arity(sig(name));
    (a.min, a.max)
}

#[test]
fn catalog_has_no_duplicates() {
    let all = all();
    assert_eq!(all.len(), TABLES.iter().map(|t| t.len()).sum::<usize>());
    for w in all.windows(2) {
        assert!(w[0].name < w[1].name, "duplicate or unsorted: {}", w[1].name);
    }
}

#[test]
fn every_signature_is_well_formed() {
    for s in all() {
        let n = s.name;
        assert!(
            !n.is_empty()
                && n.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'.'),
            "{n}: names are upper-case"
        );
        assert!(!s.category.is_empty(), "{n}: category");
        let len = s.params.len();
        assert!(len <= 16, "{n}: too many params");
        for p in s.params {
            assert!(
                !p.name.is_empty()
                    && p.name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "{n}: param `{}` is snake_case",
                p.name
            );
            if let ParamKind::Keyword(set) = p.kind {
                assert!(!set.is_empty(), "{n}: empty keyword set");
                for k in set {
                    assert!(
                        k.bytes()
                            .all(|b| b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit()),
                        "{n}: keyword {k} is upper-case"
                    );
                }
            }
        }
        let mut seen: Vec<&str> = names(s.params);
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), len, "{n}: duplicate param names");

        // Layout: prefix, then a contiguous repeating unit starting at repeat_from, then a tail.
        let (prefix, unit, tail) = match s.repeat_from {
            None => {
                assert!(
                    s.params.iter().all(|p| !p.repeating),
                    "{n}: repeating without repeat_from"
                );
                (s.params, &[][..], &[][..])
            }
            Some(r) => {
                let r = usize::from(r);
                assert!(r < len, "{n}: repeat_from out of range");
                let (prefix, rest) = s.params.split_at(r);
                let unit_len = rest.iter().take_while(|p| p.repeating).count();
                assert!(unit_len > 0, "{n}: params[repeat_from] must repeat");
                let (unit, tail) = rest.split_at(unit_len);
                assert!(
                    prefix.iter().all(|p| !p.repeating),
                    "{n}: repeating param before the unit"
                );
                assert!(
                    tail.iter().all(|p| !p.repeating),
                    "{n}: repeating param after the unit"
                );
                (prefix, unit, tail)
            }
        };
        // Optional params are trailing (or inside the unit): an optional param is never followed
        // by a required one or by a mandatory unit, except for the shape-matched functions. After
        // a repeating unit, a required tail param is told apart by count or shape (DATATABLE).
        let unit_mandatory = unit.iter().any(|p| !p.optional);
        let mut seen_optional = false;
        let mut misplaced = false;
        for p in prefix {
            misplaced |= seen_optional && !p.optional;
            seen_optional |= p.optional;
        }
        if !unit.is_empty() {
            misplaced |= seen_optional && unit_mandatory;
            seen_optional |= !unit_mandatory;
        }
        for p in tail {
            misplaced |= seen_optional && !p.optional && unit.is_empty();
            seen_optional |= p.optional;
        }
        assert_eq!(
            misplaced,
            SHAPE_MATCHED.contains(&n),
            "{n}: optional param before a required one"
        );

        match s.returns {
            Returns::Fixed(_) => {}
            Returns::SameAsArg(i) => assert!(usize::from(i) < len, "{n}: SameAsArg out of range"),
            Returns::CommonOfArgs(ix) => {
                assert!(!ix.is_empty(), "{n}: empty CommonOfArgs");
                assert!(
                    ix.iter().all(|&i| usize::from(i) < len),
                    "{n}: CommonOfArgs out of range"
                );
            }
        }
    }
}

#[test]
fn covers_every_engine_function_with_the_engine_arity() {
    let mut mismatches = Vec::new();
    for &(name, min, max, _) in ENGINE {
        let Some(s) = signature(name) else {
            mismatches.push(format!("{name}: missing"));
            continue;
        };
        let a = arity(s);
        let deviates = DEVIATIONS.iter().any(|(d, _)| *d == name);
        if (a.min, a.max) != (min, max) && !deviates {
            mismatches.push(format!("{name}: catalog {a:?}, engine {min}..{max:?}"));
        }
        if (a.min, a.max) == (min, max) && deviates {
            mismatches.push(format!("{name}: listed as a deviation but matches the engine"));
        }
    }
    assert!(mismatches.is_empty(), "{mismatches:#?}");
    // Nothing in the catalog that the engine does not know.
    for s in all() {
        assert!(
            ENGINE.iter().any(|(n, ..)| *n == s.name),
            "{} is not an engine function",
            s.name
        );
    }
    assert_eq!(all().len(), ENGINE.len());
    assert_eq!(
        ENGINE.iter().filter(|e| e.3).count(),
        470,
        "MDSCHEMA_FUNCTIONS DAX rows"
    );
}

#[test]
fn deviations_keep_the_documented_arity() {
    assert_eq!(arity_of("TRUE"), (0, Some(0)));
    assert_eq!(arity_of("FALSE"), (0, Some(0)));
    assert_eq!(arity_of("NOT"), (1, Some(1)));
    assert_eq!(arity_of("USERNAME"), (0, Some(0)));
    assert_eq!(arity_of("RANK"), (0, Some(7)));
}

/// (function, [(param, engine REPEATGROUP)], engine minimum argument count)
type GroupCase = (&'static str, &'static [(&'static str, u8)], u8);

#[test]
fn multi_section_functions_follow_the_engine_repeat_groups() {
    let cases: &[GroupCase] = &[
        (
            "SUMMARIZECOLUMNS",
            &[
                ("group_by_column", 1),
                ("filter_table", 2),
                ("name", 3),
                ("expression", 3),
            ],
            1,
        ),
        (
            "SUMMARIZE",
            &[
                ("table", 0),
                ("group_by_column", 1),
                ("name", 2),
                ("expression", 2),
            ],
            2,
        ),
        (
            "GROUPBY",
            &[
                ("table", 0),
                ("group_by_column", 1),
                ("name", 2),
                ("expression", 2),
            ],
            2,
        ),
        (
            "ADDMISSINGITEMS",
            &[
                ("show_all_column", 1),
                ("table", 0),
                ("group_by_column", 2),
                ("filter_table", 3),
            ],
            1,
        ),
    ];
    for (f, groups, min) in cases {
        let s = sig(f);
        let got: Vec<(&str, u8)> = s.params.iter().map(|p| (p.name, p.group)).collect();
        assert_eq!(got, *groups, "{f}");
        assert_eq!(arity_of(f), (*min, None), "{f}");
    }
    // Every other param has no group: one repeating unit at most.
    for s in all() {
        if cases.iter().any(|(f, ..)| *f == s.name) {
            continue;
        }
        assert!(s.params.iter().all(|p| p.group == 0), "{}", s.name);
        assert_eq!(s.min_args, None, "{}", s.name);
    }
    for s in all() {
        for p in s.params {
            assert!(
                p.group == 0 || p.repeating,
                "{}: grouped param {} must repeat",
                s.name,
                p.name
            );
        }
    }
}

#[test]
fn lookup_is_case_insensitive() {
    assert_eq!(lookup("sum"), Some(Arity { min: 1, max: Some(1) }));
    assert_eq!(lookup("Percentile.Inc"), Some(Arity { min: 2, max: Some(2) }));
    assert_eq!(lookup("info.view.tables"), Some(Arity { min: 0, max: Some(0) }));
    assert!(signature("if.eager").is_some_and(|s| s.name == "IF.EAGER"));
    assert!(lookup("MYSTERY").is_none());
    assert!(lookup("").is_none());
    assert!(lookup("SUMX2").is_none());
    assert!(lookup("é").is_none());
}

#[test]
fn derived_helpers() {
    for f in [
        "ADDCOLUMNS",
        "SELECTCOLUMNS",
        "SUMMARIZE",
        "SUMMARIZECOLUMNS",
        "GROUPBY",
        "ROW",
        "DATATABLE",
        "SUBSTITUTEWITHINDEX",
    ] {
        assert!(declares_columns(f), "{f}");
    }
    assert!(declares_columns("addcolumns"));
    assert!(!declares_columns("FILTER"));
    assert!(!declares_columns("MYSTERY"));
    for k in [
        "asc",
        "DESC",
        "Dense",
        "skip",
        "YEAR",
        "WEEK",
        "BOTH",
        "ONEWAY_LEFTFILTERSRIGHT",
        "ABS",
        "REL",
        "DEFAULT",
        "FIRST",
        "LAST",
        "INTEGER",
        "CURRENCY",
        "DATETIME",
        "ROWS",
        "LOWESTPARENT",
        "PRECISE",
        "FUZZYMATCHING",
    ] {
        assert!(is_keyword_argument(k), "{k}");
    }
    assert!(!is_keyword_argument("Sales"));
    assert!(!is_keyword_argument(""));
}

#[test]
fn calculate_and_calculatetable() {
    let s = sig("CALCULATE");
    assert_eq!(s.repeat_from, Some(1));
    assert_eq!(s.params[0].kind, SCALAR);
    assert_eq!(s.params[1].kind, ParamKind::Filter);
    assert!(s.params[1].optional && s.params[1].repeating);
    assert_eq!(s.returns, Returns::SameAsArg(0));
    assert_eq!(arity_of("CALCULATE"), (1, None));
    assert_eq!(sig("CALCULATETABLE").params[0].kind, ParamKind::Table);
    assert_eq!(sig("CALCULATETABLE").returns, R_TABLE);
}

#[test]
fn switch_and_if() {
    let s = sig("SWITCH");
    assert_eq!(names(s.params), ["expression", "value", "result", "else"]);
    assert_eq!(s.repeat_from, Some(1));
    assert!(s.params[1].repeating && s.params[2].repeating);
    assert!(!s.params[3].repeating && s.params[3].optional);
    assert_eq!(s.returns, Returns::CommonOfArgs(&[2, 3]));
    assert_eq!(arity_of("SWITCH"), (3, None));
    for f in ["IF", "IF.EAGER"] {
        let s = sig(f);
        assert_eq!(s.params[0].kind, BOOL);
        assert_eq!(s.returns, Returns::CommonOfArgs(&[1, 2]));
        assert_eq!(arity_of(f), (2, Some(3)));
    }
}

#[test]
fn summarizecolumns_sections() {
    let s = sig("SUMMARIZECOLUMNS");
    assert_eq!(s.repeat_from, Some(0));
    let kinds: Vec<ParamKind> = s.params.iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        [
            ParamKind::ColumnRef,
            ParamKind::Table,
            ParamKind::NewColumnName,
            SCALAR
        ]
    );
    assert!(s.params.iter().all(|p| p.optional && p.repeating));
    assert_eq!(s.returns, R_TABLE);
}

#[test]
fn time_intelligence_keywords() {
    let s = sig("DATEADD");
    assert_eq!(
        names(s.params),
        [
            "dates",
            "number_of_intervals",
            "interval",
            "extension",
            "truncation"
        ]
    );
    let ParamKind::Keyword(intervals) = s.params[2].kind else {
        panic!("interval")
    };
    assert_eq!(intervals, ["DAY", "WEEK", "MONTH", "QUARTER", "YEAR"]);
    assert!(matches!(s.params[3].kind, ParamKind::Keyword(k) if k.contains(&"ENDALIGNED")));
    assert!(matches!(s.params[4].kind, ParamKind::Keyword(k) if k.contains(&"ANCHORED")));
    assert_eq!(arity_of("DATEADD"), (3, Some(5)));
    assert!(matches!(sig("DATEDIFF").params[2].kind, ParamKind::Keyword(k) if k.contains(&"SECOND")));
    assert_eq!(sig("TOTALYTD").params[2].kind, ParamKind::Filter);
    assert_eq!(sig("TOTALYTD").returns, Returns::SameAsArg(0));
    assert_eq!(arity_of("TOTALWTD"), (2, Some(3)));
}

#[test]
fn rankx_and_topn() {
    let s = sig("RANKX");
    assert_eq!(names(s.params), ["table", "expression", "value", "order", "ties"]);
    assert_eq!(s.params[1].kind, ParamKind::RowExpression);
    assert_eq!(s.params[3].kind, ORDER);
    assert_eq!(s.params[4].kind, TIES);
    assert_eq!(arity_of("RANKX"), (2, Some(5)));
    let s = sig("TOPN");
    assert_eq!(s.repeat_from, Some(2));
    assert_eq!(s.params[2].kind, ParamKind::RowExpression);
    assert_eq!(s.params[3].kind, ORDER);
    assert_eq!(arity_of("TOPN"), (2, None));
}

#[test]
fn window_functions() {
    let w = sig("WINDOW");
    assert_eq!(arity_of("WINDOW"), (2, Some(10)));
    assert_eq!(w.params[1].kind, BOUND_TYPE);
    assert!(!w.params[2].optional, "`to` is required");
    for (f, first) in [("INDEX", "position"), ("OFFSET", "delta")] {
        let s = sig(f);
        assert_eq!(
            names(s.params),
            [
                first,
                "relation",
                "order_by",
                "blanks",
                "partition_by",
                "match_by",
                "reset"
            ]
        );
        assert_eq!(s.returns, R_TABLE);
        assert_eq!(arity_of(f), (1, Some(7)));
    }
    assert_eq!(arity_of("ROWNUMBER"), (0, Some(6)));
    assert_eq!(sig("RANK").params[0].kind, TIES);
    for clause in ["ORDERBY", "PARTITIONBY", "MATCHBY"] {
        assert_eq!(arity_of(clause), (0, None));
        assert_eq!(sig(clause).returns, R_UNKNOWN);
    }
    assert_eq!(sig("ORDERBY").params[1].kind, ORDER);
    assert_eq!(sig("PARTITIONBY").params[0].kind, ParamKind::ColumnRef);
}

#[test]
fn lookupvalue_and_treatas() {
    let s = sig("LOOKUPVALUE");
    assert_eq!(
        names(s.params),
        [
            "result_column",
            "search_column",
            "search_value",
            "alternate_result"
        ]
    );
    assert_eq!(s.repeat_from, Some(1));
    assert_eq!(s.returns, Returns::CommonOfArgs(&[0, 3]));
    assert_eq!(arity_of("LOOKUPVALUE"), (3, None));
    let s = sig("TREATAS");
    assert_eq!(s.params[0].kind, ParamKind::Table);
    assert_eq!(s.params[1].kind, ParamKind::ColumnRef);
    assert!(s.params[1].repeating && !s.params[1].optional);
    assert_eq!(arity_of("TREATAS"), (2, None));
}

#[test]
fn iterators_take_row_expressions() {
    for f in [
        "SUMX",
        "AVERAGEX",
        "COUNTX",
        "MAXX",
        "MINX",
        "PRODUCTX",
        "CONCATENATEX",
        "FILTER",
        "MEDIANX",
    ] {
        let s = sig(f);
        assert_eq!(s.params[0].kind, ParamKind::Table, "{f}");
        assert_eq!(s.params[1].kind, ParamKind::RowExpression, "{f}");
    }
    assert_eq!(sig("SUM").params[0].kind, ParamKind::ColumnRef);
    assert_eq!(sig("RELATEDTABLE").params[0].kind, ParamKind::TableRef);
    assert_eq!(sig("ALL").params[0].kind, ParamKind::TableOrColumnRef);
}

/// Every function of the catalog has one description in English and French (the formula bar's
/// autocomplete), and every description names a function of the catalog, once.
#[test]
fn every_function_has_a_description_in_both_languages() {
    let mut seen = std::collections::HashSet::new();
    for (name, en, fr) in descriptions::DESCRIPTIONS {
        assert!(find(name).is_some(), "{name}: not a function of the catalog");
        assert!(seen.insert(*name), "{name}: described twice");
        for d in [en, fr] {
            assert!(
                (10..=200).contains(&d.chars().count()) && d.ends_with('.'),
                "{name}: {d:?}"
            );
        }
        assert_ne!(en, fr, "{name}");
    }
    for s in all() {
        assert!(description(s.name).is_some(), "{}: no description", s.name);
    }
    assert_eq!(description("divide"), description("DIVIDE"));
    assert_eq!(description("NOPE"), None);
}
