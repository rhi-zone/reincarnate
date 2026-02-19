//! Twine story extraction from compiled HTML files.
//!
//! All Twine 2 story formats (SugarCube, Harlowe, Snowman, Chapbook) compile
//! to a single HTML file with embedded `<tw-storydata>` and `<tw-passagedata>`
//! elements. This module extracts the raw passage data from that container,
//! producing a format-agnostic `Story` that the per-format parsers consume.

use scraper::{Html, Selector};

/// A Twine story extracted from its HTML container.
#[derive(Debug)]
pub struct Story {
    /// The story title from the `name` attribute.
    pub name: String,
    /// The story format (e.g. "SugarCube", "Harlowe").
    pub format: String,
    /// The story format version (e.g. "2.36.1").
    pub format_version: String,
    /// The IFID (Interactive Fiction IDentifier).
    pub ifid: String,
    /// The pid of the starting passage.
    pub start_pid: u32,
    /// All passages in the story.
    pub passages: Vec<Passage>,
    /// Contents of `<script id="twine-user-script">` blocks.
    pub user_scripts: Vec<String>,
    /// Contents of `<style id="twine-user-stylesheet">` blocks.
    pub user_styles: Vec<String>,
    /// The story format's built-in CSS (e.g. from `<style title="Twine CSS">`).
    pub format_css: Option<String>,
}

/// A single passage extracted from a `<tw-passagedata>` element.
#[derive(Debug)]
pub struct Passage {
    /// The passage's numeric id (`pid` attribute).
    pub pid: u32,
    /// The passage name (used for linking).
    pub name: String,
    /// Space-separated tags.
    pub tags: Vec<String>,
    /// The raw passage source text (macros, links, HTML, plain text).
    pub source: String,
}

/// Extract a `Story` from Twine 2 compiled HTML.
///
/// Parses the `<tw-storydata>` element and all child `<tw-passagedata>`
/// elements. The passage `source` is left unparsed — that's the job of
/// the format-specific parser (SugarCube, Harlowe, etc.).
pub fn extract_story(html: &str) -> Result<Story, ExtractError> {
    let document = Html::parse_document(html);

    let sd_sel = Selector::parse("tw-storydata").unwrap();
    let sd = document
        .select(&sd_sel)
        .next()
        .ok_or(ExtractError::NoStoryData)?;

    let name = sd.attr("name").unwrap_or_default().to_string();
    let format = sd.attr("format").unwrap_or_default().to_string();
    let format_version = sd.attr("format-version").unwrap_or_default().to_string();
    let ifid = sd.attr("ifid").unwrap_or_default().to_string();
    let start_pid: u32 = sd
        .attr("startnode")
        .unwrap_or("1")
        .parse()
        .unwrap_or(1);

    let pd_sel = Selector::parse("tw-passagedata").unwrap();
    let passages: Vec<Passage> = document
        .select(&pd_sel)
        .map(|pd| {
            let pid = pd.attr("pid").unwrap_or("0").parse().unwrap_or(0);
            let pname = pd.attr("name").unwrap_or_default().to_string();
            let tags_str = pd.attr("tags").unwrap_or_default();
            let tags = if tags_str.is_empty() {
                Vec::new()
            } else {
                tags_str.split_whitespace().map(String::from).collect()
            };
            let source: String = pd.text().collect();
            Passage {
                pid,
                name: pname,
                tags,
                source,
            }
        })
        .collect();

    if passages.is_empty() {
        return Err(ExtractError::NoPassages);
    }

    let script_sel = Selector::parse(r#"script[id="twine-user-script"]"#).unwrap();
    let user_scripts: Vec<String> = document
        .select(&script_sel)
        .map(|el| el.text().collect())
        .collect();

    let style_sel = Selector::parse(r#"style[id="twine-user-stylesheet"]"#).unwrap();
    let user_styles: Vec<String> = document
        .select(&style_sel)
        .map(|el| el.text().collect())
        .collect();

    let css_sel = Selector::parse(r#"style[title="Twine CSS"]"#).unwrap();
    let format_css = document.select(&css_sel).next().and_then(|el| {
        let css = el.text().collect::<String>();
        let trimmed = css.trim().to_string();
        if trimmed.is_empty() { None } else { Some(trimmed) }
    });

    Ok(Story {
        name,
        format,
        format_version,
        ifid,
        start_pid,
        passages,
        user_scripts,
        user_styles,
        format_css,
    })
}

#[derive(Debug)]
pub enum ExtractError {
    NoStoryData,
    NoPassages,
}

impl std::fmt::Display for ExtractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoStoryData => write!(f, "no <tw-storydata> element found in HTML"),
            Self::NoPassages => write!(f, "no passages found in story data"),
        }
    }
}

impl std::error::Error for ExtractError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_minimal_story() {
        let html = r#"
<html><head></head><body>
<tw-storydata name="Test Story" startnode="1" format="SugarCube" format-version="2.36.1" ifid="AAAA-BBBB" hidden>
<tw-passagedata pid="1" name="Start" tags="" position="0,0" size="100,100">Hello &amp; welcome!</tw-passagedata>
<tw-passagedata pid="2" name="Room" tags="nobr" position="100,0" size="100,100">You are in a room.
&lt;&lt;link [[Leave|Start]]&gt;&gt;&lt;&lt;/link&gt;&gt;</tw-passagedata>
</tw-storydata>
</body></html>
"#;
        let story = extract_story(html).unwrap();
        assert_eq!(story.name, "Test Story");
        assert_eq!(story.format, "SugarCube");
        assert_eq!(story.format_version, "2.36.1");
        assert_eq!(story.ifid, "AAAA-BBBB");
        assert_eq!(story.start_pid, 1);
        assert_eq!(story.passages.len(), 2);
        assert_eq!(story.passages[0].name, "Start");
        assert_eq!(story.passages[0].source, "Hello & welcome!");
        assert_eq!(story.passages[1].name, "Room");
        assert_eq!(story.passages[1].tags, vec!["nobr"]);
    }

    #[test]
    fn extract_user_scripts() {
        let html = r#"
<tw-storydata name="S" startnode="1" format="SugarCube" format-version="2.0" ifid="X" hidden>
<tw-passagedata pid="1" name="Start" tags="">Hi</tw-passagedata>
</tw-storydata>
<script id="twine-user-script" type="text/twine-javascript">window.setup = {};</script>
"#;
        let story = extract_story(html).unwrap();
        assert_eq!(story.user_scripts.len(), 1);
        assert_eq!(story.user_scripts[0], "window.setup = {};");
    }

    #[test]
    fn no_story_data_errors() {
        let html = "<html><body>Nothing here</body></html>";
        assert!(extract_story(html).is_err());
    }

    #[test]
    fn extract_format_css_from_style_tag() {
        let html = r#"
<html><head>
<style title="Twine CSS">tw-story { color: white; }</style>
</head><body>
<tw-storydata name="Test" startnode="1" format="Harlowe" format-version="3.3.9" ifid="X" hidden>
<tw-passagedata pid="1" name="Start" tags="">Hello</tw-passagedata>
</tw-storydata>
</body></html>
"#;
        let story = extract_story(html).unwrap();
        assert_eq!(
            story.format_css.as_deref(),
            Some("tw-story { color: white; }")
        );
    }

    #[test]
    fn no_format_css_returns_none() {
        let html = r#"
<tw-storydata name="S" startnode="1" format="SugarCube" format-version="2.0" ifid="X" hidden>
<tw-passagedata pid="1" name="Start" tags="">Hi</tw-passagedata>
</tw-storydata>
"#;
        let story = extract_story(html).unwrap();
        assert!(story.format_css.is_none());
    }

    #[test]
    fn stylesheet_block_stops_at_style_not_script() {
        let html = r#"
<tw-storydata name="S" startnode="1" format="SugarCube" format-version="2.0" ifid="X" hidden>
<tw-passagedata pid="1" name="Start" tags="">Hi</tw-passagedata>
</tw-storydata>
<style id="twine-user-stylesheet" type="text/twine-css">body { color: red; }</style>
<script id="twine-user-script" type="text/twine-javascript">window.setup = {};</script>
"#;
        let story = extract_story(html).unwrap();
        assert_eq!(story.user_styles.len(), 1);
        assert_eq!(story.user_styles[0], "body { color: red; }");
        assert_eq!(story.user_scripts.len(), 1);
        assert_eq!(story.user_scripts[0], "window.setup = {};");
    }

    #[test]
    fn script_content_with_style_closing_tag() {
        let html = r#"
<tw-storydata name="S" startnode="1" format="SugarCube" format-version="2.0" ifid="X" hidden>
<tw-passagedata pid="1" name="Start" tags="">Hi</tw-passagedata>
</tw-storydata>
<script id="twine-user-script" type="text/twine-javascript">var s = "</style>"; var x = 1;</script>
"#;
        let story = extract_story(html).unwrap();
        assert_eq!(story.user_scripts.len(), 1);
        assert_eq!(story.user_scripts[0], r#"var s = "</style>"; var x = 1;"#);
    }

    #[test]
    fn script_style_inside_storydata() {
        let html = r#"
<tw-storydata name="S" startnode="1" format="Harlowe" format-version="3.3.9" ifid="X" hidden>
<style role="stylesheet" id="twine-user-stylesheet" type="text/twine-css">body { margin: 0; }</style>
<script role="script" id="twine-user-script" type="text/twine-javascript">setup.foo = 1;</script>
<tw-passagedata pid="1" name="Start" tags="">Hello</tw-passagedata>
</tw-storydata>
"#;
        let story = extract_story(html).unwrap();
        assert_eq!(story.user_styles, vec!["body { margin: 0; }"]);
        assert_eq!(story.user_scripts, vec!["setup.foo = 1;"]);
    }
}
