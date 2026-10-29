//! Android's accessibility hierarchy, parsed without an XML dependency.
//!
//! `uiautomator dump` emits one flat stream of `<node …>` tags whose attributes
//! are quoted strings. That is a small, fixed subset of XML, so the parser below
//! handles exactly it and is unit-tested against real dumps. Anything outside
//! that subset is ignored rather than guessed at.

use serde::{Deserialize, Serialize};

/// Where an element sits on its own Android display. Input coordinates are
/// display-local, so these bounds are what `tap` needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Bounds {
    pub fn width(&self) -> i32 {
        (self.right - self.left).max(0)
    }

    pub fn height(&self) -> i32 {
        (self.bottom - self.top).max(0)
    }

    pub fn center(&self) -> (i32, i32) {
        (self.left + self.width() / 2, self.top + self.height() / 2)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiNode {
    pub index: usize,
    pub text: String,
    pub content_desc: String,
    pub resource_id: String,
    pub class: String,
    pub package_id: String,
    pub clickable: bool,
    pub enabled: bool,
    pub focused: bool,
    pub scrollable: bool,
    pub checked: bool,
    pub bounds: Bounds,
}

impl UiNode {
    pub fn center(&self) -> (i32, i32) {
        self.bounds.center()
    }

    /// The shortest human label this element has, for a compact state summary.
    pub fn label(&self) -> Option<&str> {
        let candidates = [
            self.text.as_str(),
            self.content_desc.as_str(),
            self.resource_id.as_str(),
        ];
        candidates
            .into_iter()
            .find(|value| !value.trim().is_empty())
    }
}

/// Which element the caller means. Every field is optional and all present
/// fields must match; that keeps "the Save button" expressible without an
/// expression language.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Selector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_contains: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_desc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clickable: Option<bool>,
    /// Position in the parsed list; a last resort when text is duplicated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
}

impl Selector {
    pub fn is_empty(&self) -> bool {
        self.text.is_none()
            && self.text_contains.is_none()
            && self.content_desc.is_none()
            && self.resource_id.is_none()
            && self.class_name.is_none()
            && self.clickable.is_none()
            && self.index.is_none()
    }

    pub fn matches(&self, node: &UiNode) -> bool {
        if let Some(index) = self.index {
            if node.index != index {
                return false;
            }
        }
        if let Some(text) = &self.text {
            if node.text.trim() != text.trim() {
                return false;
            }
        }
        if let Some(needle) = &self.text_contains {
            let needle = needle.to_lowercase();
            let haystack = format!("{} {}", node.text, node.content_desc).to_lowercase();
            if !haystack.contains(&needle) {
                return false;
            }
        }
        if let Some(desc) = &self.content_desc {
            if node.content_desc.trim() != desc.trim() {
                return false;
            }
        }
        if let Some(id) = &self.resource_id {
            if node.resource_id.trim() != id.trim() {
                return false;
            }
        }
        if let Some(class) = &self.class_name {
            // Accept both the full name and its last segment ("Button").
            let matches = node.class == *class
                || node.class.rsplit('.').next().unwrap_or(&node.class) == class.as_str();
            if !matches {
                return false;
            }
        }
        if let Some(clickable) = self.clickable {
            if node.clickable != clickable {
                return false;
            }
        }
        true
    }
}

pub fn find<'a>(nodes: &'a [UiNode], selector: &Selector) -> Option<&'a UiNode> {
    nodes.iter().find(|node| selector.matches(node))
}

/// Elements worth reporting: anything a person could see or act on. Empty
/// layout containers are noise that would otherwise dominate an agent's context.
pub fn interesting(nodes: &[UiNode]) -> Vec<UiNode> {
    nodes
        .iter()
        .filter(|node| {
            !node.text.trim().is_empty()
                || !node.content_desc.trim().is_empty()
                || node.clickable
                || node.scrollable
                || node.checked
        })
        .cloned()
        .collect()
}

/// Parse `uiautomator dump` output. Keeps at most `limit` nodes.
pub fn parse_hierarchy(xml: &str, package_filter: Option<&str>, limit: usize) -> Vec<UiNode> {
    let mut nodes = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<node") {
        rest = &rest[start + "<node".len()..];
        let Some(end) = tag_end(rest) else {
            break;
        };
        let tag = &rest[..end];
        rest = &rest[end..];
        let package_id = attr(tag, "package").unwrap_or_default();
        if let Some(filter) = package_filter {
            if !filter.is_empty() && package_id != filter {
                continue;
            }
        }
        nodes.push(UiNode {
            index: nodes.len(),
            text: attr(tag, "text").unwrap_or_default(),
            content_desc: attr(tag, "content-desc").unwrap_or_default(),
            resource_id: attr(tag, "resource-id").unwrap_or_default(),
            class: attr(tag, "class").unwrap_or_default(),
            package_id,
            clickable: attr(tag, "clickable").as_deref() == Some("true"),
            enabled: attr(tag, "enabled").as_deref() != Some("false"),
            focused: attr(tag, "focused").as_deref() == Some("true"),
            scrollable: attr(tag, "scrollable").as_deref() == Some("true"),
            checked: attr(tag, "checked").as_deref() == Some("true"),
            bounds: attr(tag, "bounds")
                .and_then(|value| parse_bounds(&value))
                .unwrap_or(Bounds {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                }),
        });
        if nodes.len() >= limit {
            break;
        }
    }
    nodes
}

/// Index of the `>` that ends this tag, ignoring a `>` inside a quoted value.
fn tag_end(tag_body: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (index, ch) in tag_body.char_indices() {
        match (quote, ch) {
            (Some(open), c) if c == open => quote = None,
            (None, '"') | (None, '\'') => quote = Some(ch),
            (None, '>') => return Some(index),
            _ => {}
        }
    }
    None
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let mut search_from = 0;
    loop {
        let found = tag[search_from..].find(&needle)?;
        let value_start = search_from + found + needle.len();
        // Only an attribute name preceded by whitespace counts: `class` must not
        // match inside `myclass`.
        let preceded_by_space = tag[..search_from + found]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_whitespace());
        let end = tag[value_start..].find('"')?;
        if preceded_by_space {
            return Some(decode_entities(&tag[value_start..value_start + end]));
        }
        search_from = value_start + end;
    }
}

fn decode_entities(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        rest = &rest[index..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#")
                .and_then(|code| code.parse::<u32>().ok())
                .and_then(char::from_u32),
        };
        match decoded {
            Some(ch) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `[12,34][56,78]` → bounds.
pub fn parse_bounds(value: &str) -> Option<Bounds> {
    let value = value.trim();
    let inner = value.strip_prefix('[')?.strip_suffix(']')?;
    let (first, second) = inner.split_once("][")?;
    let (left, top) = first.split_once(',')?;
    let (right, bottom) = second.split_once(',')?;
    Some(Bounds {
        left: left.trim().parse().ok()?,
        top: top.trim().parse().ok()?,
        right: right.trim().parse().ok()?,
        bottom: bottom.trim().parse().ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?><hierarchy rotation="0"><node index="0" text="" resource-id="" class="android.widget.FrameLayout" package="org.fdroid.fdroid" content-desc="" checkable="false" checked="false" clickable="false" enabled="true" focusable="false" focused="false" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[0,0][540,1170]"><node index="0" text="Search" resource-id="org.fdroid.fdroid:id/search" class="android.widget.EditText" package="org.fdroid.fdroid" content-desc="Search apps" checkable="false" checked="false" clickable="true" enabled="true" focusable="true" focused="true" scrollable="false" long-clickable="false" password="false" selected="false" bounds="[24,120][516,220]" /><node index="1" text="Nearby &amp; more" resource-id="" class="android.widget.TextView" package="org.fdroid.fdroid" content-desc="" clickable="false" enabled="true" focused="false" scrollable="false" bounds="[24,300][300,360]" /></node></hierarchy>"#;

    #[test]
    fn a_real_dump_becomes_addressable_elements() {
        let nodes = parse_hierarchy(DUMP, None, 100);
        assert_eq!(nodes.len(), 3);
        let search = find(
            &nodes,
            &Selector {
                content_desc: Some("Search apps".into()),
                ..Default::default()
            },
        )
        .expect("search field");
        assert_eq!(search.resource_id, "org.fdroid.fdroid:id/search");
        assert_eq!(search.center(), (270, 170));
        assert!(search.clickable);
        assert!(search.focused);
    }

    #[test]
    fn entities_are_decoded_and_do_not_break_attribute_parsing() {
        let nodes = parse_hierarchy(DUMP, None, 100);
        let label = nodes.iter().find(|n| n.text.contains("Nearby")).unwrap();
        assert_eq!(label.text, "Nearby & more");
        assert_eq!(label.bounds.width(), 276);
    }

    #[test]
    fn a_package_filter_keeps_only_that_app() {
        let mixed = r#"<hierarchy><node text="A" package="app.one" class="android.widget.TextView" clickable="false" enabled="true" focused="false" scrollable="false" checked="false" bounds="[0,0][10,10]"/><node text="B" package="app.two" class="android.widget.TextView" clickable="false" enabled="true" focused="false" scrollable="false" checked="false" bounds="[0,0][10,10]"/></hierarchy>"#;
        let nodes = parse_hierarchy(mixed, Some("app.two"), 100);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].text, "B");
    }

    #[test]
    fn selectors_require_every_given_field_to_match() {
        let nodes = parse_hierarchy(DUMP, None, 100);
        let selector = Selector {
            text: Some("Search".into()),
            clickable: Some(false),
            ..Default::default()
        };
        assert!(find(&nodes, &selector).is_none());
        let selector = Selector {
            text_contains: Some("SEARCH".into()),
            clickable: Some(true),
            ..Default::default()
        };
        assert!(find(&nodes, &selector).is_some());
    }

    #[test]
    fn a_class_selector_accepts_the_short_name() {
        let nodes = parse_hierarchy(DUMP, None, 100);
        let selector = Selector {
            class_name: Some("EditText".into()),
            ..Default::default()
        };
        assert!(find(&nodes, &selector).is_some());
    }

    #[test]
    fn empty_containers_are_not_reported_as_elements() {
        let nodes = parse_hierarchy(DUMP, None, 100);
        let visible = interesting(&nodes);
        assert_eq!(visible.len(), 2);
        assert!(visible.iter().all(|n| n.bounds.width() > 0));
    }

    #[test]
    fn malformed_bounds_do_not_panic_or_invent_a_position() {
        assert_eq!(
            parse_bounds("[1,2][3,4]"),
            Some(Bounds {
                left: 1,
                top: 2,
                right: 3,
                bottom: 4
            })
        );
        assert_eq!(parse_bounds("[1,2]"), None);
        assert_eq!(parse_bounds("garbage"), None);
        let nodes = parse_hierarchy(
            r#"<node text="x" bounds="garbage" package="p" class="c" clickable="false" enabled="true" focused="false" scrollable="false" checked="false"/>"#,
            None,
            10,
        );
        assert_eq!(nodes[0].center(), (0, 0));
    }

    #[test]
    fn the_node_cap_is_respected() {
        let mut xml = String::from("<hierarchy>");
        for i in 0..50 {
            xml.push_str(&format!(
                r#"<node text="n{i}" bounds="[0,0][1,1]" package="p" class="c" clickable="false" enabled="true" focused="false" scrollable="false" checked="false"/>"#
            ));
        }
        xml.push_str("</hierarchy>");
        assert_eq!(parse_hierarchy(&xml, None, 7).len(), 7);
    }
}
