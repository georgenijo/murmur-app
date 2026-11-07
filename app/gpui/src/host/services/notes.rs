//! Release notes are passive Markdown. Images and HTML cannot fetch resources;
//! links become explicit NSWorkspace intents only after an allowed click.
use gpui_component::text::{
    MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast::Node,
};
struct Passive(bool);
impl MarkdownPlugin for Passive {
    fn is_block(&self) -> bool {
        self.0
    }
    fn name(&self) -> &str {
        if self.0 {
            "notes-passive-block"
        } else {
            "notes-passive-inline"
        }
    }
    fn parse(&self, node: &Node, _: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
        let text = match node {
            Node::Image(image) => image.alt.clone(),
            Node::ImageReference(image) => image.alt.clone(),
            Node::Html(_) => String::new(),
            _ => return None,
        };
        Some(MarkdownNode::new(self.name(), ()).text(text))
    }
}
pub fn extensions() -> MarkdownExtensions {
    MarkdownExtensions::default()
        .plugin(Passive(true))
        .plugin(Passive(false))
}
pub fn allowed_link(raw: &str) -> bool {
    url::Url::parse(raw).is_ok_and(|url| {
        matches!(url.scheme(), "https" | "http")
            && url.has_host()
            && url.username().is_empty()
            && url.password().is_none()
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notes_links_are_explicit_web_destinations() {
        assert!(allowed_link(
            "https://github.com/georgenijo/murmur-app/releases"
        ));
        for url in [
            "javascript:alert(1)",
            "file:///synthetic/secret",
            "data:image/png,synthetic",
            "https://user:secret@example.com",
            "murmur://run",
        ] {
            assert!(!allowed_link(url));
        }
    }
}
