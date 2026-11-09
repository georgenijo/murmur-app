//! Content-free explanations for model download and removal failures.
//!
//! Native install and removal errors can embed OS error text. The presentation
//! layer never shows that text; it shows one of these static explanations,
//! which keep the shipped remediation wording ("Retry to resume", "Remove that
//! copy manually") for every failure the install transaction can report.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Download,
    Remove,
}

/// Service messages that already contain only static, user-actionable text.
const STATIC_MESSAGES: &[&str] = &[
    "This model is already installing. Cancel it or wait for it to finish.",
    "Wait for installer cleanup, then retry Cancel or Download.",
    "This model is not supported on the current platform",
    "Core ML transcription requires Apple Silicon macOS",
    "Wait for this model's download or removal to finish",
    "Wait for model preparation or transcription to finish",
    "Wait for this model's download to finish",
    "Wait for the current recording transition to finish",
    "Wait for the current dictation operation to finish",
    "Wait for the current model operation to finish before removing a model",
    "Wait for the previous Core ML installer to exit before removing this model",
    "Choose another model before removing the selected or active model",
    "Stop recording and wait for transcription to finish before removing a model",
    "This model is not installed",
    "Unexpected model artifact type; nothing was removed",
    "This model has a copy outside Murmur's model storage. Remove that copy manually before using Remove here.",
    "This model is outside Murmur's model storage. Remove that copy manually.",
    "The shared Core ML cache cannot be removed by an isolated preview.",
];

/// Prefix rules, checked in order. A prefix covers messages whose tail carries
/// native text (an OS error, an HTTP status or a byte count).
const DOWNLOAD_PREFIXES: &[(&str, &str)] = &[
    (
        "Not enough disk space",
        "Not enough free disk space to install this model. Free some space, then retry the download.",
    ),
    (
        "Could not check free disk space",
        "Murmur could not check free disk space for model storage. Free some space, then retry the download.",
    ),
    (
        "Could not locate the model storage volume",
        "Murmur could not check free disk space for model storage. Free some space, then retry the download.",
    ),
    (
        "Download stalled",
        "Download stalled: no data received. Retry to resume.",
    ),
    (
        "Download request failed",
        "Download interrupted. Check your internet connection, then retry to resume.",
    ),
    (
        "Download interrupted",
        "Download interrupted. Check your internet connection, then retry to resume.",
    ),
    (
        "Download ended early",
        "Download interrupted. Check your internet connection, then retry to resume.",
    ),
    (
        "Failed to create HTTP client",
        "Download could not start. Check your internet connection, then retry the download.",
    ),
    (
        "Download failed with status",
        "The model server could not provide this download. Retry later, or choose another model.",
    ),
    (
        "Model server returned",
        "The model server could not provide this download. Retry later, or choose another model.",
    ),
    (
        "Model installation completed but validation failed",
        "The download finished, but the model did not validate. Retry the download to repair it.",
    ),
    (
        "Core ML setup stopped after reaching its time limit",
        "Core ML setup stopped after reaching its time limit. Retry Core ML, or choose Whisper Base or the CPU Parakeet fallback.",
    ),
    (
        "Core ML setup stopped, but Murmur could not confirm installer cleanup",
        "Core ML setup stopped, but Murmur could not confirm installer cleanup. Retry Core ML, or choose Whisper Base or the CPU Parakeet fallback.",
    ),
    (
        "Failed to ",
        "Murmur could not write the model to its storage folder. Check free disk space and folder permissions, then retry the download.",
    ),
];

const DOWNLOAD_COREML: &str = "Core ML setup did not finish. Retry Core ML, or choose Whisper Base or the CPU Parakeet fallback.";
const DOWNLOAD_GENERIC: &str =
    "Model download did not finish. Retry the download, or choose another model.";

const REMOVE_PREFIXES: &[(&str, &str)] = &[
    (
        "Model removal does not follow symbolic links",
        "Model removal does not follow symbolic links. Remove the model's files manually from Murmur's model storage.",
    ),
    (
        "Could not remove model",
        "Could not remove the model files. Make sure nothing is using them, then try again or remove them manually from Murmur's model storage.",
    ),
    (
        "Could not inspect model artifact",
        "Could not remove the model files. Make sure nothing is using them, then try again or remove them manually from Murmur's model storage.",
    ),
    (
        "Could not locate the Core ML model cache",
        "Could not locate the Core ML model cache. Remove it manually if it still exists.",
    ),
];
const REMOVE_GENERIC: &str = "Could not remove this model. Try again, or remove its files manually from Murmur's model storage.";

/// The static explanation for a failed operation, or `None` when the error is
/// an owner-requested cancellation rather than a failure.
pub fn explain(operation: Operation, error: &str) -> Option<&'static str> {
    if error.starts_with(crate::model_download::CANCELLED) {
        return None;
    }
    if let Some(message) = STATIC_MESSAGES.iter().find(|m| **m == error).copied() {
        return Some(message);
    }
    let (prefixes, generic) = match operation {
        Operation::Download => (DOWNLOAD_PREFIXES, DOWNLOAD_GENERIC),
        Operation::Remove => (REMOVE_PREFIXES, REMOVE_GENERIC),
    };
    if let Some((_, message)) = prefixes
        .iter()
        .find(|(prefix, _)| error.starts_with(*prefix))
    {
        return Some(*message);
    }
    if operation == Operation::Download
        && (error.contains("Retry Core ML") || error.starts_with("Core ML"))
    {
        return Some(DOWNLOAD_COREML);
    }
    Some(generic)
}

/// Whether a failed download of this model offers the explicit fallback choices
/// the shipped downloader shows for Core ML.
pub fn offers_fallbacks(model: &str) -> bool {
    model == crate::transcriber::COREML_MODEL_NAME
}

/// Test-only fault injection, compiled in only for `native-host-qa` builds.
/// `MURMUR_QA_MODEL_FAULT=<download|remove>:<kind>` makes that operation fail
/// once with the real service error string for `<kind>`, so Retry afterwards
/// runs the real operation; unset, it does nothing.
#[cfg(feature = "native-host-qa")]
pub(crate) fn injected(operation: Operation) -> Option<String> {
    static FIRED: [std::sync::atomic::AtomicBool; 2] = [
        std::sync::atomic::AtomicBool::new(false),
        std::sync::atomic::AtomicBool::new(false),
    ];
    let value = std::env::var("MURMUR_QA_MODEL_FAULT").ok()?;
    let (op, kind) = value.split_once(':')?;
    let message = match (operation, op, kind) {
        (Operation::Download, "download", "network") => {
            "Download interrupted. Retry to resume: synthetic"
        }
        (Operation::Download, "download", "disk") => {
            "Not enough disk space: need 2000 MB, available 1 MB. Free space and retry."
        }
        (Operation::Download, "download", "validation") => {
            "Model installation completed but validation failed"
        }
        (Operation::Download, "download", "coreml") => {
            "Core ML model setup failed. Retry Core ML, or choose Whisper Base or the CPU Parakeet fallback."
        }
        (Operation::Remove, "remove", "outside-copy") => {
            "This model has a copy outside Murmur's model storage. Remove that copy manually before using Remove here."
        }
        (Operation::Remove, "remove", "io") => "Could not remove model: synthetic",
        _ => return None,
    };
    let fired = &FIRED[usize::from(operation == Operation::Remove)];
    (!fired.swap(true, std::sync::atomic::Ordering::SeqCst)).then(|| message.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_download_failures_get_static_remediation() {
        let disk = crate::model_download::check_available(2_000_000_000, 1_000_000).unwrap_err();
        assert!(explain(Operation::Download, &disk)
            .unwrap()
            .contains("Free some space"));
        for error in [
            "Download stalled: no data received. Retry to resume.",
            "Download interrupted. Retry to resume: connection reset",
            "Download request failed (retry to resume): dns error",
            "Download ended early: received 1 of 2 bytes. Retry to resume.",
        ] {
            let message = explain(Operation::Download, error).unwrap();
            assert!(
                message.to_lowercase().contains("retry to resume"),
                "{error}"
            );
        }
        assert!(explain(
            Operation::Download,
            "Model installation completed but validation failed"
        )
        .unwrap()
        .contains("did not validate"));
        assert!(explain(
            Operation::Download,
            "Failed to write model: No space left on device (os error 28)"
        )
        .unwrap()
        .contains("storage folder"));
    }

    #[test]
    fn core_ml_failures_keep_fallback_choices() {
        let failure = crate::coreml_installer::InstallFailure {
            code: "native_initialization_failed",
            repaired_cache: false,
            repeated_repair: false,
            termination_confirmed: true,
        };
        let message = explain(Operation::Download, &failure.user_message()).unwrap();
        assert!(message.contains("Whisper Base"));
        assert!(message.contains("CPU Parakeet"));
        let timeout = crate::coreml_installer::InstallFailure {
            code: "installer_timeout",
            ..failure
        };
        assert!(explain(Operation::Download, &timeout.user_message())
            .unwrap()
            .contains("time limit"));
        assert!(offers_fallbacks(crate::transcriber::COREML_MODEL_NAME));
        assert!(!offers_fallbacks("base.en"));
    }

    #[test]
    fn removal_failures_keep_remove_manually_guidance() {
        for error in [
            "This model has a copy outside Murmur's model storage. Remove that copy manually before using Remove here.",
            "This model is outside Murmur's model storage. Remove that copy manually.",
        ] {
            assert_eq!(explain(Operation::Remove, error), Some(error));
        }
        for error in [
            "Could not remove model: Permission denied (os error 13)",
            "Could not inspect model artifact: No such file or directory (os error 2)",
        ] {
            assert!(
                explain(Operation::Remove, error)
                    .unwrap()
                    .contains("manually"),
                "{error}"
            );
        }
        assert_eq!(
            explain(Operation::Remove, "private path /Users/x/model"),
            Some(REMOVE_GENERIC)
        );
    }

    #[test]
    fn native_text_never_reaches_the_explanation_and_cancel_is_not_a_failure() {
        assert_eq!(
            explain(
                Operation::Download,
                "Failed to write model: /Users/private/secret.bin"
            )
            .map(|m| m.contains("/Users")),
            Some(false)
        );
        assert_eq!(
            explain(Operation::Download, crate::model_download::CANCELLED),
            None
        );
    }
}
