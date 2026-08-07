//! Deterministic English spoken-number normalization for live dictation.
//!
//! The parser is deliberately bounded and accepts only well-formed cardinal
//! groups. Invalid or out-of-order scale phrases are normalized only through
//! the last unambiguous group instead of being assigned a guessed value.

const MAX_NUMBER_WORDS: usize = 64;
const MAX_FRACTION_DIGITS: usize = 32;
const MAX_SPOKEN_NUMBER_INPUT_BYTES: usize = 16 * 1024;

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
}

/// Convert bounded English cardinal phrases to decimal digits while preserving
/// every byte outside the matched word spans.
pub(crate) fn normalize_spoken_numbers(input: &str) -> String {
    if input.trim().is_empty() || input.len() > MAX_SPOKEN_NUMBER_INPUT_BYTES {
        return input.to_string();
    }

    let words = word_spans(input);
    let mut output = String::with_capacity(input.len());
    let mut word_index = 0;
    let mut copied_through = 0;
    let mut changed = false;

    while word_index < words.len() {
        if should_preserve_prose_one(&words, input, word_index) {
            if words[word_index].lower == "1" {
                output.push_str(&input[copied_through..words[word_index].start]);
                output.push_str("one");
                copied_through = words[word_index].end;
                changed = true;
            }
            word_index += 1;
            continue;
        }

        let parsed = parse_existing_large_integer(&words, input, word_index)
            .or_else(|| parse_number(&words, input, word_index));
        let Some(parsed) = parsed else {
            word_index += 1;
            continue;
        };

        output.push_str(&input[copied_through..words[word_index].start]);
        output.push_str(&parsed.rendered);
        copied_through = words[parsed.end_word - 1].end;
        word_index = parsed.end_word;
        changed = true;
    }

    if !changed {
        return input.to_string();
    }
    output.push_str(&input[copied_through..]);
    output
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
        && (has_integer || point_index == number_start)
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
    })
}

fn parse_existing_large_integer(
    words: &[WordSpan],
    input: &str,
    start: usize,
) -> Option<ParsedNumber> {
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

    // Labels conventionally use digits even though the value is isolated.
    if start > 0
        && connected(words, input, start - 1, start)
        && matches!(
            words[start - 1].lower.as_str(),
            "number"
                | "step"
                | "option"
                | "version"
                | "chapter"
                | "page"
                | "line"
                | "issue"
                | "ticket"
                | "phase"
                | "level"
                | "rank"
                | "build"
                | "model"
        )
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
    separator
        .chars()
        .all(|character| matches!(character, ' ' | '\t' | ',' | ';' | '/'))
        && is_number_token(&other_word.lower)
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

    #[test]
    fn converts_single_and_separated_numbers() {
        assert_eq!(
            normalize_spoken_numbers("one, two, three, four, five, six"),
            "1, 2, 3, 4, 5, 6"
        );
        assert_eq!(normalize_spoken_numbers("zero one two"), "0 1 2");
        assert_eq!(
            normalize_spoken_numbers("I need ten copies"),
            "I need 10 copies"
        );
    }

    #[test]
    fn converts_compound_and_hyphenated_cardinals() {
        assert_eq!(
            normalize_spoken_numbers("The result is eight hundred fifty-seven."),
            "The result is 857."
        );
        assert_eq!(normalize_spoken_numbers("fifty-seven"), "57");
        assert_eq!(normalize_spoken_numbers("twelve hundred thirty"), "1,230");
        assert_eq!(normalize_spoken_numbers("one hundred and five"), "105");
    }

    #[test]
    fn converts_large_descending_scales() {
        assert_eq!(
            normalize_spoken_numbers(
                "ten million one hundred three thousand four hundred forty-five"
            ),
            "10,103,445"
        );
        assert_eq!(
            normalize_spoken_numbers(
                "nine quadrillion eight trillion seven billion six million five thousand four"
            ),
            "9,008,007,006,005,004"
        );
    }

    #[test]
    fn groups_spoken_and_existing_large_integers() {
        assert_eq!(
            normalize_spoken_numbers(
                "thirty-five million four hundred fifty-five thousand thirty-four"
            ),
            "35,455,034"
        );
        assert_eq!(normalize_spoken_numbers("35455034"), "35,455,034");
        assert_eq!(
            normalize_spoken_numbers("one million point five"),
            "1,000,000.5"
        );
        assert_eq!(normalize_spoken_numbers("one thousand"), "1,000");
    }

    #[test]
    fn existing_years_leading_zeroes_and_decimal_fractions_are_not_grouped() {
        assert_eq!(normalize_spoken_numbers("2026"), "2026");
        assert_eq!(normalize_spoken_numbers("01234567"), "01234567");
        assert_eq!(normalize_spoken_numbers("2.12345678"), "2.12345678");
        assert_eq!(normalize_spoken_numbers("123"), "123");
    }

    #[test]
    fn isolated_one_remains_spelled_out_in_prose() {
        assert_eq!(
            normalize_spoken_numbers(
                "one thing, one more thing, one idea, that one, and maybe one day"
            ),
            "one thing, one more thing, one idea, that one, and maybe one day"
        );
        assert_eq!(
            normalize_spoken_numbers("1 thing, 1 idea, that 1, and 1 day"),
            "one thing, one idea, that one, and one day"
        );
        assert_eq!(normalize_spoken_numbers("one hundred things"), "100 things");
    }

    #[test]
    fn explicit_one_sequences_compounds_and_labels_remain_numeric() {
        assert_eq!(normalize_spoken_numbers("one"), "1");
        assert_eq!(normalize_spoken_numbers("one, two, three"), "1, 2, 3");
        assert_eq!(normalize_spoken_numbers("one or two"), "1 or 2");
        assert_eq!(
            normalize_spoken_numbers("number one, step one, and version one"),
            "number 1, step 1, and version 1"
        );
        assert_eq!(
            normalize_spoken_numbers("one hundred, one point five, negative one"),
            "100, 1.5, -1"
        );
        assert_eq!(normalize_spoken_numbers("1,000 and 1.5"), "1,000 and 1.5");
        assert_eq!(
            normalize_spoken_numbers("maybe like one/2"),
            "maybe like 1/2"
        );
    }

    #[test]
    fn converts_articles_negatives_and_decimals() {
        assert_eq!(
            normalize_spoken_numbers("a hundred reasons and a million more"),
            "100 reasons and 1,000,000 more"
        );
        assert_eq!(normalize_spoken_numbers("negative forty-two"), "-42");
        assert_eq!(
            normalize_spoken_numbers("three point one four and point oh five"),
            "3.14 and 0.05"
        );
    }

    #[test]
    fn leaves_non_number_articles_and_existing_digits_alone() {
        let input = "A cat has 9 lives in version 2.0 with one2 one_two and oneé";
        assert_eq!(normalize_spoken_numbers(input), input);
        assert_eq!(normalize_spoken_numbers("one\nhundred"), "1\n100");
    }

    #[test]
    fn malformed_scales_are_not_combined_into_a_guessed_value() {
        assert_eq!(
            normalize_spoken_numbers("one thousand two million"),
            "1,002 1,000,000"
        );
        assert_eq!(
            normalize_spoken_numbers("three hundred four hundred"),
            "304 100"
        );
    }

    #[test]
    fn normalization_is_idempotent() {
        let once = normalize_spoken_numbers(
            "Call negative eight hundred fifty-seven at three point one four.",
        );
        assert_eq!(normalize_spoken_numbers(&once), once);
        let grouped = normalize_spoken_numbers(
            "thirty-five million four hundred fifty-five thousand thirty-four",
        );
        assert_eq!(normalize_spoken_numbers(&grouped), grouped);
    }

    #[test]
    fn over_limit_input_is_preserved() {
        let input = format!("one {}", "word ".repeat(MAX_SPOKEN_NUMBER_INPUT_BYTES));
        assert_eq!(normalize_spoken_numbers(&input), input);
    }
}
