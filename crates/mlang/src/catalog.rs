//! BIcraft's own catalogue of the M functions its evaluator (`bicraft-data`) runs: each one's
//! parameters (required, then optional) with their types, its return type and a one-line
//! description written for BIcraft.
//!
//! The parameter names, counts and types are those Power BI Desktop's M engine reports for its
//! own library functions (`Type.FunctionParameters`, recorded as our data in
//! `tests/oracle/m_signatures.json`, M11.40), so a signature reads as the M people already know;
//! the descriptions are our own words, not the reference's text. `bicraft-data`'s tests check the
//! table against the evaluator: every function it runs has a line here, with as many required and
//! optional parameters as it takes; this crate's `oracle` test checks it against the recording.

/// One line per function: `Name(param as type, optional param as type) as type | description`.
pub(crate) const FUNCTIONS: &str = "\
#binary(value as any) as any | Makes a binary value from a list of byte numbers or from base-64 text.
#date(year as number, month as number, day as number) as date | Builds a date from its year, month and day.
#datetime(year as number, month as number, day as number, hour as number, minute as number, second as number) as datetime | Builds a date and time from its parts.
#datetimezone(year as number, month as number, day as number, hour as number, minute as number, second as number, offsetHours as number, offsetMinutes as number) as datetimezone | Builds a date and time with its offset from UTC.
#duration(days as number, hours as number, minutes as number, seconds as number) as duration | Builds a duration from days, hours, minutes and seconds.
#table(columns as any, rows as any) as any | Builds a table from its column names (or a table type) and a list of rows, each row a list of values.
#time(hour as number, minute as number, second as number) as time | Builds a time of day from its hour, minute and second.
Binary.ApproximateLength(binary as nullable binary) as nullable number | The length of a binary value in bytes, which may be an estimate.
Binary.Buffer(binary as nullable binary) as nullable binary | Reads a binary value into memory once, so later steps reuse it.
Binary.Combine(binaries as list) as binary | Joins a list of binary values end to end.
Binary.Decompress(binary as nullable binary, compressionType as number) as nullable binary | Decompresses a binary value compressed with a Compression kind (GZip, Deflate).
Binary.From(value as any, optional encoding as nullable number) as nullable binary | Converts text, a number or a binary value to a binary value.
Binary.FromList(list as list) as binary | Makes a binary value from a list of byte numbers (0 to 255).
Binary.FromText(text as nullable text, optional encoding as nullable number) as nullable binary | Decodes base-64 (the default) or hexadecimal text into a binary value.
Binary.Length(binary as nullable binary) as nullable number | The number of bytes in a binary value.
Binary.Range(binary as binary, offset as number, optional count as nullable number) as binary | The bytes from offset on: count of them, or all the rest.
Binary.Split(binary as binary, pageSize as number) as list | Cuts a binary value into a list of pieces of pageSize bytes.
Binary.ToList(binary as binary) as list | The bytes of a binary value as a list of numbers.
Binary.ToText(binary as nullable binary, optional encoding as nullable number) as nullable text | Encodes a binary value as base-64 (the default) or hexadecimal text.
Byte.From(value as any, optional culture as nullable text, optional roundingMode as nullable number) as nullable number | Converts a value to a whole number from 0 to 255.
Character.FromNumber(number as nullable number) as nullable text | The character whose Unicode code point is the number.
Character.ToNumber(character as nullable text) as nullable number | The Unicode code point of a one-character text.
Combiner.CombineTextByDelimiter(delimiter as text, optional quoteStyle as nullable number) as function | A function that joins a list of texts, the delimiter between them.
Comparer.Equals(comparer as function, x as any, y as any) as logical | Whether x and y are equal under the comparer.
Comparer.FromCulture(culture as text, optional ignoreCase as nullable logical) as function | A comparer of texts by a culture's rules, ignoring case or not.
Csv.Document(source as any, optional columns as any, optional delimiter as any, optional extraValues as nullable number, optional encoding as nullable number) as table | Reads CSV text or a CSV file's bytes as a table of text columns.
Currency.From(value as any, optional culture as nullable text, optional roundingMode as nullable number) as nullable number | Converts a value to a fixed decimal number with four decimal places.
Date.AddDays(dateTime as any, numberOfDays as number) as any | The date moved by a number of days (negative goes back).
Date.AddMonths(dateTime as any, numberOfMonths as number) as any | The date moved by a number of months; the day is kept within the new month.
Date.AddQuarters(dateTime as any, numberOfQuarters as number) as any | The date moved by a number of quarters.
Date.AddWeeks(dateTime as any, numberOfWeeks as number) as any | The date moved by a number of weeks.
Date.AddYears(dateTime as any, numberOfYears as number) as any | The date moved by a number of years.
Date.Day(dateTime as any) as nullable number | The day of the month, 1 to 31.
Date.DayOfWeek(dateTime as any, optional firstDayOfWeek as nullable number) as nullable number | The day of the week, 0 to 6, counted from firstDayOfWeek (Sunday by default).
Date.DayOfWeekName(date as any, optional culture as nullable text) as nullable text | The name of the day of the week, in a culture's language.
Date.DayOfYear(dateTime as any) as nullable number | The day of the year, 1 to 366.
Date.DaysInMonth(dateTime as any) as nullable number | How many days the date's month has.
Date.EndOfDay(dateTime as any) as any | The last moment of the value's day.
Date.EndOfMonth(dateTime as any) as any | The last moment of the value's month.
Date.EndOfQuarter(dateTime as any) as any | The last moment of the value's quarter.
Date.EndOfWeek(dateTime as any, optional firstDayOfWeek as nullable number) as any | The last moment of the value's week.
Date.EndOfYear(dateTime as any) as any | The last moment of the value's year.
Date.From(value as any, optional culture as nullable text) as nullable date | Converts text, a number or a date and time to a date.
Date.FromText(text as nullable text, optional options as any) as nullable date | Reads a date written as text, in a format or a culture's usual forms.
Date.IsInCurrentMonth(dateTime as any) as nullable logical | Whether the value falls in the current month of the system clock.
Date.IsInCurrentYear(dateTime as any) as nullable logical | Whether the value falls in the current year of the system clock.
Date.IsInPreviousNDays(dateTime as any, days as number) as nullable logical | Whether the value falls in the given number of days before today.
Date.IsLeapYear(dateTime as any) as nullable logical | Whether the value's year is a leap year.
Date.Month(dateTime as any) as nullable number | The month, 1 to 12.
Date.MonthName(date as any, optional culture as nullable text) as nullable text | The name of the month, in a culture's language.
Date.QuarterOfYear(dateTime as any) as nullable number | The quarter, 1 to 4.
Date.StartOfDay(dateTime as any) as any | The first moment of the value's day.
Date.StartOfMonth(dateTime as any) as any | The first day of the value's month.
Date.StartOfQuarter(dateTime as any) as any | The first day of the value's quarter.
Date.StartOfWeek(dateTime as any, optional firstDayOfWeek as nullable number) as any | The first day of the value's week.
Date.StartOfYear(dateTime as any) as any | The first day of the value's year.
Date.ToRecord(date as date) as record | A record with the date's Year, Month and Day.
Date.ToText(date as nullable date, optional options as any, optional culture as nullable text) as nullable text | Writes a date as text, in a format or a culture's usual form.
Date.WeekOfMonth(dateTime as any, optional firstDayOfWeek as nullable number) as nullable number | The week of the month, from 1.
Date.WeekOfYear(dateTime as any, optional firstDayOfWeek as nullable number) as nullable number | The week of the year, from 1.
Date.Year(dateTime as any) as nullable number | The year.
DateTime.AddZone(dateTime as nullable datetime, timezoneHours as number, optional timezoneMinutes as nullable number) as nullable datetimezone | Gives a date and time an offset from UTC, its clock time unchanged.
DateTime.Date(dateTime as any) as nullable date | The date part of a date and time.
DateTime.FixedLocalNow() as datetime | The local date and time, the same value throughout one evaluation.
DateTime.From(value as any, optional culture as nullable text) as nullable datetime | Converts text, a number or a date to a date and time.
DateTime.FromText(text as nullable text, optional options as any) as nullable datetime | Reads a date and time written as text.
DateTime.LocalNow() as datetime | The current local date and time.
DateTime.Time(dateTime as any) as nullable time | The time part of a date and time.
DateTime.ToRecord(dateTime as datetime) as record | A record with the value's Year, Month, Day, Hour, Minute and Second.
DateTime.ToText(dateTime as nullable datetime, optional options as any, optional culture as nullable text) as nullable text | Writes a date and time as text.
DateTimeZone.FixedLocalNow() as datetimezone | The local date and time with its offset, the same value throughout one evaluation.
DateTimeZone.FixedUtcNow() as datetimezone | The UTC date and time, the same value throughout one evaluation.
DateTimeZone.From(value as any, optional culture as nullable text) as nullable datetimezone | Converts a value to a date and time with an offset.
DateTimeZone.FromText(text as nullable text, optional options as any) as nullable datetimezone | Reads a date and time with an offset written as text.
DateTimeZone.LocalNow() as datetimezone | The current local date and time with its offset.
DateTimeZone.RemoveZone(dateTimeZone as nullable datetimezone) as nullable datetime | Drops the offset, keeping the clock time.
DateTimeZone.SwitchZone(dateTimeZone as nullable datetimezone, timezoneHours as number, optional timezoneMinutes as nullable number) as nullable datetimezone | The same instant seen from another offset.
DateTimeZone.ToLocal(dateTimeZone as nullable datetimezone) as nullable datetimezone | The same instant seen from the local offset.
DateTimeZone.ToRecord(dateTimeZone as datetimezone) as record | A record with the value's date and time parts and its offset.
DateTimeZone.ToText(dateTimeZone as nullable datetimezone, optional options as any, optional culture as nullable text) as nullable text | Writes a date and time with an offset as text.
DateTimeZone.ToUtc(dateTimeZone as nullable datetimezone) as nullable datetimezone | The same instant seen from UTC.
DateTimeZone.UtcNow() as datetimezone | The current date and time in UTC.
DateTimeZone.ZoneHours(dateTimeZone as nullable datetimezone) as nullable number | The hours of the value's offset.
DateTimeZone.ZoneMinutes(dateTimeZone as nullable datetimezone) as nullable number | The minutes of the value's offset.
Decimal.From(value as any, optional culture as nullable text) as nullable number | Converts a value to a decimal number.
Diagnostics.Trace(traceLevel as number, message as anynonnull, value as any, optional delayed as nullable logical) as any | Writes a trace message and returns the value unchanged.
Double.From(value as any, optional culture as nullable text) as nullable number | Converts a value to a double-precision number.
Duration.Days(duration as nullable duration) as nullable number | The days part of a duration.
Duration.From(value as any) as nullable duration | Converts text or a number of days to a duration.
Duration.FromText(text as nullable text) as nullable duration | Reads a duration written as text (d.hh:mm:ss).
Duration.Hours(duration as nullable duration) as nullable number | The hours part of a duration.
Duration.Minutes(duration as nullable duration) as nullable number | The minutes part of a duration.
Duration.Seconds(duration as nullable duration) as nullable number | The seconds part of a duration.
Duration.ToText(duration as nullable duration, optional format as nullable text) as nullable text | Writes a duration as text (d.hh:mm:ss).
Duration.TotalDays(duration as nullable duration) as nullable number | The whole duration counted in days.
Duration.TotalHours(duration as nullable duration) as nullable number | The whole duration counted in hours.
Duration.TotalMinutes(duration as nullable duration) as nullable number | The whole duration counted in minutes.
Duration.TotalSeconds(duration as nullable duration) as nullable number | The whole duration counted in seconds.
Error.Record(reason as text, optional message as nullable text, optional detail as any) as record | An error record to raise with error.
Excel.Workbook(workbook as binary, optional useHeaders as any, optional delayTypes as nullable logical) as table | The sheets, tables and named ranges of an Excel workbook's bytes, as a table.
Expression.Evaluate(document as text, optional environment as nullable record) as any | Evaluates M text, with the names of a record in scope.
File.Contents(path as text, optional options as nullable record) as binary | The bytes of a local file (within the folders BIcraft may read).
Folder.Contents(path as text, optional options as nullable record) as table | The files and folders directly in a folder, as a table.
Folder.Files(path as text, optional options as nullable record) as table | Every file in a folder and its subfolders, as a table with their contents.
Function.Invoke(function as function, args as list) as any | Calls a function with a list of arguments.
Int16.From(value as any, optional culture as nullable text, optional roundingMode as nullable number) as nullable number | Converts a value to a 16-bit whole number.
Int32.From(value as any, optional culture as nullable text, optional roundingMode as nullable number) as nullable number | Converts a value to a 32-bit whole number.
Int64.From(value as any, optional culture as nullable text, optional roundingMode as nullable number) as nullable number | Converts a value to a 64-bit whole number.
Int8.From(value as any, optional culture as nullable text, optional roundingMode as nullable number) as nullable number | Converts a value to an 8-bit whole number.
Json.Document(jsonText as any, optional encoding as nullable number) as any | Reads JSON text or bytes as M values (records, lists, texts, numbers).
Json.FromValue(value as any, optional encoding as nullable number) as binary | Writes a value as JSON bytes.
Lines.FromBinary(binary as binary, optional quoteStyle as any, optional includeLineSeparators as nullable logical, optional encoding as nullable number) as list | The lines of a binary value's text, as a list.
Lines.FromText(text as text, optional quoteStyle as any, optional includeLineSeparators as nullable logical) as list | The lines of a text, as a list.
Lines.ToText(lines as list, optional lineSeparator as nullable text) as text | Joins a list of lines into one text, each followed by the separator.
List.Accumulate(list as list, seed as any, accumulator as function) as any | Folds a list into one value: the accumulator gets the running value and each item in turn.
List.AllTrue(list as list) as logical | Whether every item of a list of logical values is true.
List.Alternate(list as list, count as number, optional repeatInterval as nullable number, optional offset as nullable number) as list | Skips count items, keeps repeatInterval, again and again, from offset.
List.AnyTrue(list as list) as logical | Whether at least one item of a list of logical values is true.
List.Average(list as list, optional precision as nullable number) as any | The mean of the list's numbers, dates or durations.
List.Buffer(list as list) as list | Evaluates a list once and keeps it in memory.
List.Combine(lists as list) as list | Joins a list of lists into one list.
List.Contains(list as list, value as any, optional equationCriteria as any) as logical | Whether the list holds the value.
List.ContainsAll(list as list, values as list, optional equationCriteria as any) as logical | Whether the list holds every one of the values.
List.ContainsAny(list as list, values as list, optional equationCriteria as any) as logical | Whether the list holds at least one of the values.
List.Count(list as list) as number | The number of items.
List.Covariance(numberList1 as list, numberList2 as list) as nullable number | The covariance of two lists of numbers.
List.DateTimeZones(start as datetimezone, count as number, step as duration) as list | count dates and times with offsets from start, step apart.
List.DateTimes(start as datetime, count as number, step as duration) as list | count dates and times from start, step apart.
List.Dates(start as date, count as number, step as duration) as list | count dates from start, step (a duration) apart.
List.Difference(list1 as list, list2 as list, optional equationCriteria as any) as list | The items of list1 not in list2 (one removed per occurrence).
List.Distinct(list as list, optional equationCriteria as any) as list | The list without repeated items.
List.Durations(start as duration, count as number, step as duration) as list | count durations from start, step apart.
List.First(list as list, optional defaultValue as any) as any | The first item, or the default when the list is empty.
List.FirstN(list as list, countOrCondition as any) as any | The first count items, or the leading items that meet a condition.
List.Generate(initial as function, condition as function, next as function, optional selector as nullable function) as list | Builds a list: from the initial value, while the condition holds, each value made by next.
List.InsertRange(list as list, index as number, values as list) as list | The list with values inserted at index.
List.Intersect(lists as list, optional equationCriteria as any) as list | The items found in every list.
List.IsDistinct(list as list, optional equationCriteria as any) as logical | Whether no item is repeated.
List.IsEmpty(list as list) as logical | Whether the list has no item.
List.Last(list as list, optional defaultValue as any) as any | The last item, or the default when the list is empty.
List.LastN(list as list, optional countOrCondition as any) as any | The last count items, or the trailing items that meet a condition.
List.MatchesAll(list as list, condition as function) as logical | Whether every item meets the condition.
List.MatchesAny(list as list, condition as function) as logical | Whether at least one item meets the condition.
List.Max(list as list, optional default as any, optional comparisonCriteria as any, optional includeNulls as nullable logical) as any | The largest item, or the default when the list is empty.
List.MaxN(list as list, countOrCondition as any, optional comparisonCriteria as any, optional includeNulls as nullable logical) as list | The largest items, largest first.
List.Median(list as list, optional comparisonCriteria as any) as any | The middle item once sorted.
List.Min(list as list, optional default as any, optional comparisonCriteria as any, optional includeNulls as nullable logical) as any | The smallest item, or the default when the list is empty.
List.MinN(list as list, countOrCondition as any, optional comparisonCriteria as any, optional includeNulls as nullable logical) as list | The smallest items, smallest first.
List.Mode(list as list, optional equationCriteria as any) as any | The item that appears most often.
List.Modes(list as list, optional equationCriteria as any) as list | Every item tied for appearing most often.
List.NonNullCount(list as list) as number | The number of items that are not null.
List.Numbers(start as number, count as number, optional increment as nullable number) as list | count numbers from start, increment apart (1 by default).
List.Percentile(list as list, percentiles as any, optional options as nullable record) as any | The value at a percentile (0 to 1), or a list of them for a list of percentiles.
List.PositionOf(list as list, value as any, optional occurrence as nullable number, optional equationCriteria as any) as any | Where the value is in the list (from 0), or -1.
List.PositionOfAny(list as list, values as list, optional occurrence as nullable number, optional equationCriteria as any) as any | Where any of the values is in the list, or -1.
List.Positions(list as list) as list | The positions of the items: 0, 1, 2 and so on.
List.Product(numbersList as list, optional precision as nullable number) as nullable number | The product of the numbers.
List.Range(list as list, offset as number, optional count as nullable number) as list | The items from offset on: count of them, or all the rest.
List.RemoveFirstN(list as list, optional countOrCondition as any) as list | The list without its first items (one by default).
List.RemoveItems(list1 as list, list2 as list) as list | list1 without any of the items of list2.
List.RemoveLastN(list as list, optional countOrCondition as any) as list | The list without its last items (one by default).
List.RemoveMatchingItems(list1 as list, list2 as list, optional equationCriteria as any) as list | list1 without every occurrence of the items of list2.
List.RemoveNulls(list as list) as list | The list without its null items.
List.RemoveRange(list as list, index as number, optional count as nullable number) as list | The list without count items from index (one by default).
List.Repeat(list as list, count as number) as list | The list repeated count times.
List.ReplaceMatchingItems(list as list, replacements as list, optional equationCriteria as any) as list | The list with each {old, new} pair of replacements applied.
List.ReplaceRange(list as list, index as number, count as number, replaceWith as list) as list | The list with count items from index replaced by a list.
List.ReplaceValue(list as list, oldValue as any, newValue as any, replacer as function) as list | The list with a replacer function applied to every item.
List.Reverse(list as list) as list | The items in reverse order.
List.Select(list as list, selection as function) as list | The items for which the selection function returns true.
List.Single(list as list) as any | The only item; an error when there are none or several.
List.SingleOrDefault(list as list, optional default as any) as any | The only item, or the default when there is none.
List.Skip(list as list, optional countOrCondition as any) as list | The list without its first items (one by default).
List.Sort(list as list, optional comparisonCriteria as any) as list | The items in order (ascending by default).
List.Split(list as list, pageSize as number) as list | Cuts a list into a list of lists of pageSize items.
List.StandardDeviation(numbersList as list) as nullable number | The sample standard deviation of the numbers.
List.Sum(list as list, optional precision as nullable number) as any | The sum of the numbers or durations.
List.Times(start as time, count as number, step as duration) as list | count times of day from start, step apart.
List.Transform(list as list, transform as function) as list | Each item passed through the transform function.
List.TransformMany(list as list, collectionTransform as function, resultTransform as function) as list | For each item, a list of values, each combined with the item; all of them in one list.
List.Union(lists as list, optional equationCriteria as any) as list | The items of every list, a repeated item kept as often as in the list that has it most.
List.Zip(lists as list) as list | Pairs the items at the same position: a list of lists.
Logical.From(value as any) as nullable logical | Converts a number or text to true or false.
Logical.FromText(text as nullable text) as nullable logical | Reads \"true\" or \"false\".
Logical.ToText(logicalValue as nullable logical) as nullable text | Writes true or false as text.
Number.Abs(number as nullable number) as nullable number | The absolute value.
Number.Acos(number as nullable number) as nullable number | The arccosine, in radians.
Number.Asin(number as nullable number) as nullable number | The arcsine, in radians.
Number.Atan(number as nullable number) as nullable number | The arctangent, in radians.
Number.BitwiseAnd(number1 as nullable number, number2 as nullable number) as nullable number | The bits set in both numbers.
Number.BitwiseNot(number as any) as any | Every bit of the number flipped.
Number.BitwiseOr(number1 as nullable number, number2 as nullable number) as nullable number | The bits set in either number.
Number.BitwiseShiftLeft(number1 as nullable number, number2 as nullable number) as nullable number | number1's bits moved left by number2 places.
Number.BitwiseShiftRight(number1 as nullable number, number2 as nullable number) as nullable number | number1's bits moved right by number2 places.
Number.BitwiseXor(number1 as nullable number, number2 as nullable number) as nullable number | The bits set in exactly one of the numbers.
Number.Cos(number as nullable number) as nullable number | The cosine of an angle in radians.
Number.Exp(number as nullable number) as nullable number | e raised to the number.
Number.Factorial(number as nullable number) as nullable number | The product of the whole numbers from 1 to the number (1 for 0).
Number.From(value as any, optional culture as nullable text) as nullable number | Converts text, a logical, a date or a duration to a number.
Number.FromText(text as nullable text, optional culture as nullable text) as nullable number | Reads a number written as text.
Number.IntegerDivide(number1 as nullable number, number2 as nullable number, optional precision as nullable number) as nullable number | The whole part of number1 divided by number2.
Number.IsEven(number as number) as logical | Whether the number is even.
Number.IsNaN(number as number) as logical | Whether the value is NaN (not a number).
Number.IsOdd(number as number) as logical | Whether the number is odd.
Number.Ln(number as nullable number) as nullable number | The natural logarithm.
Number.Log(number as nullable number, optional base as nullable number) as nullable number | The logarithm in a base (e by default).
Number.Log10(number as nullable number) as nullable number | The base-10 logarithm.
Number.Mod(number as nullable number, divisor as nullable number, optional precision as nullable number) as nullable number | The remainder of the whole division of the number by the divisor.
Number.Power(number as nullable number, power as nullable number) as nullable number | The number raised to a power.
Number.Random() as number | A random number from 0 up to 1.
Number.RandomBetween(bottom as number, top as number) as number | A random number between bottom and top.
Number.Round(number as nullable number, optional digits as nullable number, optional roundingMode as nullable number) as nullable number | Rounds to a number of decimal places (0 by default), halves to even unless a RoundingMode says otherwise.
Number.RoundAwayFromZero(number as nullable number, optional digits as nullable number) as nullable number | Rounds halves away from zero.
Number.RoundDown(number as nullable number, optional digits as nullable number) as nullable number | Rounds down to a number of decimal places.
Number.RoundTowardZero(number as nullable number, optional digits as nullable number) as nullable number | Rounds toward zero.
Number.RoundUp(number as nullable number, optional digits as nullable number) as nullable number | Rounds up to a number of decimal places.
Number.Sign(number as nullable number) as nullable number | 1 for a positive number, -1 for a negative one, 0 for zero.
Number.Sin(number as nullable number) as nullable number | The sine of an angle in radians.
Number.Sqrt(number as nullable number) as nullable number | The square root.
Number.Tan(number as nullable number) as nullable number | The tangent of an angle in radians.
Number.ToText(number as nullable number, optional format as nullable text, optional culture as nullable text) as nullable text | Writes a number as text, in a format and culture.
Percentage.From(value as any, optional culture as nullable text) as nullable number | Converts a value to a number; text such as \"15%\" reads as 0.15.
Record.AddField(record as record, fieldName as text, value as any, optional delayed as nullable logical) as record | The record with one more field.
Record.Combine(records as list) as record | Merges a list of records; a later field wins over an earlier one of the same name.
Record.Field(record as record, field as text) as any | The value of a field, by name.
Record.FieldCount(record as record) as number | The number of fields.
Record.FieldNames(record as record) as list | The names of the fields, in order.
Record.FieldOrDefault(record as nullable record, field as text, optional defaultValue as any) as any | The value of a field, or the default when there is no such field.
Record.FieldValues(record as record) as list | The values of the fields, in order.
Record.FromList(list as list, fields as any) as record | A record from a list of values and their field names.
Record.FromTable(table as table) as record | A record from a table of Name and Value columns.
Record.HasFields(record as record, fields as any) as logical | Whether the record has the field or every field of a list.
Record.RemoveFields(record as record, fields as any, optional missingField as nullable number) as record | The record without some fields.
Record.RenameFields(record as record, renames as list, optional missingField as nullable number) as record | The record with fields renamed by {old, new} pairs.
Record.ReorderFields(record as record, fieldOrder as list, optional missingField as nullable number) as record | The record with some fields moved into the order given.
Record.SelectFields(record as record, fields as any, optional missingField as nullable number) as record | The record with only some fields.
Record.ToList(record as record) as list | The field values as a list.
Record.ToTable(record as record) as table | A table of Name and Value columns, one row per field.
Record.TransformFields(record as record, transformOperations as list, optional missingField as nullable number) as record | The record with {field, function} transforms applied.
Replacer.ReplaceText(text as nullable text, old as text, new as text) as nullable text | Replaces every occurrence of old in a text; for the replacer of a replace step.
Replacer.ReplaceValue(value as any, old as any, new as any) as any | Replaces a value equal to old; for the replacer of a replace step.
Single.From(value as any, optional culture as nullable text) as nullable number | Converts a value to a single-precision number.
Splitter.SplitByNothing() as function | A splitter that keeps the text whole, as a one-item list.
Splitter.SplitTextByAnyDelimiter(delimiters as list, optional quoteStyle as nullable number, optional startAtEnd as nullable logical) as function | A splitter that cuts text at any of the delimiters.
Splitter.SplitTextByCharacterTransition(before as anynonnull, after as anynonnull) as function | A splitter that cuts text between a character of one kind and one of another (lower to upper case, letter to digit).
Splitter.SplitTextByDelimiter(delimiter as text, optional quoteStyle as nullable number) as function | A splitter that cuts text at each occurrence of the delimiter.
Splitter.SplitTextByEachDelimiter(delimiters as list, optional quoteStyle as nullable number, optional startAtEnd as nullable logical) as function | A splitter that cuts text once at each delimiter in turn.
Splitter.SplitTextByLengths(lengths as list, optional startAtEnd as nullable logical) as function | A splitter that cuts text into pieces of the lengths given.
Splitter.SplitTextByPositions(positions as list, optional startAtEnd as nullable logical) as function | A splitter that cuts text at the positions given.
Splitter.SplitTextByRanges(ranges as list, optional startAtEnd as nullable logical) as function | A splitter that takes {offset, length} ranges of the text.
Splitter.SplitTextByRepeatedLengths(length as number, optional startAtEnd as nullable logical) as function | A splitter that cuts text into pieces of one length.
Splitter.SplitTextByWhitespace(optional quoteStyle as nullable number) as function | A splitter that cuts text at runs of white space.
Table.AddColumn(table as table, newColumnName as text, columnGenerator as function, optional columnType as nullable type) as table | Adds a column whose value the generator works out from each row.
Table.AddIndexColumn(table as table, newColumnName as text, optional initialValue as nullable number, optional increment as nullable number, optional columnType as nullable type) as table | Adds a column that numbers the rows (from 0 by default).
Table.AddJoinColumn(table1 as table, key1 as any, table2 as any, key2 as any, newColumnName as text) as table | Adds a column holding, for each row, the rows of table2 whose key matches.
Table.AddKey(table as table, columns as list, isPrimary as logical) as table | Declares a key over some columns.
Table.AddRankColumn(table as table, newColumnName as text, comparisonCriteria as any, optional options as nullable record) as table | Adds a column with each row's rank by the criteria.
Table.AggregateTableColumn(table as table, column as text, aggregations as list) as table | Replaces a column of nested tables with aggregates of their columns.
Table.AlternateRows(table as table, offset as number, skip as number, take as number) as table | From offset, keeps take rows then skips skip rows, again and again.
Table.ApproximateRowCount(table as table) as number | The number of rows, which may be an estimate.
Table.Buffer(table as table, optional options as nullable record) as table | Evaluates a table once and keeps it in memory.
Table.Column(table as table, column as text) as list | The values of one column, as a list.
Table.ColumnCount(table as table) as number | The number of columns.
Table.ColumnNames(table as table) as list | The column names, in order.
Table.ColumnsOfType(table as table, listOfTypes as list) as list | The names of the columns of any of the types given.
Table.Combine(tables as list, optional columns as any) as table | Appends tables; a column missing from one gets nulls.
Table.CombineColumns(table as table, sourceColumns as list, combiner as function, column as text) as table | Merges some columns into one through a combiner.
Table.CombineColumnsToRecord(table as table, newColumnName as text, sourceColumns as list, optional options as nullable record) as table | Merges some columns into one column of records.
Table.Contains(table as table, row as record, optional equationCriteria as any) as logical | Whether a row matches the record given.
Table.ContainsAll(table as table, rows as list, optional equationCriteria as any) as logical | Whether every record of a list matches a row.
Table.ContainsAny(table as table, rows as list, optional equationCriteria as any) as logical | Whether any record of a list matches a row.
Table.DemoteHeaders(table as table) as table | Moves the column names into a first row; the columns become Column1, Column2 and so on.
Table.Distinct(table as table, optional equationCriteria as any) as table | The table without repeated rows (or rows repeated in some columns).
Table.DuplicateColumn(table as table, columnName as text, newColumnName as text, optional columnType as nullable type) as table | Adds a copy of a column.
Table.ExpandListColumn(table as table, column as text) as table | One row per item of a column of lists.
Table.ExpandRecordColumn(table as table, column as text, fieldNames as list, optional newColumnNames as nullable list) as table | Turns fields of a column of records into columns.
Table.ExpandTableColumn(table as table, column as text, columnNames as list, optional newColumnNames as nullable list) as table | Turns columns of a column of nested tables into columns, one row per nested row.
Table.FillDown(table as table, columns as list) as table | Fills nulls in some columns with the value above.
Table.FillUp(table as table, columns as list) as table | Fills nulls in some columns with the value below.
Table.FindText(table as table, text as text) as table | The rows where any value contains the text.
Table.First(table as table, optional default as any) as any | The first row as a record, or the default when the table is empty.
Table.FirstN(table as table, countOrCondition as any) as table | The first count rows, or the leading rows that meet a condition.
Table.FirstValue(table as table, optional default as any) as any | The first column's value in the first row.
Table.FromColumns(lists as list, optional columns as any) as table | A table from a list of columns, each a list of values.
Table.FromList(list as list, optional splitter as nullable function, optional columns as any, optional default as any, optional extraValues as nullable number) as table | A table from a list, each item split into a row.
Table.FromRecords(records as list, optional columns as any, optional missingField as nullable number) as table | A table from a list of records, one row each.
Table.FromRows(rows as list, optional columns as any) as table | A table from a list of rows, each a list of values.
Table.FromValue(value as any, optional options as nullable record) as table | A table from a record, a list or a single value.
Table.Group(table as table, key as any, aggregatedColumns as list, optional groupKind as nullable number, optional comparer as nullable function) as table | One row per key value, with {name, function, type} aggregates of each group.
Table.HasColumns(table as table, columns as any) as logical | Whether the table has the column or every column of a list.
Table.InsertRows(table as table, offset as number, rows as list) as table | The table with a list of records inserted at offset.
Table.IsDistinct(table as table, optional comparisonCriteria as any) as logical | Whether no row is repeated.
Table.IsEmpty(table as table) as logical | Whether the table has no row.
Table.Join(table1 as table, key1 as any, table2 as table, key2 as any, optional joinKind as nullable number, optional joinAlgorithm as nullable number, optional keyEqualityComparers as nullable list) as table | Joins two tables on their keys, the columns of both side by side.
Table.Last(table as table, optional default as any) as any | The last row as a record, or the default when the table is empty.
Table.LastN(table as table, countOrCondition as any) as table | The last count rows, or the trailing rows that meet a condition.
Table.MatchesAllRows(table as table, condition as function) as logical | Whether every row meets the condition.
Table.MatchesAnyRows(table as table, condition as function) as logical | Whether at least one row meets the condition.
Table.Max(table as table, comparisonCriteria as any, optional default as any) as any | The largest row by the criteria, as a record.
Table.MaxN(table as table, comparisonCriteria as any, countOrCondition as any) as table | The largest rows by the criteria.
Table.Min(table as table, comparisonCriteria as any, optional default as any) as any | The smallest row by the criteria, as a record.
Table.MinN(table as table, comparisonCriteria as any, countOrCondition as any) as table | The smallest rows by the criteria.
Table.NestedJoin(table1 as table, key1 as any, table2 as any, key2 as any, newColumnName as text, optional joinKind as nullable number, optional keyEqualityComparers as nullable list) as table | Joins two tables on their keys, the matching rows of table2 in a new column of nested tables.
Table.Partition(table as table, column as text, groups as number, hash as function) as list | Splits a table into groups tables by a hash of a column.
Table.Pivot(table as table, pivotValues as list, attributeColumn as text, valueColumn as text, optional aggregationFunction as nullable function) as table | Turns the values of an attribute column into columns.
Table.PositionOf(table as table, row as record, optional occurrence as any, optional equationCriteria as any) as any | Where a row matching the record is (from 0), or -1.
Table.PositionOfAny(table as table, rows as list, optional occurrence as nullable number, optional equationCriteria as any) as any | Where a row matching any record of a list is, or -1.
Table.PrefixColumns(table as table, prefix as text) as table | Renames every column to prefix.name.
Table.PromoteHeaders(table as table, optional options as nullable record) as table | Uses the first row as the column names.
Table.Range(table as table, offset as number, optional count as nullable number) as table | The rows from offset on: count of them, or all the rest.
Table.RemoveColumns(table as table, columns as any, optional missingField as nullable number) as table | The table without some columns.
Table.RemoveFirstN(table as table, optional countOrCondition as any) as table | The table without its first rows (one by default).
Table.RemoveLastN(table as table, optional countOrCondition as any) as table | The table without its last rows (one by default).
Table.RemoveMatchingRows(table as table, rows as list, optional equationCriteria as any) as table | The table without the rows matching any record of a list.
Table.RemoveRows(table as table, offset as number, optional count as nullable number) as table | The table without count rows from offset (one by default).
Table.RemoveRowsWithErrors(table as table, optional columns as nullable list) as table | The table without the rows that hold an error (in some columns).
Table.RenameColumns(table as table, renames as list, optional missingField as nullable number) as table | Renames columns by {old, new} pairs.
Table.ReorderColumns(table as table, columnOrder as list, optional missingField as nullable number) as table | Moves some columns into the order given.
Table.Repeat(table as table, count as number) as table | The rows repeated count times.
Table.ReplaceErrorValues(table as table, errorReplacement as list) as table | Replaces errors in columns by {column, value} pairs.
Table.ReplaceKeys(table as table, keys as list) as table | Replaces the table's declared keys.
Table.ReplaceMatchingRows(table as table, replacements as list, optional equationCriteria as any) as table | Replaces rows by {old, new} record pairs.
Table.ReplaceRows(table as table, offset as number, count as number, rows as list) as table | Replaces count rows from offset with a list of records.
Table.ReplaceValue(table as table, oldValue as any, newValue as any, replacer as function, columnsToSearch as list) as table | Replaces values in some columns through a replacer (Replacer.ReplaceValue or ReplaceText).
Table.ReverseRows(table as table) as table | The rows in reverse order.
Table.RowCount(table as table) as number | The number of rows.
Table.Schema(table as table) as table | A table describing each column: name, position, type and facets.
Table.SelectColumns(table as table, columns as any, optional missingField as nullable number) as table | Keeps only some columns, in the order given.
Table.SelectRows(table as table, condition as function) as table | Keeps the rows for which the condition returns true.
Table.SelectRowsWithErrors(table as table, optional columns as nullable list) as table | Keeps the rows that hold an error (in some columns).
Table.SingleRow(table as table) as record | The only row as a record; an error when there are none or several.
Table.Skip(table as table, optional countOrCondition as any) as table | The table without its first rows (one by default).
Table.Sort(table as table, comparisonCriteria as any) as table | Sorts the rows by columns, each ascending or descending ({column, Order.Descending}).
Table.Split(table as table, pageSize as number) as list | Cuts a table into a list of tables of pageSize rows.
Table.SplitAt(table as table, count as number) as list | A list of two tables: the first count rows and the rest.
Table.SplitColumn(table as table, sourceColumn as text, splitter as function, optional columnNamesOrNumber as any, optional default as any, optional extraColumns as any) as table | Splits a column into several through a splitter.
Table.StopFolding(table as table) as table | Keeps later steps from being sent to the source.
Table.ToColumns(table as table) as list | The columns as a list of lists.
Table.ToList(table as table, optional combiner as nullable function) as list | Each row combined into one value (its texts joined with commas by default).
Table.ToRecords(table as table) as list | The rows as a list of records.
Table.ToRows(table as table) as list | The rows as a list of lists.
Table.TransformColumnNames(table as table, nameGenerator as function, optional options as nullable record) as table | Renames every column through a function of its name.
Table.TransformColumnTypes(table as table, typeTransformations as list, optional culture as any) as table | Changes column types by {column, type} pairs, converting the values.
Table.TransformColumns(table as table, transformOperations as list, optional defaultTransformation as nullable function, optional missingField as nullable number) as table | Applies {column, function} transforms to columns' values.
Table.TransformRows(table as table, transform as function) as list | Each row, as a record, passed through a function: a list of the results.
Table.Transpose(table as table, optional columns as any) as table | Turns rows into columns and columns into rows.
Table.Unpivot(table as table, pivotColumns as list, attributeColumn as text, valueColumn as text) as table | Turns some columns into attribute and value rows.
Table.UnpivotOtherColumns(table as table, pivotColumns as list, attributeColumn as text, valueColumn as text) as table | Turns every column but some into attribute and value rows.
Text.AfterDelimiter(text as nullable text, delimiter as text, optional index as any) as any | The part of the text after a delimiter (the first one by default).
Text.At(text as nullable text, index as number) as nullable text | The character at a position (from 0).
Text.BeforeDelimiter(text as nullable text, delimiter as text, optional index as any) as any | The part of the text before a delimiter (the first one by default).
Text.BetweenDelimiters(text as nullable text, startDelimiter as text, endDelimiter as text, optional startIndex as any, optional endIndex as any) as any | The part of the text between two delimiters.
Text.Clean(text as nullable text) as nullable text | The text without its control characters.
Text.Combine(texts as list, optional separator as nullable text) as text | Joins a list of texts, the separator between them; nulls are skipped.
Text.Contains(text as nullable text, substring as text, optional comparer as nullable function) as nullable logical | Whether the text holds the substring.
Text.End(text as nullable text, count as number) as nullable text | The last count characters.
Text.EndsWith(text as nullable text, substring as text, optional comparer as nullable function) as nullable logical | Whether the text ends with the substring.
Text.Format(formatString as text, arguments as any, optional culture as nullable text) as text | Fills #{0}, #[name] placeholders of a format with a list's or a record's values.
Text.From(value as any, optional culture as nullable text) as nullable text | Writes a number, date, logical or other value as text.
Text.FromBinary(binary as nullable binary, optional encoding as nullable number) as nullable text | Decodes bytes into text (UTF-8 by default).
Text.Insert(text as nullable text, offset as number, newText as text) as nullable text | The text with newText inserted at offset.
Text.Length(text as nullable text) as nullable number | The number of characters.
Text.Lower(text as nullable text, optional culture as nullable text) as nullable text | The text in lower case.
Text.Middle(text as nullable text, start as number, optional count as nullable number) as nullable text | count characters from start (all the rest by default).
Text.NewGuid() as text | A new random GUID, as text.
Text.PadEnd(text as nullable text, count as number, optional character as nullable text) as nullable text | Pads the end of the text to count characters (with spaces by default).
Text.PadStart(text as nullable text, count as number, optional character as nullable text) as nullable text | Pads the start of the text to count characters (with spaces by default).
Text.PositionOf(text as text, substring as text, optional occurrence as nullable number, optional comparer as nullable function) as any | Where the substring is in the text (from 0), or -1.
Text.PositionOfAny(text as text, characters as list, optional occurrence as nullable number) as any | Where any of the characters is in the text, or -1.
Text.Proper(text as nullable text, optional culture as nullable text) as nullable text | The text with each word capitalized.
Text.Range(text as nullable text, offset as number, optional count as nullable number) as nullable text | count characters from offset (all the rest by default); an error past the end.
Text.Remove(text as nullable text, removeChars as any) as nullable text | The text without the characters given.
Text.RemoveRange(text as nullable text, offset as number, optional count as nullable number) as nullable text | The text without count characters from offset (one by default).
Text.Repeat(text as nullable text, count as number) as nullable text | The text repeated count times.
Text.Replace(text as nullable text, old as text, new as text) as nullable text | The text with every occurrence of old replaced by new.
Text.ReplaceRange(text as nullable text, offset as number, count as number, newText as text) as nullable text | The text with count characters from offset replaced by newText.
Text.Reverse(text as nullable text) as nullable text | The characters in reverse order.
Text.Select(text as nullable text, selectChars as any) as nullable text | Only the characters given, kept from the text.
Text.Split(text as text, separator as text) as list | Cuts the text at each occurrence of the separator.
Text.SplitAny(text as text, separators as text) as list | Cuts the text at any of the separator characters.
Text.Start(text as nullable text, count as number) as nullable text | The first count characters.
Text.StartsWith(text as nullable text, substring as text, optional comparer as nullable function) as nullable logical | Whether the text starts with the substring.
Text.ToBinary(text as nullable text, optional encoding as nullable number, optional includeByteOrderMark as nullable logical) as nullable binary | Encodes text into bytes (UTF-8 by default).
Text.ToList(text as text) as list | The characters as a list.
Text.Trim(text as nullable text, optional trim as any) as nullable text | The text without leading and trailing white space (or the characters given).
Text.TrimEnd(text as nullable text, optional trim as any) as nullable text | The text without trailing white space (or the characters given).
Text.TrimStart(text as nullable text, optional trim as any) as nullable text | The text without leading white space (or the characters given).
Text.Upper(text as nullable text, optional culture as nullable text) as nullable text | The text in upper case.
Time.EndOfHour(dateTime as any) as any | The last moment of the value's hour.
Time.From(value as any, optional culture as nullable text) as nullable time | Converts text, a number or a date and time to a time of day.
Time.FromText(text as nullable text, optional options as any) as nullable time | Reads a time written as text.
Time.Hour(dateTime as any) as nullable number | The hour, 0 to 23.
Time.Minute(dateTime as any) as nullable number | The minute, 0 to 59.
Time.Second(dateTime as any) as nullable number | The second, with its fraction.
Time.StartOfHour(dateTime as any) as any | The first moment of the value's hour.
Time.ToRecord(time as time) as record | A record with the time's Hour, Minute and Second.
Time.ToText(time as nullable time, optional options as any, optional culture as nullable text) as nullable text | Writes a time as text.
Type.Is(type1 as type, type2 as type) as logical | Whether values of type1 are always values of type2.
Type.IsNullable(type as type) as logical | Whether the type allows null.
Type.NonNullable(type as type) as type | The type without null.
Type.TableColumn(tableType as type, column as text) as type | The type of a column of a table type.
Value.Add(value1 as any, value2 as any, optional precision as nullable number) as any | value1 + value2.
Value.As(value as any, type as type) as any | The value when it is of the type, else an error.
Value.Compare(value1 as any, value2 as any, optional precision as nullable number) as number | -1, 0 or 1 as value1 is less than, equal to or greater than value2.
Value.Divide(value1 as any, value2 as any, optional precision as nullable number) as any | value1 / value2.
Value.Equals(value1 as any, value2 as any, optional precision as nullable number) as logical | Whether the values are equal.
Value.FromText(text as any, optional culture as nullable text) as any | Reads a number, date, time, logical or other value from text, as Power Query guesses it.
Value.Is(value as any, type as type) as logical | Whether the value is of the type.
Value.Metadata(value as any) as any | The value's metadata record.
Value.Multiply(value1 as any, value2 as any, optional precision as nullable number) as any | value1 * value2.
Value.NullableEquals(value1 as any, value2 as any, optional precision as nullable number) as nullable logical | Whether the values are equal; null when either is null.
Value.RemoveMetadata(value as any, optional metaValue as any) as any | The value without its metadata (or some fields of it).
Value.ReplaceMetadata(value as any, metaValue as any) as any | The value with a new metadata record.
Value.ReplaceType(value as any, type as type) as any | The value with another type of the same kind (column types of a table, say).
Value.Subtract(value1 as any, value2 as any, optional precision as nullable number) as any | value1 - value2.
Value.Type(value as any) as type | The value's type.
";

/// Library constants and type names the evaluator knows, one group per line:
/// `description | Name Name …`.
pub(crate) const CONSTANTS: &str = "\
How a sort orders values | Order.Ascending Order.Descending
How two tables join | JoinKind.Inner JoinKind.LeftOuter JoinKind.RightOuter JoinKind.FullOuter JoinKind.LeftAnti JoinKind.RightAnti
How a join is worked out | JoinAlgorithm.Dynamic
What a step does with a missing field or column | MissingField.Error MissingField.Ignore MissingField.UseNull
How Table.Group groups rows | GroupKind.Local GroupKind.Global
How quotes are read in delimited text | QuoteStyle.None QuoteStyle.Csv
How CSV values are quoted | CsvStyle.QuoteAfterDelimiter CsvStyle.QuoteAlways
A compression format | Compression.GZip Compression.Deflate Compression.Snappy Compression.Brotli Compression.LZ4 Compression.Zstandard
How bytes are written as text | BinaryEncoding.Base64 BinaryEncoding.Hex
Which occurrences a search returns | Occurrence.First Occurrence.Last Occurrence.All
How a number is rounded | RoundingMode.Up RoundingMode.Down RoundingMode.AwayFromZero RoundingMode.TowardZero RoundingMode.ToEven
The precision of arithmetic | Precision.Double Precision.Decimal
A day of the week | Day.Sunday Day.Monday Day.Tuesday Day.Wednesday Day.Thursday Day.Friday Day.Saturday
What happens to values past the columns | ExtraValues.List ExtraValues.Error ExtraValues.Ignore
How ties are ranked | RankKind.Competition RankKind.Dense RankKind.Ordinal
How a percentile is interpolated | PercentileMode.ExcelInc PercentileMode.ExcelExc PercentileMode.SqlDisc PercentileMode.SqlCont
A number constant | Number.PI Number.E Number.Epsilon Number.PositiveInfinity Number.NegativeInfinity Number.NaN
A text encoding (code page) | TextEncoding.Utf8 TextEncoding.Utf16 TextEncoding.Unicode TextEncoding.BigEndianUnicode TextEncoding.Windows TextEncoding.Ascii
The culture of the evaluation | Culture.Current
A comparer of texts | Comparer.Ordinal Comparer.OrdinalIgnoreCase
A type | Any.Type None.Type Null.Type Logical.Type Number.Type Text.Type Date.Type DateTime.Type DateTimeZone.Type Time.Type Duration.Type Binary.Type List.Type Record.Type Table.Type Function.Type Type.Type Int64.Type Int32.Type Int16.Type Int8.Type Byte.Type Double.Type Single.Type Decimal.Type Currency.Type Percentage.Type RankKind.Type PercentileMode.Type
";

/// The M keywords, with what each one does.
pub(crate) const KEYWORDS: &[(&str, &str)] = &[
    ("and", "Logical and: true when both sides are true."),
    (
        "as",
        "Checks that a value is of a type (also a parameter's or a function's type).",
    ),
    ("catch", "After try: a function that handles the error."),
    (
        "each",
        "Starts a one-parameter function; its parameter is _ and [Column] reads _'s field.",
    ),
    ("else", "The branch of if taken when the condition is false."),
    ("error", "Raises an error (a text or a record from Error.Record)."),
    ("false", "The logical value false."),
    ("if", "Chooses between two values: if condition then a else b."),
    (
        "in",
        "Ends the steps of let: the expression after it is the result.",
    ),
    ("is", "Whether a value is of a type."),
    (
        "let",
        "Starts named steps, Name = expression, separated by commas, then in and the result.",
    ),
    ("meta", "Attaches a metadata record to a value."),
    ("not", "Logical not."),
    ("null", "The missing value."),
    ("or", "Logical or: true when either side is true."),
    (
        "otherwise",
        "After try: the value used when the expression fails.",
    ),
    ("section", "Starts a section document of shared members."),
    ("shared", "Marks a section member visible to other sections."),
    ("then", "The branch of if taken when the condition is true."),
    ("true", "The logical value true."),
    (
        "try",
        "Evaluates an expression and catches its error (with otherwise or catch).",
    ),
    (
        "type",
        "Starts a type: type table [A = text], type nullable number…",
    ),
];
