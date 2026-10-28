//! Deterministic English spoken-number normalization for live dictation.
//!
//! The parser is deliberately bounded and accepts only well-formed cardinal
//! groups. Invalid or out-of-order scale phrases are normalized only through
//! the last unambiguous group instead of being assigned a guessed value.
//!
//! Rendering follows a content-free [`NumberPolicy`] resolved once from the
//! frozen recording context. The policy is never inferred from transcript text.

const MAX_NUMBER_WORDS: usize = 64;
const MAX_FRACTION_DIGITS: usize = 32;
const MAX_DOTTED_SEGMENTS: usize = 8;
const MAX_SPOKEN_NUMBER_INPUT_BYTES: usize = 16 * 1024;

/// How eagerly one recording renders number words as digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumberPolicy {
    /// Everyday, Messages, Email, and Notes. Small standalone counts (zero
    /// through nine) attached to ordinary language stay words.
    Prose,
    /// Technical, Terminal, and local project context. Every cardinal becomes
    /// digits, and spaced digit runs and dotted versions are joined.
    Technical,
}

/// Words that conventionally label the following value with digits.
const LABEL_WORDS: &[&str] = &[
    "number",
    "step",
    "option",
    "version",
    "chapter",
    "page",
    "line",
    "issue",
    "ticket",
    "phase",
    "level",
    "room",
    "port",
    "pr",
    "section",
    "figure",
    "item",
    "episode",
    "season",
    "volume",
    "slide",
    "gate",
    "floor",
    "row",
    "column",
    "seat",
    "flight",
    "route",
    "platform",
    "sprint",
    "question",
    "exercise",
    "lesson",
    "article",
    "error",
    "extension",
    "v",
];

/// Labels that are also common verbs ("build two apps", "track two metrics").
/// They label a value only when it is dotted or ends the phrase.
const VERB_LABEL_WORDS: &[&str] = &[
    "build", "release", "model", "rank", "table", "part", "track", "pin",
];

/// Labels whose value may be a dotted version such as `2.17.2`.
const VERSION_LABELS: &[&str] = &["version", "release", "build", "v"];

/// Measurement, currency, and percentage words that keep a count numeric.
/// Durations such as minutes or days are deliberately absent: prose writes
/// "two days" with words.
const UNIT_WORDS: &[&str] = &[
    "percent",
    "percentage",
    "dollar",
    "dollars",
    "buck",
    "bucks",
    "cent",
    "cents",
    "euro",
    "euros",
    "pound",
    "pounds",
    "yen",
    "rupee",
    "rupees",
    "degree",
    "degrees",
    "kilogram",
    "kilograms",
    "kg",
    "gram",
    "grams",
    "milligram",
    "milligrams",
    "mg",
    "lb",
    "lbs",
    "ounce",
    "ounces",
    "oz",
    "ton",
    "tons",
    "tonne",
    "tonnes",
    "mile",
    "miles",
    "kilometer",
    "kilometers",
    "kilometre",
    "kilometres",
    "km",
    "meter",
    "meters",
    "metre",
    "metres",
    "centimeter",
    "centimeters",
    "cm",
    "millimeter",
    "millimeters",
    "mm",
    "inch",
    "inches",
    "feet",
    "ft",
    "yard",
    "yards",
    "liter",
    "liters",
    "litre",
    "litres",
    "milliliter",
    "milliliters",
    "ml",
    "gallon",
    "gallons",
    "mph",
    "kph",
    "hz",
    "khz",
    "mhz",
    "ghz",
    "byte",
    "bytes",
    "bit",
    "bits",
    "kb",
    "mb",
    "gb",
    "tb",
    "kilobyte",
    "kilobytes",
    "megabyte",
    "megabytes",
    "gigabyte",
    "gigabytes",
    "terabyte",
    "terabytes",
    "millisecond",
    "milliseconds",
    "ms",
    "px",
    "pixel",
    "pixels",
    "fps",
    "volt",
    "volts",
    "watt",
    "watts",
    "amp",
    "amps",
];

/// Words that make an `hour minutes` pair a clock time.
const TIME_CUES: &[&str] = &[
    "at", "by", "until", "till", "from", "before", "after", "since",
];

/// Words that make a spoken `two thousand twenty six` a year.
const YEAR_CUES: &[&str] = &[
    "in", "since", "year", "until", "till", "from", "before", "after", "during", "circa",
];

/// Words after which a leading `point five` is a decimal rather than the noun
/// in phrases such as "at this point five people".
const LEADING_POINT_CONTEXT: &[&str] = &[
    "and",
    "or",
    "is",
    "was",
    "are",
    "were",
    "be",
    "equals",
    "equal",
    "of",
    "by",
    "minus",
    "plus",
    "times",
    "about",
    "around",
    "approximately",
    "roughly",
    "nearly",
    "almost",
    "only",
    "just",
    "than",
    "to",
];

/// Connectors that join small counts such as "two or three days".
const COUNT_COORDINATORS: &[&str] = &["and", "or", "to", "through"];

/// Month names. `true` marks names that are also ordinary words; those must be
/// capitalized and followed by an ordinal day ("May fifth", not "may five").
const MONTHS: &[(&str, bool)] = &[
    ("january", false),
    ("february", false),
    ("march", true),
    ("april", false),
    ("may", true),
    ("june", false),
    ("july", false),
    ("august", true),
    ("september", false),
    ("october", false),
    ("november", false),
    ("december", false),
];

#[derive(Debug, Clone)]
struct WordSpan {
    start: usize,
    end: usize,
    lower: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastNumberPart {
    Unit,
    Teen,
    Tens,
    Hundred,
    Scale,
}

#[derive(Debug, Clone, Copy)]
struct CardinalState {
    total: u128,
    group: u128,
    last_scale: u128,
    last_part: Option<LastNumberPart>,
    saw_number: bool,
}

impl Default for CardinalState {
    fn default() -> Self {
        Self {
            total: 0,
            group: 0,
            last_scale: u128::MAX,
            last_part: None,
            saw_number: false,
        }
    }
}

#[derive(Debug)]
struct ParsedNumber {
    end_word: usize,
    rendered: String,
    /// The value of a plain non-negative integer span.
    value: Option<u128>,
}

enum NumberSpan {
    Digits(ParsedNumber),
    /// A number phrase this policy deliberately leaves as words.
    Words {
        end_word: usize,
    },
}

/// Convert bounded English number phrases to decimal digits under `policy`
/// while preserving every byte outside the matched word spans.
pub(crate) fn normalize_spoken_numbers(input: &str, policy: NumberPolicy) -> String {
    if input.trim().is_empty() || input.len() > MAX_SPOKEN_NUMBER_INPUT_BYTES {
        return input.to_string();
    }

    let words = word_spans(input);
    let mut output = String::with_capacity(input.len());
    let mut word_index = 0;
    let mut copied_through = 0;
    let mut changed = false;

    while word_index < words.len() {
        // Dates and clock times outrank the prose `one` rule so that
        // "September first" and "one PM" still render digits.
        let span = match parse_calendar_or_clock(&words, input, word_index) {
            Some(parsed) => Some(NumberSpan::Digits(parsed)),
            None if should_preserve_prose_one(&words, input, word_index) => {
                if words[word_index].lower == "1" {
                    output.push_str(&input[copied_through..words[word_index].start]);
                    output.push_str("one");
                    copied_through = words[word_index].end;
                    changed = true;
                }
                word_index += 1;
                continue;
            }
            None => next_number_span(&words, input, word_index, policy),
        };

        match span {
            None => word_index += 1,
            Some(NumberSpan::Words { end_word }) => word_index = end_word.max(word_index + 1),
            Some(NumberSpan::Digits(parsed)) => {
                output.push_str(&input[copied_through..words[word_index].start]);
                output.push_str(&parsed.rendered);
                copied_through = words[parsed.end_word - 1].end;
                word_index = parsed.end_word;
                changed = true;
            }
        }
    }

    if !changed {
        return input.to_string();
    }
    output.push_str(&input[copied_through..]);
    output
}

fn next_number_span(
    words: &[WordSpan],
    input: &str,
    start: usize,
    policy: NumberPolicy,
) -> Option<NumberSpan> {
    let structured = parse_labelled(words, input, start, policy)
        .or_else(|| parse_dotted(words, input, start, policy, false))
        .or_else(|| {
            (policy == NumberPolicy::Technical)
                .then(|| parse_digit_run(words, input, start, true, false))
                .flatten()
        });
    if let Some(parsed) = structured {
        return Some(NumberSpan::Digits(parsed));
    }

    if let Some((value, end_word)) = parse_ordinal(words, input, start, false) {
        // Unit ordinals ("first" through "ninth") are never rewritten outside
        // a date. Larger ordinals become digits only where digits are chosen.
        if value < 10 {
            return None;
        }
        return Some(match policy {
            NumberPolicy::Technical => NumberSpan::Digits(ParsedNumber {
                end_word,
                rendered: format!("{value}{}", ordinal_suffix(value)),
                value: None,
            }),
            NumberPolicy::Prose => NumberSpan::Words { end_word },
        });
    }

    let mut parsed = parse_existing_large_integer(words, input, start, policy)
        .or_else(|| parse_number(words, input, start))?;
    // "two-factor", "one-on-one", and "two-by-four" are words, not values.
    if hyphen_attached(words, input, start, parsed.end_word) {
        return Some(NumberSpan::Words {
            end_word: parsed.end_word,
        });
    }
    if policy == NumberPolicy::Prose && parsed.end_word == start + 1 {
        if let Some(end_word) = prose_count_group(words, input, start) {
            return Some(NumberSpan::Words { end_word });
        }
    }
    if let Some(value) = parsed.value {
        // "in two thousand twenty six" is a year, so it takes no separator.
        if (1_000..=2_999).contains(&value)
            && value % 1_000 < 100
            && previous_word_in(words, input, start, YEAR_CUES)
        {
            parsed.rendered = value.to_string();
        }
    }
    Some(NumberSpan::Digits(parsed))
}

fn parse_calendar_or_clock(words: &[WordSpan], input: &str, start: usize) -> Option<ParsedNumber> {
    parse_month_day(words, input, start)
        .or_else(|| parse_ordinal_before_month(words, input, start))
        .or_else(|| {
            parse_year_form(words, input, start).map(|(year, end_word)| ParsedNumber {
                end_word,
                rendered: year.to_string(),
                value: None,
            })
        })
        .or_else(|| parse_clock_time(words, input, start))
}

/// `September seventeenth` → `September 17`, optionally followed by a year:
/// `September seventeenth twenty twenty six` → `September 17, 2026`.
fn parse_month_day(words: &[WordSpan], input: &str, start: usize) -> Option<ParsedNumber> {
    let month = start.checked_sub(1)?;
    let requires_ordinal = month_kind(words, input, month)?;
    if !space_between(words, input, month, start) || parse_year_form(words, input, start).is_some()
    {
        return None;
    }

    let (day, mut end_word) = match parse_ordinal(words, input, start, true) {
        Some(ordinal) => ordinal,
        None if requires_ordinal => return None,
        None => {
            let (value, end_word) = parse_cardinal(words, input, start)?;
            (u32::try_from(value).ok()?, end_word)
        }
    };
    if !(1..=31).contains(&day) || hyphen_attached(words, input, start, end_word) {
        return None;
    }

    let mut rendered = day.to_string();
    if let Some((year, year_end)) = following_year(words, input, end_word) {
        let separator = &input[words[end_word - 1].end..words[end_word].start];
        rendered.push_str(if separator.contains(',') {
            separator
        } else {
            ", "
        });
        rendered.push_str(&year.to_string());
        end_word = year_end;
    }
    Some(ParsedNumber {
        end_word,
        rendered,
        value: None,
    })
}

/// `the seventeenth of September` → `the 17th of September`.
fn parse_ordinal_before_month(
    words: &[WordSpan],
    input: &str,
    start: usize,
) -> Option<ParsedNumber> {
    let (day, end_word) = parse_ordinal(words, input, start, true)?;
    let of = words.get(end_word)?;
    if !(1..=31).contains(&day)
        || of.lower != "of"
        || !space_between(words, input, end_word - 1, end_word)
        || end_word + 1 >= words.len()
        || !space_between(words, input, end_word, end_word + 1)
        || month_kind(words, input, end_word + 1).is_none()
    {
        return None;
    }
    Some(ParsedNumber {
        end_word,
        rendered: format!("{day}{}", ordinal_suffix(day)),
        value: None,
    })
}

/// A year following a date, either a spoken year form or `two thousand ...`.
fn following_year(words: &[WordSpan], input: &str, start: usize) -> Option<(u128, usize)> {
    let next = words.get(start)?;
    let separator = &input[words[start - 1].end..next.start];
    if !separator
        .chars()
        .all(|character| matches!(character, ' ' | '\t' | ','))
        || separator.matches(',').count() > 1
    {
        return None;
    }
    if let Some(year) = parse_year_form(words, input, start) {
        return Some(year);
    }
    let (value, end_word) = parse_cardinal(words, input, start)?;
    let spoken_thousand = words[start..end_word]
        .iter()
        .any(|word| word.lower == "thousand");
    (spoken_thousand && (1_000..=2_999).contains(&value) && value % 1_000 < 100)
        .then_some((value, end_word))
}

/// `twenty twenty six` → 2026, `nineteen eighty four` → 1984, and
/// `twenty oh five` → 2005. A unit second half ("twenty one") is a cardinal.
fn parse_year_form(words: &[WordSpan], input: &str, start: usize) -> Option<(u128, usize)> {
    let century = match words.get(start)?.lower.as_str() {
        "eighteen" => 18,
        "nineteen" => 19,
        "twenty" => 20,
        _ => return None,
    };
    let second = start + 1;
    if second >= words.len() || !connected(words, input, start, second) {
        return None;
    }
    let (year, end_word) = match words[second].lower.as_str() {
        "oh" => {
            let unit = second + 1;
            let digit = words
                .get(unit)
                .filter(|_| connected(words, input, second, unit))
                .and_then(|word| small_number(&word.lower))
                .filter(|digit| (1..=9).contains(digit))?;
            (digit, unit + 1)
        }
        token => {
            if let Some(teen) = small_number(token).filter(|value| *value >= 10) {
                (teen, second + 1)
            } else {
                let tens = tens_number(token)?;
                match words
                    .get(second + 1)
                    .filter(|_| connected(words, input, second, second + 1))
                    .and_then(|word| small_number(&word.lower))
                    .filter(|unit| (1..=9).contains(unit))
                {
                    Some(unit) => (tens + unit, second + 2),
                    None => (tens, second + 1),
                }
            }
        }
    };
    let continues = words.get(end_word).is_some_and(|word| {
        connected(words, input, end_word - 1, end_word) && is_number_token(&word.lower)
    });
    if continues || hyphen_attached(words, input, start, end_word) {
        return None;
    }
    Some((century * 100 + year, end_word))
}

/// `four thirty PM` → `4:30 PM`, `at two thirty` → `at 2:30`, `four PM` →
/// `4 PM`, and `four o'clock` → `4 o'clock`. A bare `at two` stays words.
fn parse_clock_time(words: &[WordSpan], input: &str, start: usize) -> Option<ParsedNumber> {
    let hour = small_number(&words[start].lower).filter(|hour| (1..=12).contains(hour))?;
    let minute_start = start + 1;
    let minute_word = words
        .get(minute_start)
        .filter(|_| connected(words, input, start, minute_start));

    if minute_word.is_some_and(|word| matches!(word.lower.as_str(), "o'clock" | "oclock")) {
        return Some(ParsedNumber {
            end_word: start + 1,
            rendered: hour.to_string(),
            value: None,
        });
    }

    let minutes = minute_word.and_then(|word| match word.lower.as_str() {
        "oh" => words
            .get(minute_start + 1)
            .filter(|_| connected(words, input, minute_start, minute_start + 1))
            .and_then(|unit| small_number(&unit.lower))
            .filter(|unit| (1..=9).contains(unit))
            .map(|unit| (unit, minute_start + 2)),
        token => {
            if let Some(teen) = small_number(token).filter(|value| *value >= 10) {
                Some((teen, minute_start + 1))
            } else {
                let tens = tens_number(token).filter(|tens| *tens <= 50)?;
                Some(
                    match words
                        .get(minute_start + 1)
                        .filter(|_| connected(words, input, minute_start, minute_start + 1))
                        .and_then(|unit| small_number(&unit.lower))
                        .filter(|unit| (1..=9).contains(unit))
                    {
                        Some(unit) => (tens + unit, minute_start + 2),
                        None => (tens, minute_start + 1),
                    },
                )
            }
        }
    });

    match minutes {
        Some((minutes, end_word)) => {
            let continues = words.get(end_word).is_some_and(|word| {
                connected(words, input, end_word - 1, end_word) && is_number_token(&word.lower)
            });
            let cued = meridiem_follows(words, input, end_word)
                || previous_word_in(words, input, start, TIME_CUES);
            (cued && !continues && !hyphen_attached(words, input, start, end_word)).then(|| {
                ParsedNumber {
                    end_word,
                    rendered: format!("{hour}:{minutes:02}"),
                    value: None,
                }
            })
        }
        None => meridiem_follows(words, input, start + 1).then(|| ParsedNumber {
            end_word: start + 1,
            rendered: hour.to_string(),
            value: None,
        }),
    }
}

fn meridiem_follows(words: &[WordSpan], input: &str, index: usize) -> bool {
    let Some(word) = words.get(index) else {
        return false;
    };
    if index == 0 || !space_between(words, input, index - 1, index) {
        return false;
    }
    match word.lower.as_str() {
        "am" | "pm" => true,
        "a" | "p" => words
            .get(index + 1)
            .is_some_and(|m| m.lower == "m" && &input[word.end..m.start] == "."),
        _ => false,
    }
}

/// Label-cued values always use digits: `version 2.17.2`, `room 402`,
/// `port 8080`, `step 3`.
fn parse_labelled(
    words: &[WordSpan],
    input: &str,
    start: usize,
    policy: NumberPolicy,
) -> Option<ParsedNumber> {
    let label = start.checked_sub(1)?;
    let word = words[label].lower.as_str();
    if !space_between(words, input, label, start)
        || !(LABEL_WORDS.contains(&word) || VERB_LABEL_WORDS.contains(&word))
    {
        return None;
    }
    if let Some(dotted) = parse_dotted(words, input, start, policy, VERSION_LABELS.contains(&word))
    {
        return Some(dotted);
    }
    parse_digit_run(words, input, start, false, false)
        .or_else(|| {
            parse_number(words, input, start)
                .filter(|parsed| !hyphen_attached(words, input, start, parsed.end_word))
        })
        .filter(|parsed| label_applies(words, input, label, parsed.end_word))
}

/// Whether `label` labels a value ending at `end_word`. A verb-like label
/// needs the value to end the phrase: `in build two` but not `build two apps`.
fn label_applies(words: &[WordSpan], input: &str, label: usize, end_word: usize) -> bool {
    let word = words[label].lower.as_str();
    LABEL_WORDS.contains(&word)
        || (VERB_LABEL_WORDS.contains(&word)
            && words.get(end_word).is_none_or(|next| {
                !input[words[end_word - 1].end..next.start]
                    .chars()
                    .all(char::is_whitespace)
            }))
}

/// Dotted values: decimals with spoken segments (`three point eleven` →
/// `3.11`), versions, and IP-like addresses. `dot` separates segments only
/// after a version label or under the technical policy.
fn parse_dotted(
    words: &[WordSpan],
    input: &str,
    start: usize,
    policy: NumberPolicy,
    version_label: bool,
) -> Option<ParsedNumber> {
    let technical = policy == NumberPolicy::Technical;
    let (mut rendered, mut end_word) = dotted_segment(words, input, start, false)?;
    let mut segments = 1;
    while segments < MAX_DOTTED_SEGMENTS {
        let separator = end_word;
        let is_separator = words.get(separator).is_some_and(|word| {
            word.lower == "point" || ((version_label || technical) && word.lower == "dot")
        });
        if !is_separator
            || !connected(words, input, separator - 1, separator)
            || separator + 1 >= words.len()
            || !connected(words, input, separator, separator + 1)
        {
            break;
        }
        let Some((segment, segment_end)) = dotted_segment(words, input, separator + 1, true) else {
            break;
        };
        rendered.push('.');
        rendered.push_str(&segment);
        end_word = segment_end;
        segments += 1;
    }
    (segments >= 2 && !hyphen_attached(words, input, start, end_word)).then_some(ParsedNumber {
        end_word,
        rendered,
        value: None,
    })
}

/// One dotted segment: a joined digit run (`one ninety two` → `192`) or a
/// cardinal below one thousand.
fn dotted_segment(
    words: &[WordSpan],
    input: &str,
    start: usize,
    after_separator: bool,
) -> Option<(String, usize)> {
    if let Some(run) = parse_digit_run(words, input, start, false, after_separator) {
        return Some((run.rendered, run.end_word));
    }
    if after_separator && words[start].lower == "oh" {
        return Some(("0".to_string(), start + 1));
    }
    let (value, end_word) = parse_cardinal(words, input, start)?;
    (value < 1_000).then(|| (value.to_string(), end_word))
}

/// Join two or more spoken digit chunks: `eight zero eight zero` → `8080`,
/// `four oh two` → `402`. With `single_digits_only`, every chunk is one digit.
fn parse_digit_run(
    words: &[WordSpan],
    input: &str,
    start: usize,
    single_digits_only: bool,
    leading_oh: bool,
) -> Option<ParsedNumber> {
    let mut digits = String::new();
    let mut index = start;
    let mut chunks = 0;
    while index < words.len() && chunks < MAX_NUMBER_WORDS {
        if index > start && !connected(words, input, index - 1, index) {
            break;
        }
        let Some((chunk, end)) = digit_chunk(words, input, index, chunks > 0 || leading_oh) else {
            break;
        };
        if single_digits_only && chunk.len() != 1 {
            break;
        }
        digits.push_str(&chunk);
        chunks += 1;
        index = end;
    }
    if chunks < 2 {
        return None;
    }
    // A following scale makes the phrase a cardinal ("two twenty thousand").
    let scale_follows = words.get(index).is_some_and(|word| {
        connected(words, input, index - 1, index)
            && (word.lower == "hundred" || scale_number(&word.lower).is_some())
    });
    (!scale_follows && !hyphen_attached(words, input, start, index)).then_some(ParsedNumber {
        end_word: index,
        rendered: digits,
        value: None,
    })
}

fn digit_chunk(
    words: &[WordSpan],
    input: &str,
    index: usize,
    allow_oh: bool,
) -> Option<(String, usize)> {
    let token = words[index].lower.as_str();
    if allow_oh && token == "oh" {
        return Some(("0".to_string(), index + 1));
    }
    if let Some(value) = small_number(token) {
        return Some((value.to_string(), index + 1));
    }
    let tens = tens_number(token)?;
    let unit = words
        .get(index + 1)
        .filter(|_| connected(words, input, index, index + 1))
        .and_then(|word| small_number(&word.lower))
        .filter(|unit| (1..=9).contains(unit));
    Some(match unit {
        Some(unit) => ((tens + unit).to_string(), index + 2),
        None => (tens.to_string(), index + 1),
    })
}

/// Parse `seventeenth`, `twentieth`, or `twenty-first` / `twenty first`.
/// `second` is a unit of time outside dates ("twenty second timeout"), so it
/// counts as an ordinal only when `allow_second` is set.
fn parse_ordinal(
    words: &[WordSpan],
    input: &str,
    start: usize,
    allow_second: bool,
) -> Option<(u32, usize)> {
    let token = words.get(start)?.lower.as_str();
    if let Some(value) = ordinal_number(token) {
        return (allow_second || token != "second").then_some((value, start + 1));
    }
    let tens = u32::try_from(tens_number(token)?).ok()?;
    let unit_index = start + 1;
    let unit_word = words
        .get(unit_index)
        .filter(|_| connected(words, input, start, unit_index))?;
    let unit = ordinal_number(&unit_word.lower).filter(|unit| *unit <= 9)?;
    (allow_second || unit_word.lower != "second").then_some((tens + unit, unit_index + 1))
}

fn ordinal_number(word: &str) -> Option<u32> {
    Some(match word {
        "first" => 1,
        "second" => 2,
        "third" => 3,
        "fourth" => 4,
        "fifth" => 5,
        "sixth" => 6,
        "seventh" => 7,
        "eighth" => 8,
        "ninth" => 9,
        "tenth" => 10,
        "eleventh" => 11,
        "twelfth" => 12,
        "thirteenth" => 13,
        "fourteenth" => 14,
        "fifteenth" => 15,
        "sixteenth" => 16,
        "seventeenth" => 17,
        "eighteenth" => 18,
        "nineteenth" => 19,
        "twentieth" => 20,
        "thirtieth" => 30,
        "fortieth" => 40,
        "fiftieth" => 50,
        "sixtieth" => 60,
        "seventieth" => 70,
        "eightieth" => 80,
        "ninetieth" => 90,
        _ => return None,
    })
}

fn ordinal_suffix(value: u32) -> &'static str {
    match (value % 10, value % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    }
}

/// `Some(requires_ordinal)` when the word is a month name usable in a date.
fn month_kind(words: &[WordSpan], input: &str, index: usize) -> Option<bool> {
    let word = words.get(index)?;
    let (_, ambiguous) = MONTHS.iter().find(|(name, _)| *name == word.lower)?;
    let capitalized = input[word.start..]
        .chars()
        .next()
        .is_some_and(char::is_uppercase);
    (!ambiguous || capitalized).then_some(*ambiguous)
}

/// In prose, small counts attached to ordinary language stay words: "two of
/// them", "zero tolerance", "two or three days". Returns the end of such a
/// word group, or `None` when any member or its context needs digits.
fn prose_count_group(words: &[WordSpan], input: &str, start: usize) -> Option<usize> {
    if !is_small_count(words, input, start) {
        return None;
    }
    // A count that continues an earlier rendered number keeps its digit form
    // ("step 2 or 3", "2, 3").
    if let Some(previous) = start.checked_sub(1) {
        if list_joined(words, input, previous, start) && is_number_token(&words[previous].lower) {
            return None;
        }
        if COUNT_COORDINATORS.contains(&words[previous].lower.as_str())
            && connected(words, input, previous, start)
            && previous.checked_sub(1).is_some_and(|before| {
                (connected(words, input, before, previous)
                    || list_joined(words, input, before, previous))
                    && is_number_token(&words[before].lower)
            })
        {
            return None;
        }
    }

    let mut last = start;
    let mut members = 1;
    while members < MAX_NUMBER_WORDS {
        let next = last + 1;
        let Some(word) = words.get(next) else {
            break;
        };
        let member = if list_joined(words, input, last, next) && is_number_token(&word.lower) {
            next
        } else if COUNT_COORDINATORS.contains(&word.lower.as_str())
            && (connected(words, input, last, next) || list_joined(words, input, last, next))
            && words
                .get(next + 1)
                .is_some_and(|candidate| is_number_token(&candidate.lower))
            && connected(words, input, next, next + 1)
        {
            next + 1
        } else {
            break;
        };
        if !is_small_count(words, input, member) {
            return None;
        }
        last = member;
        members += 1;
    }

    let end = last + 1;
    let attached_before = start.checked_sub(1).is_some_and(|previous| {
        connected(words, input, previous, start) && !is_value_token(&words[previous].lower)
    });
    let attached_after = words
        .get(end)
        .is_some_and(|next| connected(words, input, last, end) && !is_value_token(&next.lower));
    let numeric_context = unit_follows(words, input, last)
        || currency_precedes(input, words[start].start)
        || meridiem_follows(words, input, end);
    ((attached_before || attached_after) && !numeric_context).then_some(end)
}

/// A lone spoken word from zero through nine that is not part of a compound,
/// decimal, fraction, or space-separated digit sequence.
fn is_small_count(words: &[WordSpan], input: &str, index: usize) -> bool {
    let Some(word) = words.get(index) else {
        return false;
    };
    if !word.lower.bytes().all(|byte| byte.is_ascii_alphabetic())
        || small_number(&word.lower).is_none_or(|value| value > 9)
        || hyphen_attached(words, input, index, index + 1)
    {
        return false;
    }
    // `point`, `negative`, and `minus` here were not consumed as part of a
    // value, so they are ordinary words ("at this point five people").
    let numeric_neighbor = |left: usize, right: usize, neighbor: usize| {
        is_value_token(&words[neighbor].lower)
            && (connected(words, input, left, right)
                || input[words[left].end..words[right].start].contains('/'))
    };
    !(index
        .checked_sub(1)
        .is_some_and(|previous| numeric_neighbor(previous, index, previous))
        || (index + 1 < words.len() && numeric_neighbor(index, index + 1, index + 1)))
}

fn list_joined(words: &[WordSpan], input: &str, left: usize, right: usize) -> bool {
    let separator = &input[words[left].end..words[right].start];
    separator.contains([',', ';'])
        && separator
            .chars()
            .all(|character| matches!(character, ' ' | '\t' | ',' | ';'))
}

fn unit_follows(words: &[WordSpan], input: &str, index: usize) -> bool {
    let Some(word) = words.get(index) else {
        return false;
    };
    input[word.end..]
        .trim_start_matches([' ', '\t'])
        .starts_with('%')
        || words.get(index + 1).is_some_and(|next| {
            space_between(words, input, index, index + 1)
                && UNIT_WORDS.contains(&next.lower.as_str())
        })
}

fn currency_precedes(input: &str, start: usize) -> bool {
    input[..start]
        .trim_end_matches([' ', '\t'])
        .ends_with(['$', '€', '£', '¥', '₹'])
}

fn hyphen_attached(words: &[WordSpan], input: &str, start: usize, end: usize) -> bool {
    let hyphenated = |left: usize, right: usize| {
        connected(words, input, left, right)
            && input[words[left].end..words[right].start].contains(['-', '‑'])
    };
    start.checked_sub(1).is_some_and(|previous| {
        hyphenated(previous, start) && !is_number_token(&words[previous].lower)
    }) || words
        .get(end)
        .is_some_and(|next| hyphenated(end - 1, end) && !is_number_token(&next.lower))
}

fn previous_word_in(words: &[WordSpan], input: &str, start: usize, set: &[&str]) -> bool {
    start.checked_sub(1).is_some_and(|previous| {
        space_between(words, input, previous, start)
            && set.contains(&words[previous].lower.as_str())
    })
}

fn parse_number(words: &[WordSpan], input: &str, start: usize) -> Option<ParsedNumber> {
    let mut number_start = start;
    let negative = matches!(words[start].lower.as_str(), "negative" | "minus");
    if negative {
        number_start += 1;
        if number_start >= words.len() || !connected(words, input, start, number_start) {
            return None;
        }
    }

    let (integer, integer_end) =
        parse_cardinal(words, input, number_start).unwrap_or((0, number_start));
    let has_integer = integer_end > number_start;

    let point_index = if has_integer {
        integer_end
    } else {
        number_start
    };
    if words
        .get(point_index)
        .is_some_and(|word| word.lower == "point")
        && (has_integer || negative || leading_point_allowed(words, input, point_index))
        && (point_index == number_start || connected(words, input, point_index - 1, point_index))
    {
        let fraction_start = point_index + 1;
        let mut fraction_end = fraction_start;
        let mut fraction = String::new();
        while fraction_end < words.len()
            && fraction_end - fraction_start < MAX_FRACTION_DIGITS
            && (fraction_end == fraction_start
                || connected(words, input, fraction_end - 1, fraction_end))
        {
            let Some(digit) = decimal_digit(&words[fraction_end].lower) else {
                break;
            };
            fraction.push(char::from(b'0' + digit));
            fraction_end += 1;
        }
        if !fraction.is_empty() {
            return Some(ParsedNumber {
                end_word: fraction_end,
                rendered: format!(
                    "{}{}.{}",
                    if negative { "-" } else { "" },
                    format_integer(integer),
                    fraction
                ),
                value: None,
            });
        }
    }

    has_integer.then(|| ParsedNumber {
        end_word: integer_end,
        rendered: format!(
            "{}{}",
            if negative { "-" } else { "" },
            format_integer(integer)
        ),
        value: (!negative).then_some(integer),
    })
}

/// A decimal without an integer part (`point oh five`) is accepted only at a
/// clause start or after a word that introduces a value. "at this point five
/// people" keeps the noun.
fn leading_point_allowed(words: &[WordSpan], input: &str, index: usize) -> bool {
    let Some(previous) = index.checked_sub(1) else {
        return true;
    };
    !space_between(words, input, previous, index)
        || LEADING_POINT_CONTEXT.contains(&words[previous].lower.as_str())
}

fn parse_existing_large_integer(
    words: &[WordSpan],
    input: &str,
    start: usize,
    policy: NumberPolicy,
) -> Option<ParsedNumber> {
    // Technical identifiers and labelled values (`PR 12345`) keep their exact
    // digits, which also keeps joined digit runs idempotent.
    if policy == NumberPolicy::Technical
        || start.checked_sub(1).is_some_and(|label| {
            let word = words[label].lower.as_str();
            LABEL_WORDS.contains(&word) || VERB_LABEL_WORDS.contains(&word)
        })
    {
        return None;
    }
    let word = words.get(start)?;
    let digits = &input[word.start..word.end];
    if digits.len() < 5
        || digits.starts_with('0')
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    // Do not insert a thousands separator into the fractional side of a
    // decimal such as `2.12345678`.
    if start > 0
        && words[start - 1]
            .lower
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        && input[words[start - 1].end..word.start] == *"."
    {
        return None;
    }

    let value = digits.parse::<u128>().ok()?;
    Some(ParsedNumber {
        end_word: start + 1,
        rendered: format_integer(value),
        value: Some(value),
    })
}

fn should_preserve_prose_one(words: &[WordSpan], input: &str, start: usize) -> bool {
    let Some(word) = words.get(start) else {
        return false;
    };
    if !matches!(word.lower.as_str(), "one" | "1") {
        return false;
    }

    // Keep already-numeric decimal and grouped forms numeric. Their adjacent
    // punctuation splits them into multiple WordSpans.
    if word.lower == "1" && is_numeric_component(input, word) {
        return false;
    }

    // Compound values and explicit sequences are unambiguously numeric.
    if adjacent_number(words, input, start, start + 1)
        || start
            .checked_sub(1)
            .is_some_and(|previous| adjacent_number(words, input, start, previous))
        || coordinated_number(words, input, start, true)
        || coordinated_number(words, input, start, false)
    {
        return false;
    }

    // Labels, times, units, and currency conventionally use digits even
    // though the value is isolated.
    if (start > 0
        && connected(words, input, start - 1, start)
        && label_applies(words, input, start - 1, start + 1))
        || meridiem_follows(words, input, start + 1)
        || unit_follows(words, input, start)
        || currency_precedes(input, word.start)
    {
        return false;
    }

    // An isolated one attached to ordinary language acts as a determiner or
    // pronoun ("one idea", "one day", "that one"). A bare utterance "one"
    // remains numeric to preserve the explicit number-dictation contract.
    (start > 0 && connected(words, input, start - 1, start))
        || (start + 1 < words.len() && connected(words, input, start, start + 1))
}

fn adjacent_number(words: &[WordSpan], input: &str, start: usize, other: usize) -> bool {
    let Some(other_word) = words.get(other) else {
        return false;
    };
    let separator = if other < start {
        &input[other_word.end..words[start].start]
    } else {
        &input[words[start].end..other_word.start]
    };
    // A following `point` is numeric only when a decimal digit follows it
    // ("one point five", not "the one point I want to make").
    let decimal_point = other_word.lower != "point"
        || other < start
        || words
            .get(other + 1)
            .is_some_and(|word| decimal_digit(&word.lower).is_some());
    separator
        .chars()
        .all(|character| matches!(character, ' ' | '\t' | ',' | ';' | '/'))
        && is_number_token(&other_word.lower)
        && decimal_point
}

fn coordinated_number(words: &[WordSpan], input: &str, start: usize, look_forward: bool) -> bool {
    let (connector_index, number_index) = if look_forward {
        (start + 1, start + 2)
    } else {
        let Some(connector) = start.checked_sub(1) else {
            return false;
        };
        let Some(number) = start.checked_sub(2) else {
            return false;
        };
        (connector, number)
    };
    let Some(connector) = words.get(connector_index) else {
        return false;
    };
    let Some(number) = words.get(number_index) else {
        return false;
    };
    matches!(connector.lower.as_str(), "and" | "or" | "to" | "through")
        && connected(
            words,
            input,
            start.min(connector_index),
            start.max(connector_index),
        )
        && connected(
            words,
            input,
            connector_index.min(number_index),
            connector_index.max(number_index),
        )
        && is_number_token(&number.lower)
}

fn is_number_token(token: &str) -> bool {
    token.bytes().all(|byte| byte.is_ascii_digit())
        || small_number(token).is_some()
        || tens_number(token).is_some()
        || matches!(
            token,
            "hundred"
                | "thousand"
                | "million"
                | "billion"
                | "trillion"
                | "quadrillion"
                | "point"
                | "negative"
                | "minus"
        )
}

fn is_value_token(token: &str) -> bool {
    is_number_token(token) && !matches!(token, "point" | "negative" | "minus")
}

fn is_numeric_component(input: &str, word: &WordSpan) -> bool {
    let before = input[..word.start].chars().next_back();
    let after = input[word.end..].chars().next();
    (matches!(before, Some('.' | ','))
        && input[..word.start]
            .chars()
            .rev()
            .nth(1)
            .is_some_and(|character| character.is_ascii_digit()))
        || (matches!(after, Some('.' | ','))
            && input[word.end..]
                .chars()
                .nth(1)
                .is_some_and(|character| character.is_ascii_digit()))
}

fn format_integer(value: u128) -> String {
    let digits = value.to_string();
    if digits.len() <= 3 {
        return digits;
    }

    let first_group_len = match digits.len() % 3 {
        0 => 3,
        remainder => remainder,
    };
    let mut grouped = String::with_capacity(digits.len() + (digits.len() - 1) / 3);
    grouped.push_str(&digits[..first_group_len]);
    for group_start in (first_group_len..digits.len()).step_by(3) {
        grouped.push(',');
        grouped.push_str(&digits[group_start..group_start + 3]);
    }
    grouped
}

fn parse_cardinal(words: &[WordSpan], input: &str, start: usize) -> Option<(u128, usize)> {
    let mut state = CardinalState::default();
    let mut index = start;
    let mut consumed = 0;

    while index < words.len() && consumed < MAX_NUMBER_WORDS {
        if index > start && !connected(words, input, index - 1, index) {
            break;
        }
        let token = words[index].lower.as_str();

        if token == "a" {
            let next = words.get(index + 1);
            let valid_article = !state.saw_number
                && next.is_some_and(|word| {
                    matches!(
                        word.lower.as_str(),
                        "hundred" | "thousand" | "million" | "billion" | "trillion" | "quadrillion"
                    )
                })
                && next.is_some_and(|_| connected(words, input, index, index + 1));
            if !valid_article || !apply_value(&mut state, 1) {
                break;
            }
            index += 1;
            consumed += 1;
            continue;
        }

        if token == "and" {
            let next_index = index + 1;
            if !state.saw_number
                || next_index >= words.len()
                || !connected(words, input, index, next_index)
            {
                break;
            }
            let mut trial = state;
            if !apply_token(&mut trial, &words[next_index].lower) {
                break;
            }
            index += 1;
            consumed += 1;
            continue;
        }

        if !apply_token(&mut state, token) {
            break;
        }
        index += 1;
        consumed += 1;
    }

    if !state.saw_number {
        return None;
    }
    state
        .total
        .checked_add(state.group)
        .map(|value| (value, index))
}

fn apply_token(state: &mut CardinalState, token: &str) -> bool {
    if let Some(value) = small_number(token) {
        return apply_value(state, value);
    }
    if let Some(value) = tens_number(token) {
        let allowed = !state.saw_number
            || state.last_part == Some(LastNumberPart::Scale)
            || matches!(state.last_part, Some(LastNumberPart::Hundred));
        if !allowed {
            return false;
        }
        state.group = match state.group.checked_add(value) {
            Some(group) => group,
            None => return false,
        };
        state.last_part = Some(LastNumberPart::Tens);
        state.saw_number = true;
        return true;
    }
    if token == "hundred" {
        if state.group == 0 && !state.saw_number {
            state.group = 100;
        } else if state.group <= 99
            && matches!(
                state.last_part,
                Some(LastNumberPart::Unit | LastNumberPart::Teen | LastNumberPart::Tens)
            )
        {
            state.group *= 100;
        } else {
            return false;
        }
        state.last_part = Some(LastNumberPart::Hundred);
        state.saw_number = true;
        return true;
    }
    if let Some(scale) = scale_number(token) {
        if state.group == 0 && !state.saw_number {
            state.group = 1;
        }
        if state.group == 0 || scale >= state.last_scale {
            return false;
        }
        let scaled = match state.group.checked_mul(scale) {
            Some(value) => value,
            None => return false,
        };
        state.total = match state.total.checked_add(scaled) {
            Some(value) => value,
            None => return false,
        };
        state.group = 0;
        state.last_scale = scale;
        state.last_part = Some(LastNumberPart::Scale);
        state.saw_number = true;
        return true;
    }
    false
}

fn apply_value(state: &mut CardinalState, value: u128) -> bool {
    let allowed = !state.saw_number
        || state.last_part == Some(LastNumberPart::Scale)
        || (state.last_part == Some(LastNumberPart::Tens) && value <= 9)
        || state.last_part == Some(LastNumberPart::Hundred);
    if !allowed || (value == 0 && state.group != 0) {
        return false;
    }
    state.group = match state.group.checked_add(value) {
        Some(group) => group,
        None => return false,
    };
    state.last_part = Some(if value <= 9 {
        LastNumberPart::Unit
    } else {
        LastNumberPart::Teen
    });
    state.saw_number = true;
    true
}

fn small_number(word: &str) -> Option<u128> {
    Some(match word {
        "zero" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "thirteen" => 13,
        "fourteen" => 14,
        "fifteen" => 15,
        "sixteen" => 16,
        "seventeen" => 17,
        "eighteen" => 18,
        "nineteen" => 19,
        _ => return None,
    })
}

fn tens_number(word: &str) -> Option<u128> {
    Some(match word {
        "twenty" => 20,
        "thirty" => 30,
        "forty" => 40,
        "fifty" => 50,
        "sixty" => 60,
        "seventy" => 70,
        "eighty" => 80,
        "ninety" => 90,
        _ => return None,
    })
}

fn scale_number(word: &str) -> Option<u128> {
    Some(match word {
        "thousand" => 1_000,
        "million" => 1_000_000,
        "billion" => 1_000_000_000,
        "trillion" => 1_000_000_000_000,
        "quadrillion" => 1_000_000_000_000_000,
        _ => return None,
    })
}

fn decimal_digit(word: &str) -> Option<u8> {
    Some(match word {
        "zero" | "oh" => 0,
        "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        _ => return None,
    })
}

fn connected(words: &[WordSpan], input: &str, left: usize, right: usize) -> bool {
    input[words[left].end..words[right].start]
        .chars()
        .all(|character| matches!(character, ' ' | '\t' | '-' | '‑'))
}

/// Words separated by plain spaces only (no hyphen or punctuation).
fn space_between(words: &[WordSpan], input: &str, left: usize, right: usize) -> bool {
    let separator = &input[words[left].end..words[right].start];
    !separator.is_empty()
        && separator
            .chars()
            .all(|character| matches!(character, ' ' | '\t'))
}

fn word_spans(input: &str) -> Vec<WordSpan> {
    let mut words = Vec::new();
    let mut start = None;
    for (index, character) in input.char_indices() {
        if character.is_alphanumeric() || matches!(character, '_' | '\'') {
            start.get_or_insert(index);
        } else if let Some(word_start) = start.take() {
            words.push(WordSpan {
                start: word_start,
                end: index,
                lower: input[word_start..index].to_lowercase(),
            });
        }
    }
    if let Some(word_start) = start {
        words.push(WordSpan {
            start: word_start,
            end: input.len(),
            lower: input[word_start..].to_lowercase(),
        });
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prose(input: &str) -> String {
        normalize_spoken_numbers(input, NumberPolicy::Prose)
    }

    fn technical(input: &str) -> String {
        normalize_spoken_numbers(input, NumberPolicy::Technical)
    }

    /// Cases whose rendering does not depend on the policy.
    fn both(input: &str, expected: &str) {
        assert_eq!(prose(input), expected, "prose: {input}");
        assert_eq!(technical(input), expected, "technical: {input}");
    }

    #[test]
    fn converts_single_and_separated_numbers() {
        both("one, two, three, four, five, six", "1, 2, 3, 4, 5, 6");
        assert_eq!(prose("zero one two"), "0 1 2");
        both("I need ten copies", "I need 10 copies");
    }

    #[test]
    fn converts_compound_and_hyphenated_cardinals() {
        both(
            "The result is eight hundred fifty-seven.",
            "The result is 857.",
        );
        both("fifty-seven", "57");
        both("twelve hundred thirty", "1,230");
        both("one hundred and five", "105");
    }

    #[test]
    fn converts_large_descending_scales() {
        both(
            "ten million one hundred three thousand four hundred forty-five",
            "10,103,445",
        );
        both(
            "nine quadrillion eight trillion seven billion six million five thousand four",
            "9,008,007,006,005,004",
        );
    }

    #[test]
    fn groups_spoken_and_existing_large_integers() {
        both(
            "thirty-five million four hundred fifty-five thousand thirty-four",
            "35,455,034",
        );
        assert_eq!(prose("35455034"), "35,455,034");
        both("one million point five", "1,000,000.5");
        both("one thousand", "1,000");
    }

    #[test]
    fn existing_years_leading_zeroes_and_decimal_fractions_are_not_grouped() {
        for input in ["2026", "01234567", "2.12345678", "123"] {
            both(input, input);
        }
    }

    #[test]
    fn isolated_one_remains_spelled_out_in_prose() {
        both(
            "one thing, one more thing, one idea, that one, and maybe one day",
            "one thing, one more thing, one idea, that one, and maybe one day",
        );
        both(
            "1 thing, 1 idea, that 1, and 1 day",
            "one thing, one idea, that one, and one day",
        );
        both("one hundred things", "100 things");
    }

    #[test]
    fn explicit_one_sequences_compounds_and_labels_remain_numeric() {
        both("one", "1");
        both("one, two, three", "1, 2, 3");
        both("one or two", "1 or 2");
        both(
            "number one, step one, and version one",
            "number 1, step 1, and version 1",
        );
        both("one hundred, one point five, negative one", "100, 1.5, -1");
        both("1,000 and 1.5", "1,000 and 1.5");
        both("maybe like one/2", "maybe like 1/2");
    }

    #[test]
    fn converts_articles_negatives_and_decimals() {
        both(
            "a hundred reasons and a million more",
            "100 reasons and 1,000,000 more",
        );
        both("negative forty-two", "-42");
        both("three point one four and point oh five", "3.14 and 0.05");
    }

    #[test]
    fn leaves_non_number_articles_and_existing_digits_alone() {
        let input = "A cat has 9 lives in version 2.0 with one2 one_two and oneé";
        both(input, input);
        both("one\nhundred", "1\n100");
    }

    #[test]
    fn malformed_scales_are_not_combined_into_a_guessed_value() {
        both("one thousand two million", "1,002 1,000,000");
        both("three hundred four hundred", "304 100");
    }

    #[test]
    fn prose_keeps_small_standalone_counts_as_words() {
        for input in [
            "two of them",
            "zero tolerance",
            "one of the best",
            "I have three kids and two dogs.",
            "two or three days",
            "We need two, three, or four people",
            "a couple of things",
            "meet at two",
        ] {
            assert_eq!(prose(input), input);
        }
        // Technical converts the same counts eagerly; the determiner `one`
        // stays a word in every policy.
        assert_eq!(technical("two of them"), "2 of them");
        assert_eq!(technical("zero tolerance"), "0 tolerance");
        assert_eq!(technical("one of the best"), "one of the best");
    }

    #[test]
    fn prose_still_converts_large_numbers_units_and_mixed_groups() {
        assert_eq!(prose("ten of them"), "10 of them");
        assert_eq!(prose("twenty five people"), "25 people");
        assert_eq!(prose("five percent of users"), "5 percent of users");
        assert_eq!(prose("it costs $ five"), "it costs $ 5");
        assert_eq!(prose("two or three dollars"), "2 or 3 dollars");
        assert_eq!(prose("nine or ten people"), "9 or 10 people");
        assert_eq!(prose("one or 12 options"), "1 or 12 options");
        assert_eq!(prose("one percent"), "1 percent");
        assert_eq!(prose("one foot in the door"), "one foot in the door");
        assert_eq!(prose("two feet"), "2 feet");
        assert_eq!(prose("five kilograms"), "5 kilograms");
        assert_eq!(prose("three point five people"), "3.5 people");
    }

    #[test]
    fn times_render_as_clock_values_in_every_policy() {
        both("four thirty PM", "4:30 PM");
        both("four thirty p.m.", "4:30 p.m.");
        both("meet at two thirty", "meet at 2:30");
        both("done by eleven forty-five", "done by 11:45");
        both("from ten oh five until noon", "from 10:05 until noon");
        both("four PM", "4 PM");
        both("one PM", "1 PM");
        both("five o'clock", "5 o'clock");
        both("1 PM", "1 PM");
        // Without a cue an hour/minute pair is not assumed to be a time.
        assert_eq!(technical("two thirty"), "2 30");
    }

    #[test]
    fn dates_and_years_render_as_digits_in_every_policy() {
        both("twenty twenty six", "2026");
        both("in nineteen eighty four", "in 1984");
        both("twenty oh five", "2005");
        both("in two thousand twenty six", "in 2026");
        both("the year two thousand", "the year 2000");
        both("September seventeenth", "September 17");
        both("September seventeen", "September 17");
        both("May first", "May 1");
        both("on March twenty-first", "on March 21");
        both(
            "September seventeenth twenty twenty six",
            "September 17, 2026",
        );
        both(
            "September seventeenth, two thousand twenty six",
            "September 17, 2026",
        );
        both("the fifth of November", "the 5th of November");
        both("September twenty twenty six", "September 2026");
        // Without a year cue a spoken thousand is a count.
        both("two thousand twenty six", "2,026");
        both("twenty one", "21");
        // Ambiguous month words need capitals and an ordinal day.
        assert_eq!(prose("you may two"), "you may two");
        assert_eq!(prose("May two of us come"), "May two of us come");
        assert_eq!(technical("you may two"), "you may 2");
    }

    #[test]
    fn labels_and_versions_use_digits_in_every_policy() {
        both("version two point seventeen point two", "version 2.17.2");
        both("version two point one", "version 2.1");
        both("release three dot one", "release 3.1");
        both("step two", "step 2");
        both("page twelve", "page 12");
        both("chapter three", "chapter 3");
        both("room four oh two", "room 402");
        both("port eight zero eight zero", "port 8080");
        both("port eighty eighty", "port 8080");
        both("PR seven nine one", "PR 791");
        both("issue twelve thirty four", "issue 1234");
        both("step two or three", "step 2 or 3");
        both("in build two", "in build 2");
        both("release two point one", "release 2.1");
        assert_eq!(prose("build two apps"), "build two apps");
        assert_eq!(prose("track two metrics"), "track two metrics");
        assert_eq!(prose("build one app"), "build one app");
        assert_eq!(technical("build two apps"), "build 2 apps");
        both("PR 12345", "PR 12345");
    }

    #[test]
    fn technical_joins_digit_runs_dotted_versions_and_addresses() {
        assert_eq!(technical("eight zero eight zero"), "8080");
        assert_eq!(technical("four oh four"), "404");
        assert_eq!(technical("zero one two"), "012");
        assert_eq!(technical("two point seventeen point two"), "2.17.2");
        assert_eq!(technical("two point seventeen"), "2.17");
        assert_eq!(
            technical("one ninety two point one sixty eight point one point one"),
            "192.168.1.1"
        );
        assert_eq!(technical("ten dot zero dot zero dot one"), "10.0.0.1");
        assert_eq!(technical("123456"), "123456");
        // Prose joins digit runs only after a label; dotted values are never
        // ambiguous with prose, so both policies render them.
        assert_eq!(prose("eight zero eight zero"), "8 0 8 0");
        assert_eq!(prose("two point seventeen point two"), "2.17.2");
        assert_eq!(prose("python three point eleven"), "python 3.11");
        assert_eq!(prose("ten dot zero dot one"), "10 dot zero dot one");
    }

    #[test]
    fn hyphenated_compounds_and_ordinals_are_never_split() {
        for input in [
            "two-factor auth",
            "a one-on-one",
            "a two-by-four",
            "three-way merge",
            "zero-copy reads",
            "two-thirds",
            "forty-five-minute call",
        ] {
            both(input, input);
        }
        assert_eq!(
            prose("the twenty-first century"),
            "the twenty-first century"
        );
        assert_eq!(prose("twenty first"), "twenty first");
        assert_eq!(prose("the seventeenth time"), "the seventeenth time");
        assert_eq!(technical("the twenty-first item"), "the 21st item");
        assert_eq!(technical("twenty first"), "21st");
        assert_eq!(technical("the twelfth run"), "the 12th run");
        assert_eq!(technical("twenty third"), "23rd");
        // Unit ordinals never change outside dates; `second` is also a unit.
        both("first second third", "first second third");
        assert_eq!(technical("a twenty second timeout"), "a 20 second timeout");
    }

    #[test]
    fn bare_point_is_a_decimal_only_in_a_value_context() {
        assert_eq!(
            prose("at this point five people"),
            "at this point five people"
        );
        // Technical renders the count eagerly but never invents a decimal.
        assert_eq!(
            technical("at this point five people"),
            "at this point 5 people"
        );
        assert_eq!(prose("the point five"), "the point five");
        both(
            "the one point I want to make",
            "the one point I want to make",
        );
        both("point five", "0.5");
        both("the ratio is point five", "the ratio is 0.5");
        both("negative point two", "-0.2");
    }

    #[test]
    fn normalization_is_idempotent() {
        for input in [
            "Call negative eight hundred fifty-seven at three point one four.",
            "thirty-five million four hundred fifty-five thousand thirty-four",
            "version two point seventeen point two on September seventeenth twenty twenty six",
            "meet at four thirty PM, then port eight zero eight zero",
            "two of them, the twenty-first century, and PR one two three four five",
        ] {
            for policy in [NumberPolicy::Prose, NumberPolicy::Technical] {
                let once = normalize_spoken_numbers(input, policy);
                assert_eq!(normalize_spoken_numbers(&once, policy), once, "{input}");
            }
        }
        assert_eq!(
            technical("one two three four five"),
            "12345",
            "joined runs are not regrouped"
        );
        assert_eq!(technical(&technical("one two three four five")), "12345");
    }

    #[test]
    fn over_limit_input_is_preserved() {
        let input = format!("one {}", "word ".repeat(MAX_SPOKEN_NUMBER_INPUT_BYTES));
        both(&input, &input);
        let long_run = "eight ".repeat(MAX_NUMBER_WORDS + 8);
        assert!(technical(&long_run).len() < long_run.len());
    }
}
