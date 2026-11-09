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

/// Shared with the install transaction so the producer and the classifier
/// cannot drift apart.
pub(crate) const INSTALL_VALIDATION_FAILED: &str =
    "Model installation completed but validation failed";
pub(crate) const PARAKEET_BUNDLE_INCOMPLETE: &str = "Extracted Parakeet bundle is incomplete";
pub(crate) const COREML_SUPERVISION_FAILED: &str = "Core ML installer supervision failed. Retry Core ML, or choose Whisper Base or the CPU Parakeet fallback.";

macro_rules! coreml_suffix {
    () => {
        " Retry Core ML, or choose Whisper Base or the CPU Parakeet fallback."
    };
}
macro_rules! coreml_repeated {
    () => {
        " This incomplete cache has already required repair on an earlier attempt."
    };
}
macro_rules! coreml_messages {
    ($($detail:literal),* $(,)?) => {
        &[
            COREML_SUPERVISION_FAILED,
            $(
                concat!($detail, coreml_suffix!()),
                concat!($detail, coreml_repeated!(), coreml_suffix!()),
            )*
        ]
    };
}

/// Every message `coreml_installer::InstallFailure::user_message` can produce.
/// These carry no native text, so each is shown exactly, which keeps the
/// distinct timeout, cache-repair, repeated-repair, validation and cleanup
/// explanations. A producer-backed test keeps this table in lock-step.
const COREML_MESSAGES: &[&str] = coreml_messages![
    "Core ML setup stopped after reaching its time limit. No installer was left running.",
    "Murmur could not safely repair the incomplete Core ML cache.",
    "Murmur could not safely track the incomplete Core ML cache repair.",
    "Core ML setup finished, but the installed cache did not validate.",
    "Core ML setup stopped, but Murmur could not confirm installer cleanup. Murmur will recheck cleanup before another Core ML attempt.",
    "Core ML model setup failed in the native installer.",
    "The Core ML model cache is unavailable.",
    "Core ML model setup failed.",
];

/// Service messages that already contain only static, user-actionable text.
const STATIC_MESSAGES: &[&str] = &[
    PARAKEET_BUNDLE_INCOMPLETE,
    "Parakeet archive exceeds its staging space budget",
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
        "Invalid Parakeet archive",
        "The downloaded Parakeet archive was invalid or corrupt. Retry the download to fetch a fresh copy.",
    ),
    (
        "Failed to extract archive",
        "Murmur could not extract the model archive. Check free disk space and folder permissions, then retry the download.",
    ),
    (
        "Extraction task failed",
        "Murmur could not extract the model archive. Check free disk space and folder permissions, then retry the download.",
    ),
    (
        "Unknown Parakeet model",
        "Murmur does not know how to install this Parakeet model.",
    ),
    (
        "Failed to ",
        "Murmur could not write the model to its storage folder. Check free disk space and folder permissions, then retry the download.",
    ),
];

const DOWNLOAD_WHISPER_INVALID: &str = "The downloaded Whisper model is incomplete or has an invalid GGML header. Retry the download to repair it.";
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
    if let Some(message) = STATIC_MESSAGES
        .iter()
        .chain(COREML_MESSAGES)
        .find(|m| **m == error)
        .copied()
    {
        return Some(message);
    }
    if operation == Operation::Download
        && error.starts_with("Whisper model '")
        && error.ends_with("' is incomplete or has an invalid GGML header")
    {
        return Some(DOWNLOAD_WHISPER_INVALID);
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

/// The static explanation `text` is, when it already is one. A presentation
/// that receives an error already passed through `explain` uses this to keep it
/// without ever accepting unexplained native text.
pub fn known_explanation(text: &str) -> Option<&'static str> {
    STATIC_MESSAGES
        .iter()
        .chain(COREML_MESSAGES)
        .copied()
        .chain(
            DOWNLOAD_PREFIXES
                .iter()
                .chain(REMOVE_PREFIXES)
                .map(|(_, message)| *message),
        )
        .chain([
            DOWNLOAD_GENERIC,
            DOWNLOAD_COREML,
            DOWNLOAD_WHISPER_INVALID,
            REMOVE_GENERIC,
        ])
        .find(|message| *message == text)
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
    let coreml = |code, repeated_repair| {
        crate::coreml_installer::InstallFailure {
            code,
            repaired_cache: false,
            repeated_repair,
            termination_confirmed: true,
        }
        .user_message()
    };
    let message = match (operation, op, kind) {
        (Operation::Download, "download", "network") => {
            "Download interrupted. Retry to resume: synthetic".to_string()
        }
        (Operation::Download, "download", "disk") => {
            "Not enough disk space: need 2000 MB, available 1 MB. Free space and retry."
                .to_string()
        }
        (Operation::Download, "download", "validation") => INSTALL_VALIDATION_FAILED.to_string(),
        // The real validator's error for a file that is not a Whisper model.
        (Operation::Download, "download", "whisper-validation") => {
            crate::model_validation::validate_whisper(std::path::Path::new(""), "base.en")
                .unwrap_err()
        }
        (Operation::Download, "download", "archive-incomplete") => {
            PARAKEET_BUNDLE_INCOMPLETE.to_string()
        }
        (Operation::Download, "download", "archive-corrupt") => {
            "Invalid Parakeet archive: synthetic".to_string()
        }
        (Operation::Download, "download", "coreml") => {
            coreml("native_initialization_failed", false)
        }
        (Operation::Download, "download", "coreml-timeout") => coreml("installer_timeout", false),
        (Operation::Download, "download", "coreml-validation") => {
            coreml("validation_failed", false)
        }
        (Operation::Download, "download", "coreml-repair") => {
            coreml("cache_repair_failed", true)
        }
        (Operation::Remove, "remove", "outside-copy") => {
            "This model has a copy outside Murmur's model storage. Remove that copy manually before using Remove here.".to_string()
        }
        (Operation::Remove, "remove", "io") => "Could not remove model: synthetic".to_string(),
        _ => return None,
    };
    let fired = &FIRED[usize::from(operation == Operation::Remove)];
    (!fired.swap(true, std::sync::atomic::Ordering::SeqCst)).then_some(message)
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
    fn every_core_ml_failure_keeps_its_own_explanation() {
        let mut seen = std::collections::BTreeSet::new();
        for code in [
            "installer_timeout",
            "cache_repair_failed",
            "repair_state_unavailable",
            "validation_failed",
            "termination_unconfirmed",
            "native_initialization_failed",
            "cache_unavailable",
            "spawn_failed",
            "worker_exited_early",
            "protocol_error",
        ] {
            for repeated_repair in [false, true] {
                let produced = crate::coreml_installer::InstallFailure {
                    code,
                    repaired_cache: repeated_repair,
                    repeated_repair,
                    termination_confirmed: true,
                }
                .user_message();
                // Shown exactly as the service wrote it: nothing is collapsed.
                assert_eq!(
                    explain(Operation::Download, &produced),
                    Some(produced.as_str()),
                    "{code} {repeated_repair}"
                );
                seen.insert(produced);
            }
        }
        assert!(
            seen.len() >= 14,
            "distinct explanations collapsed: {}",
            seen.len()
        );
        assert_eq!(
            explain(Operation::Download, COREML_SUPERVISION_FAILED),
            Some(COREML_SUPERVISION_FAILED)
        );
        let repeated = crate::coreml_installer::InstallFailure {
            code: "cache_repair_failed",
            repaired_cache: true,
            repeated_repair: true,
            termination_confirmed: true,
        }
        .user_message();
        assert!(explain(Operation::Download, &repeated)
            .unwrap()
            .contains("already required repair"));
    }

    #[test]
    fn real_whisper_validation_failure_is_not_the_generic_download_message() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("model.bin");
        std::fs::write(&path, b"synthetic, not a ggml file").unwrap();
        for target in [path.as_path(), std::path::Path::new("")] {
            let error = crate::model_validation::validate_whisper(target, "base.en").unwrap_err();
            let message = explain(Operation::Download, &error).unwrap();
            assert_eq!(message, DOWNLOAD_WHISPER_INVALID);
            assert_ne!(message, DOWNLOAD_GENERIC);
            assert!(message.contains("GGML header"));
        }
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
