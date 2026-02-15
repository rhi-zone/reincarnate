//! Shared HTML tag tokenization using html5ever.
//!
//! Wraps html5ever's tokenizer to extract structured tag info from raw
//! `<tag attrs...>` strings. Used by both Harlowe and SugarCube parsers.

use std::cell::RefCell;

use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};

/// Parsed information about an HTML tag.
#[derive(Debug, Clone)]
pub struct HtmlTagInfo {
    /// The tag name (lowercased by html5ever).
    pub name: String,
    /// Attributes as (name, value) pairs.
    pub attrs: Vec<(String, String)>,
    /// Whether this is an end tag (`</tag>`).
    pub is_end: bool,
    /// Whether the tag is self-closing (`<br/>`) or a known HTML5 void element.
    pub is_void: bool,
}

/// HTML5 void elements (self-closing by spec).
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
    "source", "track", "wbr",
];

/// Tokenize a raw HTML tag string (e.g. `<div class="foo">` or `</div>`)
/// into structured tag info.
pub fn tokenize_html_tag(raw: &str) -> Option<HtmlTagInfo> {
    struct TagSink {
        tag: RefCell<Option<Tag>>,
    }

    impl TokenSink for TagSink {
        type Handle = ();

        fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
            if let Token::TagToken(tag) = token {
                *self.tag.borrow_mut() = Some(tag);
            }
            TokenSinkResult::Continue
        }
    }

    let sink = TagSink {
        tag: RefCell::new(None),
    };
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());

    let input = BufferQueue::default();
    input.push_back(StrTendril::from(raw));
    let _ = tokenizer.feed(&input);
    tokenizer.end();

    let tag = tokenizer.sink.tag.into_inner()?;
    let name = tag.name.to_string();
    let is_end = tag.kind == TagKind::EndTag;
    let is_void = tag.self_closing || VOID_ELEMENTS.contains(&name.as_str());

    Some(HtmlTagInfo {
        name,
        attrs: tag
            .attrs
            .iter()
            .map(|a| (a.name.local.to_string(), a.value.to_string()))
            .collect(),
        is_end,
        is_void,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_tag() {
        let info = tokenize_html_tag("<div class=\"foo\">").unwrap();
        assert_eq!(info.name, "div");
        assert!(!info.is_end);
        assert!(!info.is_void);
        assert_eq!(info.attrs, vec![("class".to_string(), "foo".to_string())]);
    }

    #[test]
    fn test_close_tag() {
        let info = tokenize_html_tag("</div>").unwrap();
        assert_eq!(info.name, "div");
        assert!(info.is_end);
    }

    #[test]
    fn test_void_element() {
        let info = tokenize_html_tag("<br>").unwrap();
        assert_eq!(info.name, "br");
        assert!(info.is_void);
    }

    #[test]
    fn test_self_closing() {
        let info = tokenize_html_tag("<img src=\"test.png\" />").unwrap();
        assert_eq!(info.name, "img");
        assert!(info.is_void);
        assert_eq!(
            info.attrs,
            vec![("src".to_string(), "test.png".to_string())]
        );
    }

    #[test]
    fn test_multiple_attrs() {
        let info = tokenize_html_tag("<td colspan=\"2\" style=\"color:red\">").unwrap();
        assert_eq!(info.name, "td");
        assert!(!info.is_void);
        assert_eq!(info.attrs.len(), 2);
    }
}
