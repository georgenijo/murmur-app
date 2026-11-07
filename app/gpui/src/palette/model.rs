pub use crate::workspace::{Event, Problem, Session, Ticket};
use crate::{section, workspace::Projection};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub section: String,
    pub keywords: Vec<String>,
    pub hint: Option<String>,
}
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub commands: Vec<Command>,
}
impl Projection for Snapshot {
    fn revision(&self) -> u64 {
        self.revision
    }
    fn bytes(&self) -> usize {
        self.commands
            .iter()
            .map(|c| {
                c.id.len()
                    + c.title.len()
                    + c.section.len()
                    + c.keywords.iter().map(String::len).sum::<usize>()
                    + c.hint.as_ref().map_or(0, String::len)
                    + 128
            })
            .sum()
    }
    fn valid(&self) -> bool {
        self.commands.len() <= 256
            && self.commands.iter().enumerate().all(|(i, c)| {
                !c.id.is_empty()
                    && !c.title.trim().is_empty()
                    && section::text(&c.id, 128)
                    && section::text(&c.title, 256)
                    && section::text(&c.section, 64)
                    && c.keywords.len() <= 32
                    && c.keywords.iter().all(|s| section::text(s, 128))
                    && c.hint.as_ref().is_none_or(|s| section::text(s, 128))
                    && !self.commands[..i].iter().any(|p| p.id == c.id)
            })
    }
}
pub fn score(c: &Command, query: &str) -> Option<i32> {
    let title = c.title.to_lowercase();
    let tokens: Vec<_> = query.split_whitespace().map(str::to_lowercase).collect();
    if tokens.is_empty() {
        return Some(0);
    }
    let mut total = 0;
    for token in tokens {
        total += if title == token {
            1000
        } else if title.starts_with(&token) {
            800
        } else if title.split_whitespace().any(|s| s.starts_with(&token)) {
            700
        } else if title.contains(&token) {
            600
        } else if std::iter::once(&c.section)
            .chain(c.keywords.iter())
            .any(|s| s.to_lowercase().contains(&token))
        {
            400
        } else {
            let mut chars = token.chars();
            let mut next = chars.next();
            for ch in title.chars() {
                if Some(ch) == next {
                    next = chars.next()
                }
            }
            if next.is_some() {
                return None;
            }
            200
        };
    }
    Some(total * 100 - c.title.encode_utf16().count().min(99) as i32)
}
pub fn filter(commands: &[Command], query: &str) -> Vec<usize> {
    let mut rows: Vec<_> = commands
        .iter()
        .enumerate()
        .filter_map(|(i, c)| score(c, query).map(|s| (i, s)))
        .collect();
    if query.trim().is_empty() {
        let mut sections = Vec::new();
        for c in commands {
            if !sections.contains(&c.section) {
                sections.push(c.section.clone())
            }
        }
        rows.sort_by_key(|(i, _)| sections.iter().position(|s| *s == commands[*i].section));
    } else {
        rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    }
    rows.into_iter().map(|(i, _)| i).collect()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Palette,
    Search,
    Settings,
    Logs,
}
pub fn shortcut(
    key: &str,
    command: bool,
    control: bool,
    alt: bool,
    shift: bool,
    editable: bool,
) -> Option<Shortcut> {
    if !(command || control) || alt || shift || (editable && !command) {
        return None;
    }
    match key.to_lowercase().as_str() {
        "k" => Some(Shortcut::Palette),
        "f" => Some(Shortcut::Search),
        "," => Some(Shortcut::Settings),
        "l" => Some(Shortcut::Logs),
        _ => None,
    }
}
