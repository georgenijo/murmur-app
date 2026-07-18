use super::WHISPER_SAMPLE_RATE;

pub const WINDOW_SAMPLES: usize = 10 * WHISPER_SAMPLE_RATE as usize;
pub const STEP_SAMPLES: usize = 8 * WHISPER_SAMPLE_RATE as usize;
pub const OVERLAP_SAMPLES: usize = WINDOW_SAMPLES - STEP_SAMPLES;

fn normalized_word(word: &str) -> String {
    word.chars()
        .filter(|character| character.is_alphanumeric() || *character == '\'')
        .flat_map(char::to_lowercase)
        .collect()
}

fn edit_distance(left: &[String], right: &[String]) -> usize {
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left_word) in left.iter().enumerate() {
        let mut current = vec![row + 1; right.len() + 1];
        for (column, right_word) in right.iter().enumerate() {
            current[column + 1] = (previous[column + 1] + 1)
                .min(current[column] + 1)
                .min(previous[column] + usize::from(left_word != right_word));
        }
        previous = current;
    }
    previous[right.len()]
}

fn boundary_words_match(left: &str, right: &str) -> bool {
    left == right
        || (left.starts_with(right) && left.len().saturating_sub(right.len()) <= 2)
        || (right.starts_with(left) && right.len().saturating_sub(left.len()) <= 2)
}

/// Deterministically reconcile a transcript boundary. The largest near-equal
/// suffix/prefix pair within 12 words is emitted once. Earlier stable words are
/// retained, while a later boundary word may complete a truncated token or
/// remove punctuation that Whisper invented at the end of an audio window.
pub struct ReconciledText {
    pub text: String,
    pub overlap_words: usize,
}

pub fn reconcile_overlapping_text(existing: &str, next: &str) -> ReconciledText {
    let left: Vec<&str> = existing.split_whitespace().collect();
    let right: Vec<&str> = next.split_whitespace().collect();
    if left.is_empty() {
        return ReconciledText {
            text: next.trim().to_string(),
            overlap_words: 0,
        };
    }
    if right.is_empty() {
        return ReconciledText {
            text: existing.trim().to_string(),
            overlap_words: 0,
        };
    }

    let left_norm: Vec<String> = left.iter().map(|word| normalized_word(word)).collect();
    let right_norm: Vec<String> = right.iter().map(|word| normalized_word(word)).collect();
    let max_overlap = left.len().min(right.len()).min(12);
    let mut best: Option<(usize, usize)> = None;
    for count in 1..=max_overlap {
        // Anchor the candidate's right edge so a longer fuzzy match cannot
        // consume the first genuinely new word after the overlap.
        if !right_norm
            .get(count - 1)
            .map(|right_word| boundary_words_match(left_norm.last().unwrap(), right_word))
            .unwrap_or(false)
        {
            continue;
        }
        let distance = edit_distance(&left_norm[left_norm.len() - count..], &right_norm[..count]);
        let acceptable = if count == 1 {
            distance == 0
        } else {
            distance.saturating_mul(3) <= count
        };
        if !acceptable {
            continue;
        }
        let candidate = (count, usize::MAX - distance);
        if best.map(|current| candidate > current).unwrap_or(true) {
            best = Some(candidate);
        }
    }

    let overlap = best.map(|(count, _)| count).unwrap_or(0);
    if overlap == 0 {
        return ReconciledText {
            text: format!("{} {}", existing.trim(), next.trim()),
            overlap_words: 0,
        };
    }

    let stable_len = left.len() - overlap;
    let mut words: Vec<String> = left[..stable_len]
        .iter()
        .map(|word| (*word).to_string())
        .collect();
    for index in 0..overlap {
        let earlier = left[stable_len + index];
        let later = right[index];
        let earlier_norm = &left_norm[stable_len + index];
        let later_norm = &right_norm[index];
        let earlier_terminal = earlier.ends_with(['.', '!', '?']);
        let later_terminal = later.ends_with(['.', '!', '?']);
        let choose_later = (earlier_norm == later_norm && earlier_terminal != later_terminal)
            || (boundary_words_match(earlier_norm, later_norm)
                && later_norm.len() > earlier_norm.len());
        words.push(if choose_later { later } else { earlier }.to_string());
    }
    words.extend(right[overlap..].iter().map(|word| (*word).to_string()));
    ReconciledText {
        text: words.join(" "),
        overlap_words: overlap,
    }
}

pub fn merge_overlapping_text(existing: &str, next: &str) -> String {
    reconcile_overlapping_text(existing, next).text
}

#[cfg(test)]
mod tests {
    use super::{merge_overlapping_text, reconcile_overlapping_text};

    #[test]
    fn exact_overlap_is_emitted_once() {
        assert_eq!(
            merge_overlapping_text("one two three four", "three four five six"),
            "one two three four five six"
        );
    }

    #[test]
    fn punctuation_and_case_do_not_duplicate_boundary() {
        assert_eq!(
            merge_overlapping_text("Hello, WORLD.", "world this continues"),
            "Hello, world this continues"
        );
    }

    #[test]
    fn one_word_difference_in_long_overlap_is_reconciled() {
        assert_eq!(
            merge_overlapping_text(
                "alpha the voice activity detector before being handed",
                "the voice activity detectors before being handed to whisper"
            ),
            "alpha the voice activity detectors before being handed to whisper"
        );
    }

    #[test]
    fn unrelated_chunks_are_appended() {
        assert_eq!(
            merge_overlapping_text("one two", "three four"),
            "one two three four"
        );
        assert_eq!(
            reconcile_overlapping_text("one two", "three four").overlap_words,
            0
        );
    }

    #[test]
    fn empty_side_is_stable() {
        assert_eq!(merge_overlapping_text("", " next words "), "next words");
        assert_eq!(
            merge_overlapping_text("existing words", ""),
            "existing words"
        );
    }

    #[test]
    fn repeated_phrase_chooses_largest_boundary_match() {
        assert_eq!(
            merge_overlapping_text("go now then go now", "then go now and stop"),
            "go now then go now and stop"
        );
    }

    #[test]
    fn fuzzy_match_does_not_consume_first_new_word() {
        assert_eq!(
            merge_overlapping_text(
                "a double tap mode for dictating code.",
                "a double tap mode for dictating code comments and chat"
            ),
            "a double tap mode for dictating code comments and chat"
        );
    }
}
