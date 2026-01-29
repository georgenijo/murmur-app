"""
Accuracy measurement utilities using Word Error Rate (WER).

WER = (Substitutions + Deletions + Insertions) / Total Reference Words

Accuracy = 1 - WER (clamped to 0-100%)
"""

from jiwer import wer, cer


def calculate_accuracy(reference: str, hypothesis: str) -> dict:
    """
    Calculate accuracy metrics between reference and hypothesis.

    Args:
        reference: The ground truth transcript
        hypothesis: The transcription to evaluate

    Returns:
        dict with wer, cer, and accuracy percentage
    """
    if not reference or not hypothesis:
        return {
            "wer": 1.0,
            "cer": 1.0,
            "accuracy": 0.0,
            "reference": reference,
            "hypothesis": hypothesis,
        }

    # Normalize: lowercase and strip
    ref_clean = reference.lower().strip()
    hyp_clean = hypothesis.lower().strip()

    word_error_rate = wer(ref_clean, hyp_clean)
    char_error_rate = cer(ref_clean, hyp_clean)

    # Accuracy as percentage (clamped to 0-100)
    accuracy = max(0, (1 - word_error_rate)) * 100

    return {
        "wer": word_error_rate,
        "cer": char_error_rate,
        "accuracy": accuracy,
        "reference": ref_clean,
        "hypothesis": hyp_clean,
    }


def format_accuracy(metrics: dict) -> str:
    """Format accuracy metrics for display."""
    return f"WER: {metrics['wer']:.1%} | Accuracy: {metrics['accuracy']:.1f}%"
