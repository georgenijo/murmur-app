//! Safe recovery explanations for rejected meeting operations.
//!
//! Native error strings can carry OS details, so a presentation never shows
//! them. This maps the service's own rejection, by command, to a static line
//! that says what to change. The wording follows the shipped meeting review
//! (`useMeetings.saveMetadata`, `applyCalendarEvent` and the export surfaces).

/// Shipped `saveMetadata` failure line. The shipped UI shows it for any rejection.
pub const METADATA_REJECTED: &str = "Meeting details could not be saved. Use a title and attendee names of at most 200 characters, with up to 100 attendees.";
/// Shipped `applyCalendarEvent` failure line.
pub const CALENDAR_REJECTED: &str =
    "Calendar details could not be applied. Look up the event again, or name this meeting manually.";
const DESTINATION_UNUSABLE: &str = "That export destination cannot be used. Choose a file name in a folder that exists, then try again.";
const DESTINATION_UNWRITABLE: &str = "The export could not be written to that location. Check that the folder is writable and has free space, then try again.";
const EXPORT_TOO_LARGE: &str =
    "This export is too large to save. Choose another format or a shorter meeting.";
const BUNDLE_EXTENSION: &str =
    "Choose a file ending in .zip for the meeting audio bundle, then try again.";
const BUNDLE_TOO_LARGE: &str = "This meeting's audio is larger than the 2 GiB bundle limit. Export the review document on its own instead.";

fn extension(error: &str) -> Option<&'static str> {
    let ext = error
        .strip_prefix("Choose a .")?
        .strip_suffix(" file for this export format.")?;
    Some(match ext {
        "md" => "Choose a file ending in .md for the Markdown export, then try again.",
        "txt" => "Choose a file ending in .txt for the plain text export, then try again.",
        "json" => "Choose a file ending in .json for the JSON export, then try again.",
        "srt" => "Choose a file ending in .srt for the SRT captions export, then try again.",
        "vtt" => "Choose a file ending in .vtt for the WebVTT captions export, then try again.",
        _ => DESTINATION_UNUSABLE,
    })
}

/// The explanation for `command` rejecting with `error`, or `None` when the
/// operation has no specific recovery wording and callers keep their generic one.
pub fn explain(command: &str, error: &str) -> Option<&'static str> {
    match command {
        "save_meeting_metadata" => Some(METADATA_REJECTED),
        "apply_meeting_calendar_event" => Some(CALENDAR_REJECTED),
        "save_meeting_review_export"
        | "save_meeting_bundle_export"
        | "get_meeting_bundle_preview" => {
            if let Some(line) = extension(error) {
                return Some(line);
            }
            if error.starts_with("Unsupported export type") {
                return Some(if command == "save_meeting_review_export" {
                    DESTINATION_UNUSABLE
                } else {
                    BUNDLE_EXTENSION
                });
            }
            if error.starts_with("Export is too large") {
                return Some(EXPORT_TOO_LARGE);
            }
            if error.starts_with("Export path ")
                || error.starts_with("Export file name ")
                || error.starts_with("Export folder ")
            {
                return Some(DESTINATION_UNUSABLE);
            }
            if error.starts_with("Failed to write export")
                || error.starts_with("Failed to publish export")
                || error == crate::meeting_bundle::WRITE_FAILED
            {
                return Some(DESTINATION_UNWRITABLE);
            }
            // The bundle owner's own content-free lines are already actionable.
            [
                crate::meeting_bundle::NOTHING_TO_BUNDLE,
                crate::meeting_bundle::BUNDLE_REVOKED,
                crate::meeting_bundle::BUNDLE_BUSY,
                crate::meeting_audio::BUNDLE_CAPTURE_BUSY,
            ]
            .into_iter()
            .find(|line| *line == error)
            .or_else(|| {
                error
                    .starts_with("This meeting's audio is larger than")
                    .then_some(BUNDLE_TOO_LARGE)
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        commands::export::{validate_bundle_path, write_text_export},
        meeting_review::MeetingReviewExportFormat as Format,
        meeting_store::{MeetingRepository, SaveMeetingMetadataRequest},
    };
    use std::path::Path;

    const REVIEW: &str = "save_meeting_review_export";
    const BUNDLE: &str = "save_meeting_bundle_export";

    fn repository() -> (tempfile::TempDir, MeetingRepository) {
        let temp = tempfile::TempDir::new().unwrap();
        let (repository, _) =
            MeetingRepository::initialize(temp.path().canonicalize().unwrap()).unwrap();
        (temp, repository)
    }

    // The real repository rejections, one per limit the shipped line names.
    #[test]
    fn rejected_metadata_is_explained_for_each_real_limit() {
        let (_temp, repository) = repository();
        repository
            .create_session("metadata-session", "base.en", "en", true, false)
            .unwrap();
        for (title, attendees) in [
            (Some("x".repeat(10_000)), Vec::new()),
            (None, vec!["x".repeat(10_000)]),
            (None, vec!["attendee".to_string(); 10_000]),
        ] {
            let error = repository
                .save_metadata(SaveMeetingMetadataRequest {
                    session_id: "metadata-session".into(),
                    title,
                    attendees,
                })
                .unwrap_err();
            assert_eq!(
                explain("save_meeting_metadata", &error),
                Some(METADATA_REJECTED),
                "{error}"
            );
        }
        assert!(METADATA_REJECTED.contains("200 characters") && METADATA_REJECTED.contains("100"));
    }

    #[test]
    fn calendar_rejections_keep_the_shipped_line() {
        for error in [
            crate::calendar::STALE_SELECTION,
            crate::calendar::ACCESS_REQUIRED,
        ] {
            assert_eq!(
                explain("apply_meeting_calendar_event", error),
                Some(CALENDAR_REJECTED)
            );
        }
    }

    #[test]
    fn a_wrong_review_export_extension_names_the_required_one() {
        for (format, name) in [
            (Format::Markdown, "Markdown"),
            (Format::Text, "plain text"),
            (Format::Json, "JSON"),
            (Format::Srt, "SRT"),
            (Format::Vtt, "WebVTT"),
        ] {
            // The exact message `save_meeting_review_export` returns.
            let error = format!(
                "Choose a .{} file for this export format.",
                format.extension()
            );
            let line = explain(REVIEW, &error).unwrap();
            assert!(line.contains(&format!(".{}", format.extension())), "{line}");
            assert!(line.contains(name), "{line}");
        }
    }

    #[test]
    fn real_destination_rejections_are_explained_without_the_native_text() {
        let dir = tempfile::TempDir::new().unwrap();
        let missing = dir.path().join("absent").join("meeting.md");
        let errors = [
            write_text_export(Path::new("relative.md"), "x").unwrap_err(),
            write_text_export(dir.path(), "x").unwrap_err(),
            write_text_export(&missing, "x").unwrap_err(),
            write_text_export(&dir.path().join(".hidden.md"), "x").unwrap_err(),
            write_text_export(&dir.path().join("noextension"), "x").unwrap_err(),
            write_text_export(&dir.path().join("big.md"), &"x".repeat(9 * 1024 * 1024))
                .unwrap_err(),
        ];
        for error in errors {
            let line = explain(REVIEW, &error).unwrap_or_else(|| panic!("unexplained: {error}"));
            assert!(
                !line.contains(dir.path().to_str().unwrap()),
                "{line} leaks the destination"
            );
        }
        assert_eq!(
            explain(REVIEW, &write_text_export(dir.path(), "x").unwrap_err()),
            Some(DESTINATION_UNUSABLE)
        );
        assert_eq!(
            explain(
                REVIEW,
                &write_text_export(&dir.path().join("big.md"), &"x".repeat(9 * 1024 * 1024))
                    .unwrap_err()
            ),
            Some(EXPORT_TOO_LARGE)
        );
    }

    #[test]
    #[cfg(unix)]
    fn a_failed_destination_write_is_explained_as_a_destination_failure() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = write_text_export(&dir.path().join("meeting.md"), "x");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        // A user that can write anyway (root) never reaches the failure.
        if let Err(error) = result {
            assert_eq!(
                explain(REVIEW, &error),
                Some(DESTINATION_UNWRITABLE),
                "{error}"
            );
        }
        assert_eq!(
            explain(BUNDLE, crate::meeting_bundle::WRITE_FAILED),
            Some(DESTINATION_UNWRITABLE)
        );
    }

    #[test]
    fn a_bundle_destination_without_zip_is_explained() {
        let dir = tempfile::TempDir::new().unwrap();
        let error = validate_bundle_path(&dir.path().join("meeting.md")).unwrap_err();
        assert_eq!(explain(BUNDLE, &error), Some(BUNDLE_EXTENSION));
    }

    #[test]
    fn the_bundle_owners_own_lines_pass_through() {
        for line in [
            crate::meeting_bundle::NOTHING_TO_BUNDLE,
            crate::meeting_bundle::BUNDLE_REVOKED,
            crate::meeting_bundle::BUNDLE_BUSY,
            crate::meeting_audio::BUNDLE_CAPTURE_BUSY,
        ] {
            assert_eq!(explain(BUNDLE, line), Some(line));
            assert_eq!(explain("get_meeting_bundle_preview", line), Some(line));
        }
        assert_eq!(
            explain(BUNDLE, &crate::meeting_bundle::too_large()),
            Some(BUNDLE_TOO_LARGE)
        );
        assert_eq!(BUNDLE_TOO_LARGE, crate::meeting_bundle::too_large());
    }

    #[test]
    fn unrelated_commands_and_unknown_errors_stay_generic() {
        assert_eq!(explain("delete_meeting", "anything"), None);
        assert_eq!(
            explain(REVIEW, "The local meeting transcript store is unavailable."),
            None
        );
        assert_eq!(
            explain(REVIEW, "/Users/someone/private.md could not be opened"),
            None
        );
    }
}
