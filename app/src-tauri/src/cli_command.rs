//! Deterministic spoken CLI recognition and canonical formatting.
//!
//! Activation is deliberately conservative: an utterance must start with an
//! explicit command trigger, start with a known tool plus command-shaped
//! evidence, or come from a profile that explicitly enables CLI formatting.
//! Text that does not activate is returned byte-for-byte unchanged.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CliFormattingMode {
    Auto,
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Alias {
    spoken: Vec<String>,
    written: String,
}

/// Local, in-memory command lexicon. Built-ins cover common CLI spellings;
/// project vocabulary and approved user mappings can extend it without adding
/// whole-command templates to the formatter.
#[derive(Debug, Clone)]
pub(crate) struct CliLexicon {
    aliases: Vec<Alias>,
}

const BUILTIN_ALIASES: &[(&str, &str)] = &[
    ("n p m", "npm"),
    ("n p x", "npx"),
    ("p n p m", "pnpm"),
    ("node js", "node"),
    ("cc usage", "ccusage"),
    ("create vite", "create-vite"),
    ("react typescript", "react-ts"),
    ("test threads", "test-threads"),
    ("tauri", "tauri"),
    ("type script", "typescript"),
    ("java script", "javascript"),
    ("zero", "0"),
    ("one", "1"),
    ("two", "2"),
    ("three", "3"),
    ("four", "4"),
    ("five", "5"),
    ("six", "6"),
    ("seven", "7"),
    ("eight", "8"),
    ("nine", "9"),
    ("ten", "10"),
];

const KNOWN_TOOLS: &[&str] = &[
    "npm", "npx", "pnpm", "yarn", "bun", "git", "cargo", "rustup", "docker", "kubectl", "node",
    "deno", "python", "python3", "pip", "pip3", "go", "make", "cmake", "curl", "wget",
];

const PROSE_FOLLOWERS: &[&str] = &[
    "and", "are", "can", "does", "has", "is", "means", "should", "was", "will",
];

impl CliLexicon {
    pub(crate) fn from_context(
        prompt: Option<&str>,
        approved_mappings: &[(String, String)],
    ) -> Self {
        let mut aliases = Vec::new();

        // Existing custom voice-command pairs are user-approved. Only atom-like
        // replacements are useful as lexicon entries; sentences remain owned by
        // the voice-command stage that runs before this formatter.
        for (spoken, written) in approved_mappings {
            if is_shell_atom(written) {
                add_alias(&mut aliases, spoken, written);
            }
        }

        // CLI-specific aliases win over generic project casing (for example the
        // project vocabulary writes `Tauri`, while an npm script is `tauri`).
        for (spoken, written) in BUILTIN_ALIASES {
            add_alias(&mut aliases, spoken, written);
        }

        // Project/code-vocabulary terms are already captured in the immutable
        // recording context. Derive a spoken form for structured identifiers.
        if let Some(prompt) = prompt {
            for term in prompt.split_whitespace() {
                add_term_alias(&mut aliases, term);
            }
        }

        aliases.sort_by(|a, b| b.spoken.len().cmp(&a.spoken.len()));
        let mut seen = std::collections::HashSet::new();
        aliases.retain(|alias| seen.insert(alias.spoken.clone()));
        Self { aliases }
    }

    #[cfg(test)]
    fn builtins() -> Self {
        Self::from_context(None, &[])
    }

    fn resolve(&self, words: &[String], start: usize) -> Option<(&str, usize)> {
        self.aliases.iter().find_map(|alias| {
            let end = start.checked_add(alias.spoken.len())?;
            if end > words.len() {
                return None;
            }
            words[start..end]
                .iter()
                .zip(&alias.spoken)
                .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
                .then_some((alias.written.as_str(), alias.spoken.len()))
        })
    }
}

fn add_alias(aliases: &mut Vec<Alias>, spoken: &str, written: &str) {
    let spoken = spoken
        .split_whitespace()
        .map(|word| word.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let written = written.trim();
    if !spoken.is_empty() && !written.is_empty() {
        aliases.push(Alias {
            spoken,
            written: written.to_string(),
        });
    }
}

fn add_term_alias(aliases: &mut Vec<Alias>, term: &str) {
    let term = term.trim_matches(|ch: char| ch == ',' || ch == ';');
    if term.is_empty() || !is_shell_atom(term) {
        return;
    }
    let spoken = derive_spoken_form(term);
    if !spoken.is_empty() {
        add_alias(aliases, &spoken, term);
    }
}

fn is_shell_atom(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && !value.chars().any(char::is_whitespace)
        && value
            .chars()
            .all(|ch| ch.is_alphanumeric() || "_-./:@".contains(ch))
}

fn derive_spoken_form(written: &str) -> String {
    let mut out = String::new();
    let mut previous: Option<char> = None;
    for ch in written.chars() {
        let separator = match ch {
            '_' | '-' => Some(" "),
            '/' => Some(" slash "),
            '.' => Some(" dot "),
            ':' => Some(" colon "),
            '@' => Some(" at "),
            _ => None,
        };
        if let Some(separator) = separator {
            out.push_str(separator);
            previous = None;
            continue;
        }
        if ch.is_uppercase()
            && previous.is_some_and(|prev| prev.is_lowercase() || prev.is_numeric())
        {
            out.push(' ');
        }
        out.extend(ch.to_lowercase());
        previous = Some(ch);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Canonicalize one likely command. Non-activated input is returned exactly as
/// received, including whitespace and Unicode surface forms.
pub(crate) fn canonicalize_cli(
    input: &str,
    mode: CliFormattingMode,
    lexicon: &CliLexicon,
) -> String {
    let mut words = input
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if words.is_empty() {
        return input.to_string();
    }

    let trigger_words = explicit_trigger_len(&words);
    let explicit = trigger_words > 0;
    if explicit {
        words.drain(0..trigger_words);
        if words.is_empty() {
            return input.to_string();
        }
    }

    let activated = explicit
        || match mode {
            CliFormattingMode::Disabled => false,
            CliFormattingMode::Enabled => profile_activation(&words, lexicon),
            CliFormattingMode::Auto => automatic_activation(&words, lexicon),
        };
    if !activated {
        return input.to_string();
    }

    strip_sentence_punctuation(&mut words);
    let formatted = format_words(&words, lexicon);
    apply_command_idioms(formatted)
}

fn explicit_trigger_len(words: &[String]) -> usize {
    if starts_with(words, &["terminal", "command"]) || starts_with(words, &["shell", "command"]) {
        2
    } else if starts_with(words, &["command"]) {
        1
    } else {
        0
    }
}

fn automatic_activation(words: &[String], lexicon: &CliLexicon) -> bool {
    let Some((tool, consumed)) = known_tool(words, lexicon) else {
        return false;
    };
    let remainder = &words[consumed..];
    if remainder.is_empty() {
        return false;
    }
    let next = clean_detection_word(&remainder[0]);
    if PROSE_FOLLOWERS.contains(&next.as_str()) {
        return false;
    }
    contains_spoken_syntax(remainder)
        || remainder.iter().any(|word| {
            word.contains('-') || word.contains('/') || word.contains('@') || word.contains('=')
        })
        || known_action(tool, &next)
        || matches!(tool, "npx" | "pnpm" | "yarn" | "bun")
}

fn profile_activation(words: &[String], lexicon: &CliLexicon) -> bool {
    if automatic_activation(words, lexicon) {
        return true;
    }
    if words.len() < 2 {
        return false;
    }
    // Enabling a CLI profile is an explicit user choice to treat a multi-token
    // utterance as command text. The formatter still applies only bounded token
    // and symbol rules; it never performs a free-form prose rewrite.
    true
}

fn known_tool<'a>(words: &[String], lexicon: &'a CliLexicon) -> Option<(&'a str, usize)> {
    if let Some((resolved, consumed)) = lexicon.resolve(words, 0) {
        if KNOWN_TOOLS.contains(&resolved) {
            return Some((resolved, consumed));
        }
    }
    let first = clean_detection_word(words.first()?);
    KNOWN_TOOLS
        .iter()
        .find(|tool| **tool == first)
        .map(|tool| (*tool, 1))
}

fn known_action(tool: &str, action: &str) -> bool {
    let actions: &[&str] = match tool {
        "npm" | "pnpm" | "yarn" | "bun" => &[
            "add", "audit", "build", "exec", "install", "publish", "remove", "run", "start",
            "test", "update",
        ],
        "git" => &[
            "add",
            "bisect",
            "branch",
            "checkout",
            "cherry-pick",
            "clone",
            "commit",
            "diff",
            "fetch",
            "log",
            "merge",
            "pull",
            "push",
            "rebase",
            "remote",
            "reset",
            "restore",
            "revert",
            "show",
            "stash",
            "status",
            "switch",
            "tag",
        ],
        "cargo" => &[
            "add", "bench", "build", "check", "clean", "clippy", "doc", "fix", "fmt", "install",
            "metadata", "new", "publish", "remove", "run", "test", "tree", "update",
        ],
        "docker" => &[
            "build",
            "compose",
            "container",
            "exec",
            "image",
            "inspect",
            "login",
            "logs",
            "network",
            "ps",
            "pull",
            "push",
            "run",
            "stop",
            "tag",
            "volume",
        ],
        "kubectl" => &[
            "apply",
            "config",
            "create",
            "delete",
            "describe",
            "edit",
            "exec",
            "explain",
            "get",
            "logs",
            "patch",
            "port-forward",
            "rollout",
            "scale",
            "set",
            "top",
            "wait",
        ],
        "rustup" => &[
            "component",
            "default",
            "doc",
            "install",
            "override",
            "run",
            "show",
            "target",
            "toolchain",
            "update",
        ],
        "go" => &[
            "build", "clean", "env", "fmt", "generate", "get", "install", "list", "mod", "run",
            "test", "tool", "version", "vet", "work",
        ],
        _ => &[],
    };
    actions.contains(&action)
}

fn contains_spoken_syntax(words: &[String]) -> bool {
    (0..words.len()).any(|index| {
        starts_with_at(words, index, &["at", "latest"])
            || starts_with_at(words, index, &["dash"])
            || starts_with_at(words, index, &["slash"])
            || starts_with_at(words, index, &["dot"])
            || starts_with_at(words, index, &["colon"])
            || starts_with_at(words, index, &["equals"])
            || starts_with_at(words, index, &["pipe"])
            || starts_with_at(words, index, &["redirect"])
            || starts_with_at(words, index, &["double", "quote"])
            || starts_with_at(words, index, &["single", "quote"])
            || starts_with_at(words, index, &["new", "line"])
    })
}

fn strip_sentence_punctuation(words: &mut [String]) {
    let Some(last) = words.last_mut() else {
        return;
    };
    if last.len() > 1
        && last.chars().last().is_some_and(|ch| {
            matches!(ch, ',' | ';' | '?' | '!') || (ch == '.' && !last.contains('/'))
        })
    {
        last.pop();
    }
}

#[derive(Default)]
struct CommandBuilder {
    output: String,
}

impl CommandBuilder {
    fn word(&mut self, word: &str) {
        if word.is_empty() {
            return;
        }
        let joined = self
            .output
            .chars()
            .last()
            .is_some_and(|ch| matches!(ch, ' ' | '\n' | '/' | '.' | ':' | '@' | '='));
        if !self.output.is_empty() && !joined {
            self.output.push(' ');
        }
        self.output.push_str(word);
    }

    fn attached(&mut self, value: &str) {
        while self.output.ends_with(' ') {
            self.output.pop();
        }
        self.output.push_str(value);
    }

    fn operator(&mut self, value: &str) {
        while self.output.ends_with(' ') {
            self.output.pop();
        }
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push(' ');
        }
        self.output.push_str(value);
        self.output.push(' ');
    }

    fn newline(&mut self) {
        while self.output.ends_with(' ') {
            self.output.pop();
        }
        if !self.output.is_empty() && !self.output.ends_with('\n') {
            self.output.push('\n');
        }
    }

    fn finish(mut self) -> String {
        while self.output.ends_with(' ') {
            self.output.pop();
        }
        self.output
    }
}

fn format_words(words: &[String], lexicon: &CliLexicon) -> String {
    let mut out = CommandBuilder::default();
    let mut index = 0;
    while index < words.len() {
        if starts_with_at(words, index, &["new", "line"])
            || starts_with_at(words, index, &["next", "line"])
        {
            out.newline();
            index += 2;
            continue;
        }
        if let Some((quote, trigger_len)) = quote_trigger(words, index) {
            if let Some((content, consumed)) = quoted_content(words, index + trigger_len, quote) {
                out.word(&format!("{quote}{content}{quote}"));
                index += trigger_len + consumed;
                continue;
            }
        }
        if starts_with_at(words, index, &["pipe"]) {
            out.operator("|");
            index += 1;
            continue;
        }
        if starts_with_at(words, index, &["append", "redirect"])
            || starts_with_at(words, index, &["append", "to"])
        {
            out.operator(">>");
            index += 2;
            continue;
        }
        if starts_with_at(words, index, &["redirect", "to"])
            || starts_with_at(words, index, &["greater", "than"])
        {
            out.operator(">");
            index += 2;
            continue;
        }
        if starts_with_at(words, index, &["less", "than"])
            || starts_with_at(words, index, &["redirect", "from"])
        {
            out.operator("<");
            index += 2;
            continue;
        }
        if starts_with_at(words, index, &["equals"]) || starts_with_at(words, index, &["equal"]) {
            out.attached("=");
            index += 1;
            continue;
        }
        if starts_with_at(words, index, &["slash"])
            || starts_with_at(words, index, &["forward", "slash"])
        {
            out.attached("/");
            index += usize::from(words[index].eq_ignore_ascii_case("forward")) + 1;
            continue;
        }
        if starts_with_at(words, index, &["dot"]) || starts_with_at(words, index, &["period"]) {
            out.attached(".");
            index += 1;
            continue;
        }
        if starts_with_at(words, index, &["colon"]) {
            out.attached(":");
            index += 1;
            continue;
        }
        if starts_with_at(words, index, &["at"])
            && index + 1 < words.len()
            && is_version_word(&words[index + 1])
        {
            out.attached("@");
            index += 1;
            continue;
        }
        if starts_with_at(words, index, &["dash", "dash"]) {
            if index + 2 >= words.len() || words[index + 2].eq_ignore_ascii_case("dash") {
                out.word("--");
                index += 2;
                continue;
            }
            let (flag, consumed) = resolve_atom(words, index + 2, lexicon);
            out.word(&format!("--{flag}"));
            index += 2 + consumed;
            continue;
        }
        if starts_with_at(words, index, &["dash"]) && index + 1 < words.len() {
            let (flag, consumed) = resolve_atom(words, index + 1, lexicon);
            let prefix = if flag.chars().count() == 1 { "-" } else { "--" };
            out.word(&format!("{prefix}{flag}"));
            index += 1 + consumed;
            continue;
        }
        let (atom, consumed) = resolve_atom(words, index, lexicon);
        let atom = if index == 0
            && KNOWN_TOOLS
                .iter()
                .any(|tool| atom.eq_ignore_ascii_case(tool))
        {
            atom.to_ascii_lowercase()
        } else {
            atom
        };
        out.word(&atom);
        index += consumed;
    }
    out.finish()
}

fn resolve_atom(words: &[String], index: usize, lexicon: &CliLexicon) -> (String, usize) {
    lexicon
        .resolve(words, index)
        .map(|(written, consumed)| (written.to_string(), consumed))
        .unwrap_or_else(|| (words[index].clone(), 1))
}

fn apply_command_idioms(command: String) -> String {
    command
        .lines()
        .map(|line| {
            let mut words = line
                .split_whitespace()
                .map(str::to_string)
                .collect::<Vec<_>>();
            if words.len() >= 4 && words[0] == "npx" && words[1].starts_with("create-vite") {
                let flag = words[2..]
                    .iter()
                    .position(|word| word.starts_with('-'))
                    .map(|position| position + 2)
                    .unwrap_or(words.len());
                if flag > 3 {
                    let app_name = words[2..flag].join("-");
                    words.splice(2..flag, [app_name]);
                }
            }
            words.join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn quote_trigger(words: &[String], index: usize) -> Option<(char, usize)> {
    if starts_with_at(words, index, &["double", "quote"])
        || starts_with_at(words, index, &["open", "quote"])
    {
        Some(('"', 2))
    } else if starts_with_at(words, index, &["single", "quote"]) {
        Some(('\'', 2))
    } else if starts_with_at(words, index, &["quote"]) {
        Some(('"', 1))
    } else {
        None
    }
}

fn quoted_content(words: &[String], start: usize, quote: char) -> Option<(String, usize)> {
    let mut index = start;
    let mut content = Vec::new();
    while index < words.len() {
        let closing = match quote {
            '\'' => starts_with_at(words, index, &["single", "quote"]),
            _ => {
                starts_with_at(words, index, &["double", "quote"])
                    || starts_with_at(words, index, &["close", "quote"])
                    || starts_with_at(words, index, &["quote"])
            }
        };
        if closing {
            let closing_len = if words[index].eq_ignore_ascii_case("quote") {
                1
            } else {
                2
            };
            return Some((content.join(" "), index - start + closing_len));
        }
        content.push(words[index].clone());
        index += 1;
    }
    None
}

fn is_version_word(word: &str) -> bool {
    let word = clean_detection_word(word);
    matches!(
        word.as_str(),
        "latest" | "next" | "beta" | "canary" | "stable"
    ) || word.chars().next().is_some_and(|ch| ch.is_ascii_digit())
}

fn clean_detection_word(word: &str) -> String {
    word.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '-' && ch != '_')
        .to_ascii_lowercase()
}

fn starts_with(words: &[String], expected: &[&str]) -> bool {
    starts_with_at(words, 0, expected)
}

fn starts_with_at(words: &[String], index: usize, expected: &[&str]) -> bool {
    let Some(slice) = words.get(index..index.saturating_add(expected.len())) else {
        return false;
    };
    slice
        .iter()
        .zip(expected)
        .all(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn auto(input: &str) -> String {
        canonicalize_cli(input, CliFormattingMode::Auto, &CliLexicon::builtins())
    }

    #[test]
    fn issue_golden_examples_are_exact() {
        let fixtures = [
            ("npx cc usage at latest", "npx ccusage@latest"),
            ("NPM run Tauri dev", "npm run tauri dev"),
            (
                "npx create vite at latest my app dash dash template react typescript",
                "npx create-vite@latest my-app --template react-ts",
            ),
            (
                "git checkout dash b feature slash streaming",
                "git checkout -b feature/streaming",
            ),
            (
                "cargo test dash dash dash test threads equals one",
                "cargo test -- --test-threads=1",
            ),
        ];
        for (spoken, expected) in fixtures {
            assert_eq!(auto(spoken), expected, "fixture: {spoken}");
        }
    }

    #[test]
    fn ordinary_prose_is_byte_for_byte_unchanged() {
        let fixtures = [
            "I use git and cargo every day.",
            "Git is useful, but Docker is not a verb here.",
            "cargo cults are a historical topic",
            "Please send it at noon — pipe is a noun.",
            "  npm is a package manager, not this sentence.  ",
        ];
        for prose in fixtures {
            assert_eq!(
                auto(prose).as_bytes(),
                prose.as_bytes(),
                "fixture: {prose:?}"
            );
        }
    }

    #[test]
    fn explicit_trigger_formats_unknown_tools() {
        assert_eq!(
            auto("command mytool dash dash config path slash file"),
            "mytool --config path/file"
        );
    }

    #[test]
    fn enabled_profile_supports_unknown_tools_without_rewriting_plain_text() {
        let lexicon = CliLexicon::builtins();
        assert_eq!(
            canonicalize_cli(
                "mytool dash dash unicode café",
                CliFormattingMode::Enabled,
                &lexicon,
            ),
            "mytool --unicode café"
        );
        assert_eq!(
            canonicalize_cli(
                "ordinary Unicode café 日本語",
                CliFormattingMode::Enabled,
                &lexicon
            ),
            "ordinary Unicode café 日本語"
        );
    }

    #[test]
    fn disabled_profile_still_allows_explicit_trigger_only() {
        let lexicon = CliLexicon::builtins();
        assert_eq!(
            canonicalize_cli(
                "git checkout dash b test",
                CliFormattingMode::Disabled,
                &lexicon
            ),
            "git checkout dash b test"
        );
        assert_eq!(
            canonicalize_cli(
                "command git checkout dash b test",
                CliFormattingMode::Disabled,
                &lexicon
            ),
            "git checkout -b test"
        );
    }

    #[test]
    fn canonical_output_is_idempotent() {
        let fixtures = [
            "npx ccusage@latest",
            "npm run tauri dev",
            "git checkout -b feature/streaming",
            "cargo test -- --test-threads=1",
            "docker run --name café image:latest",
        ];
        for command in fixtures {
            assert_eq!(auto(command), command, "fixture: {command}");
        }
    }

    #[test]
    fn punctuation_quotes_redirects_and_unicode_are_safe() {
        assert_eq!(
            auto("kubectl get pods pipe grep double quote café 日本語 double quote redirect to output dot txt"),
            "kubectl get pods | grep \"café 日本語\" > output.txt"
        );
    }

    #[test]
    fn context_terms_and_approved_atom_mappings_extend_the_lexicon() {
        let lexicon = CliLexicon::from_context(
            Some("deployPreview custom_script"),
            &[("my package".to_string(), "@scope/pkg".to_string())],
        );
        assert_eq!(
            canonicalize_cli(
                "command deploy preview dash dash task custom script",
                CliFormattingMode::Auto,
                &lexicon,
            ),
            "deployPreview --task custom_script"
        );
        assert_eq!(
            canonicalize_cli("command npx my package", CliFormattingMode::Auto, &lexicon),
            "npx @scope/pkg"
        );
    }
}
