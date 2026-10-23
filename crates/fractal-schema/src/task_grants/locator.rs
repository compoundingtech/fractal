//! Mirrors http-capture browser-control.ts, excluding ephemeral `ref` locators.
//! Text limits count UTF-16 code units (JavaScript `.length`). CSS syntax is
//! checked by http-capture at dispatch, not at grant creation.
use super::{TaskGrantError, invalid};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AriaRole {
    Alert, Alertdialog, Application, Article, Banner, Blockquote, Button, Caption,
    Cell, Checkbox, Code, Columnheader, Combobox, Complementary, Contentinfo,
    Definition, Deletion, Dialog, Directory, Document, Emphasis, Feed, Figure,
    Form, Generic, Grid, Gridcell, Group, Heading, Img, Insertion, Link, List,
    Listbox, Listitem, Log, Main, Marquee, Math, Meter, Menu, Menubar, Menuitem,
    Menuitemcheckbox, Menuitemradio, Navigation, None, Note, Option, Paragraph,
    Presentation, Progressbar, Radio, Radiogroup, Region, Row, Rowgroup, Rowheader,
    Scrollbar, Search, Searchbox, Separator, Slider, Spinbutton, Status, Strong,
    Subscript, Superscript, Switch, Tab, Table, Tablist, Tabpanel, Term, Textbox,
    Time, Timer, Toolbar, Tooltip, Tree, Treegrid, Treeitem,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Locator {
    Role { role: AriaRole, name: String },
    Label { value: String },
    Placeholder { value: String },
    TestId { value: String },
    Text { value: String },
    Css { selector: String },
    Nth { within: Box<Locator>, role: AriaRole, index: u64 },
    Within { scope: Box<Locator>, target: Box<Locator> },
    WithinFrame { frame: Box<Locator>, target: Box<Locator> },
}

// ECMAScript String.prototype.trim whitespace, including BOM and excluding
// Rust-only whitespace such as U+0085. Capture uses value.trim().length.
fn js_whitespace(value: char) -> bool {
    matches!(value, '\u{0009}'..='\u{000d}' | '\u{0020}' | '\u{00a0}' |
        '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' |
        '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}')
}

pub(super) fn bounded_text(value: &str, maximum: usize) -> Result<(), TaskGrantError> {
    if value.encode_utf16().count() > maximum || value.trim_matches(js_whitespace).is_empty()
        || value.contains('\0') {
        return Err(invalid(format!("locator text must be nonblank, NUL-free and at most {maximum} UTF-16 code units")));
    }
    Ok(())
}

impl Locator {
    pub fn validate(&self) -> Result<(), TaskGrantError> {
        fn visit(locator: &Locator, depth: usize, nodes: &mut usize) -> Result<(), TaskGrantError> {
            *nodes += 1;
            if depth > 8 || *nodes > 32 {
                return Err(invalid("locator exceeds 32 nodes or recursive depth 8"));
            }
            match locator {
                Locator::Role { name, .. } => bounded_text(name, 512)?,
                Locator::Label { value } | Locator::Placeholder { value } |
                Locator::TestId { value } | Locator::Text { value } => bounded_text(value, 512)?,
                Locator::Css { selector } => bounded_text(selector, 2048)?,
                Locator::Nth { within, index, .. } => {
                    if !(1..=512).contains(index) { return Err(invalid("nth.index must be an integer from 1 through 512")); }
                    visit(within, depth + 1, nodes)?;
                    // Capture counts nth's role target as a synthetic leaf node.
                    *nodes += 1;
                    if *nodes > 32 { return Err(invalid("locator exceeds 32 nodes")); }
                }
                Locator::Within { scope, target } => {
                    visit(scope, depth + 1, nodes)?;
                    visit(target, depth + 1, nodes)?;
                }
                Locator::WithinFrame { frame, target } => {
                    if depth != 1 { return Err(invalid("withinFrame is allowed only at the outermost node")); }
                    visit(frame, depth + 1, nodes)?;
                    visit(target, depth + 1, nodes)?;
                }
            }
            Ok(())
        }
        visit(self, 1, &mut 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn leaf() -> Value { json!({"kind":"text","value":"example"}) }
    fn refused(value: Value) {
        if let Ok(locator) = serde_json::from_value::<Locator>(value) {
            assert!(locator.validate().is_err());
        }
    }

    #[test]
    fn locator_refusals() {
        refused(json!({"kind":"ref","ref":"e1"}));
        refused(json!({"kind":"role","role":"unknown","name":"example"}));
        refused(json!({"kind":"role","role":"button","name":"example","extra":true}));
        for index in [json!(0), json!(513), json!(1.5)] {
            refused(json!({"kind":"nth","within":leaf(),"role":"button","index":index}));
        }
        let mut deep = leaf();
        for _ in 0..8 { deep = json!({"kind":"nth","within":deep,"role":"button","index":1}); }
        refused(deep);
        let mut broad = leaf();
        for _ in 0..4 { broad = json!({"kind":"within","scope":broad.clone(),"target":broad}); }
        // 31-node tree plus a new root and leaf = 33, with depth only six.
        refused(json!({"kind":"within","scope":broad,"target":leaf()}));
        refused(json!({"kind":"within","scope":leaf(),"target":{"kind":"withinFrame","frame":leaf(),"target":leaf()}}));
        refused(json!({"kind":"text","value":"x".repeat(513)}));
        refused(json!({"kind":"css","selector":"x".repeat(2049)}));
        for value in ["".into(), "\u{feff}".into(), "example\0".into(), "😀".repeat(257)] {
            refused(json!({"kind":"label","value":value}));
        }
    }

    #[test]
    fn capture_bounds_and_dispatch_css_policy() {
        let locator: Locator = serde_json::from_value(json!({"kind":"text","value":"😀".repeat(256)})).unwrap();
        locator.validate().unwrap();
        let locator: Locator = serde_json::from_value(json!({"kind":"withinFrame","frame":{"kind":"css","selector":"[not CSS"},"target":leaf()})).unwrap();
        locator.validate().unwrap(); // Syntax is deliberately deferred to dispatch.
        let mut deepest = leaf();
        for _ in 0..7 { deepest = json!({"kind":"nth","within":deepest,"role":"button","index":512}); }
        serde_json::from_value::<Locator>(deepest).unwrap().validate().unwrap();
    }
}
