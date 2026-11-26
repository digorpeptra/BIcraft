//! Financial functions (Excel-compatible).

use super::*;

const C: &str = "financial";

const SETTLEMENT: Param = p("settlement", DATE);
const MATURITY: Param = p("maturity", DATE);
const BASIS: Param = o("basis", INT);
const FREQUENCY: Param = p("frequency", INT);
const RATE: Param = p("rate", NUM);
const NPER: Param = p("nper", NUM);
const PV: Param = p("pv", NUM);

#[rustfmt::skip]
pub(super) const FUNCTIONS: &[Signature] = &[
    f(C, "ACCRINT", &[
        p("issue", DATE), p("first_interest", DATE), SETTLEMENT, RATE, p("par", NUM), FREQUENCY,
        BASIS, o("calc_method", BOOL),
    ], R_NUM),
    f(C, "ACCRINTM", &[p("issue", DATE), MATURITY, RATE, p("par", NUM), BASIS], R_NUM),
    f(C, "AMORDEGRC", &[
        p("cost", NUM), p("date_purchased", DATE), p("first_period", DATE), p("salvage", NUM),
        p("period", NUM), RATE, BASIS,
    ], R_NUM),
    f(C, "AMORLINC", &[
        p("cost", NUM), p("date_purchased", DATE), p("first_period", DATE), p("salvage", NUM),
        p("period", NUM), RATE, BASIS,
    ], R_NUM),
    f(C, "COUPDAYBS", &[SETTLEMENT, MATURITY, FREQUENCY, BASIS], R_NUM),
    f(C, "COUPDAYS", &[SETTLEMENT, MATURITY, FREQUENCY, BASIS], R_NUM),
    f(C, "COUPDAYSNC", &[SETTLEMENT, MATURITY, FREQUENCY, BASIS], R_NUM),
    f(C, "COUPNCD", &[SETTLEMENT, MATURITY, FREQUENCY, BASIS], R_DATE),
    f(C, "COUPNUM", &[SETTLEMENT, MATURITY, FREQUENCY, BASIS], R_NUM),
    f(C, "COUPPCD", &[SETTLEMENT, MATURITY, FREQUENCY, BASIS], R_DATE),
    f(C, "CUMIPMT", &[RATE, NPER, PV, p("start_period", INT), p("end_period", INT), p("type", INT)], R_NUM),
    f(C, "CUMPRINC", &[RATE, NPER, PV, p("start_period", INT), p("end_period", INT), p("type", INT)], R_NUM),
    f(C, "DB", &[p("cost", NUM), p("salvage", NUM), p("life", NUM), p("period", NUM), o("month", INT)], R_NUM),
    f(C, "DDB", &[p("cost", NUM), p("salvage", NUM), p("life", NUM), p("period", NUM), o("factor", NUM)], R_NUM),
    f(C, "DISC", &[SETTLEMENT, MATURITY, p("pr", NUM), p("redemption", NUM), BASIS], R_NUM),
    f(C, "DOLLARDE", &[p("fractional_dollar", NUM), p("fraction", INT)], R_NUM),
    f(C, "DOLLARFR", &[p("decimal_dollar", NUM), p("fraction", INT)], R_NUM),
    f(C, "DURATION", &[SETTLEMENT, MATURITY, p("coupon", NUM), p("yld", NUM), FREQUENCY, BASIS], R_NUM),
    f(C, "EFFECT", &[p("nominal_rate", NUM), p("npery", INT)], R_NUM),
    f(C, "FV", &[RATE, NPER, p("pmt", NUM), o("pv", NUM), o("type", INT)], R_NUM),
    f(C, "INTRATE", &[SETTLEMENT, MATURITY, p("investment", NUM), p("redemption", NUM), BASIS], R_NUM),
    f(C, "IPMT", &[RATE, p("per", NUM), NPER, PV, o("fv", NUM), o("type", INT)], R_NUM),
    f(C, "ISPMT", &[RATE, p("per", NUM), NPER, PV], R_NUM),
    f(C, "MDURATION", &[SETTLEMENT, MATURITY, p("coupon", NUM), p("yld", NUM), FREQUENCY, BASIS], R_NUM),
    f(C, "NOMINAL", &[p("effect_rate", NUM), p("npery", INT)], R_NUM),
    f(C, "NPER", &[RATE, p("pmt", NUM), PV, o("fv", NUM), o("type", INT)], R_NUM),
    f(C, "ODDFPRICE", &[
        SETTLEMENT, MATURITY, p("issue", DATE), p("first_coupon", DATE), RATE, p("yld", NUM),
        p("redemption", NUM), FREQUENCY, BASIS,
    ], R_NUM),
    f(C, "ODDFYIELD", &[
        SETTLEMENT, MATURITY, p("issue", DATE), p("first_coupon", DATE), RATE, p("pr", NUM),
        p("redemption", NUM), FREQUENCY, BASIS,
    ], R_NUM),
    f(C, "ODDLPRICE", &[
        SETTLEMENT, MATURITY, p("last_interest", DATE), RATE, p("yld", NUM), p("redemption", NUM),
        FREQUENCY, BASIS,
    ], R_NUM),
    f(C, "ODDLYIELD", &[
        SETTLEMENT, MATURITY, p("last_interest", DATE), RATE, p("pr", NUM), p("redemption", NUM),
        FREQUENCY, BASIS,
    ], R_NUM),
    f(C, "PDURATION", &[RATE, PV, p("fv", NUM)], R_NUM),
    f(C, "PMT", &[RATE, NPER, PV, o("fv", NUM), o("type", INT)], R_NUM),
    f(C, "PPMT", &[RATE, p("per", NUM), NPER, PV, o("fv", NUM), o("type", INT)], R_NUM),
    f(C, "PRICE", &[SETTLEMENT, MATURITY, RATE, p("yld", NUM), p("redemption", NUM), FREQUENCY, BASIS], R_NUM),
    f(C, "PRICEDISC", &[SETTLEMENT, MATURITY, p("discount", NUM), p("redemption", NUM), BASIS], R_NUM),
    f(C, "PRICEMAT", &[SETTLEMENT, MATURITY, p("issue", DATE), RATE, p("yld", NUM), BASIS], R_NUM),
    f(C, "PV", &[RATE, NPER, p("pmt", NUM), o("fv", NUM), o("type", INT)], R_NUM),
    f(C, "RATE", &[NPER, p("pmt", NUM), PV, o("fv", NUM), o("type", INT), o("guess", NUM)], R_NUM),
    f(C, "RECEIVED", &[SETTLEMENT, MATURITY, p("investment", NUM), p("discount", NUM), BASIS], R_NUM),
    f(C, "RRI", &[NPER, PV, p("fv", NUM)], R_NUM),
    f(C, "SLN", &[p("cost", NUM), p("salvage", NUM), p("life", NUM)], R_NUM),
    f(C, "SYD", &[p("cost", NUM), p("salvage", NUM), p("life", NUM), p("per", NUM)], R_NUM),
    f(C, "TBILLEQ", &[SETTLEMENT, MATURITY, p("discount", NUM)], R_NUM),
    f(C, "TBILLPRICE", &[SETTLEMENT, MATURITY, p("discount", NUM)], R_NUM),
    f(C, "TBILLYIELD", &[SETTLEMENT, MATURITY, p("pr", NUM)], R_NUM),
    f(C, "VDB", &[
        p("cost", NUM), p("salvage", NUM), p("life", NUM), p("start_period", NUM),
        p("end_period", NUM), o("factor", NUM), o("no_switch", BOOL),
    ], R_NUM),
    f(C, "XIRR", &[
        p("table", TABLE), p("values", ROW), p("dates", ROW), o("guess", NUM),
        o("alternate_result", SCALAR),
    ], R_NUM),
    f(C, "XNPV", &[p("table", TABLE), p("values", ROW), p("dates", ROW), RATE], R_NUM),
    f(C, "YIELD", &[SETTLEMENT, MATURITY, RATE, p("pr", NUM), p("redemption", NUM), FREQUENCY, BASIS], R_NUM),
    f(C, "YIELDDISC", &[SETTLEMENT, MATURITY, p("pr", NUM), p("redemption", NUM), BASIS], R_NUM),
    f(C, "YIELDMAT", &[SETTLEMENT, MATURITY, p("issue", DATE), RATE, p("pr", NUM), BASIS], R_NUM),
];
