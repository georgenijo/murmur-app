//! Failure explanations a file import or export may show verbatim.
//!
//! Services report failures as strings that may carry paths, ids or record
//! content. Only text on these allow lists, all of it static and content-free,
//! reaches the presentation as `Problem::Operation`; anything else stays the
//! generic `Problem::Unavailable`.
use crate::workspace::Problem;

/// Knowledge store explanations the shipped app shows for an import, an export
/// and a bulk change. Each is a literal in the knowledge repository; a test
/// keeps the two in step.
pub const KNOWLEDGE_STORE: [&str; 21] = [
    "Murmur could not read the selected knowledge import.",
    "Knowledge imports are limited to 8 MiB.",
    "The selected file is not a valid Murmur knowledge export.",
    "The selected knowledge export format is not supported by this Murmur build.",
    "Knowledge imports support at most 10,000 records.",
    "The import contains duplicate record IDs.",
    "The import contains an ID that conflicts with different local knowledge.",
    "The import would exceed the 10,000-record knowledge limit.",
    "Knowledge changed after import review",
    "Knowledge changed during import review",
    "Murmur could not write the knowledge export.",
    "Murmur could not publish the knowledge export.",
    "Knowledge export could not be serialized.",
    "Choose a valid export destination.",
    "Murmur could not access the local personal knowledge store.",
    "This knowledge record changed in another window. Refresh and try again.",
    "Knowledge record ID is invalid.",
    "Knowledge triggers and written terms must be between 1 and 256 characters.",
    "Replacement text must be between 1 and 4,096 characters.",
    "Snippet bodies must be between 1 and 65,536 characters.",
    "Voice commands support global or per-app scope.",
];

fn known() -> impl Iterator<Item = &'static str> {
    crate::modes::exchange::reason::FAILURES
        .into_iter()
        .chain(KNOWLEDGE_STORE)
        .chain(super::knowledge::ADAPTER)
        .chain(
            [
                crate::themes::import::reason::IMPORT_PARSE,
                crate::themes::import::reason::IMPORT_READ,
                crate::themes::import::reason::EXPORT_WRITE,
            ]
            .into_iter()
            .flatten()
            .copied(),
        )
}

/// The allow-listed explanation equal to `message`, if there is one.
pub fn operation(message: &str) -> Option<&'static str> {
    known().find(|known| *known == message)
}

/// The presentation problem for a service failure string.
pub fn problem(message: &str) -> Problem {
    operation(message).map_or(Problem::Unavailable, Problem::Operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modes::exchange::reason;

    #[test]
    fn allow_listed_text_is_preserved_and_everything_else_is_generic() {
        assert_eq!(problem(reason::STALE), Problem::Operation(reason::STALE));
        for private in [
            "Failed to write /Users/someone/private/murmur-modes.json: denied",
            "Source writer unavailable",
            "",
        ] {
            assert_eq!(problem(private), Problem::Unavailable);
        }
    }

    #[test]
    fn theme_explanations_are_the_service_literals_with_the_shipped_prefix() {
        use crate::themes::import::reason::*;
        let service = include_str!("../../../../crates/murmur-services/src/commands/theme.rs");
        for (prefix, lines) in [(IMPORT_PREFIX, IMPORT_READ), (EXPORT_PREFIX, EXPORT_WRITE)] {
            for line in lines {
                let text = line.strip_prefix(prefix).unwrap();
                assert!(
                    service.contains(&format!("\"{text}\"")),
                    "not a theme service literal: {text}"
                );
                assert_eq!(problem(line), Problem::Operation(line));
            }
        }
        for line in IMPORT_PARSE {
            assert_eq!(problem(line), Problem::Operation(line));
        }
        // The bare reason, as a service reports it, is not the shown line,
        // and a failure that names a path is never shown.
        assert_eq!(problem(UNREADABLE), Problem::Unavailable);
        assert_eq!(
            problem("Failed to import theme: /Users/synthetic/theme.json"),
            Problem::Unavailable
        );
    }

    #[test]
    fn knowledge_explanations_are_the_repository_literals() {
        let repository =
            include_str!("../../../../crates/murmur-services/src/knowledge_store/repository.rs");
        for text in KNOWLEDGE_STORE {
            assert!(
                repository.contains(&format!("\"{text}\"")),
                "not a repository literal: {text}"
            );
            assert_eq!(problem(text), Problem::Operation(text));
        }
        for text in super::super::knowledge::ADAPTER {
            assert_eq!(problem(text), Problem::Operation(text));
        }
        // A schema failure carries a version number, a database failure
        // carries storage detail: neither is on the list.
        assert_eq!(
            problem("This knowledge database uses schema version 9."),
            Problem::Unavailable
        );
    }
}
