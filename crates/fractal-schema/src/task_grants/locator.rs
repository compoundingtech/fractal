use serde::{Deserialize, Serialize};

/// HTTP Capture's exact closed role vocabulary at dotfiles revision 6cf44a336955.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AriaRole {
    Alert, AlertDialog, Application, Article, Banner, Blockquote, Button, Caption,
    Cell, Checkbox, Code, ColumnHeader, Combobox, Complementary, ContentInfo,
    Definition, Deletion, Dialog, Directory, Document, Emphasis, Feed, Figure,
    Form, Generic, Grid, GridCell, Group, Heading, Img, Insertion, Link, List,
    Listbox, ListItem, Log, Main, Marquee, Math, Meter, Menu, Menubar, MenuItem,
    MenuItemCheckbox, MenuItemRadio, Navigation, None, Note, Option, Paragraph,
    Presentation, Progressbar, Radio, RadioGroup, Region, Row, RowGroup,
    RowHeader, Scrollbar, Search, Searchbox, Separator, Slider, Spinbutton,
    Status, Strong, Subscript, Superscript, Switch, Tab, Table, TabList,
    TabPanel, Term, Textbox, Time, Timer, Toolbar, Tooltip, Tree, TreeGrid,
    TreeItem,
}

// Share the wire variants without flattening a closed object. Every recursive
// child is non-frame, so withinFrame can only occur at the outermost level.
macro_rules! locator_enum {
    ($name:ident { $($extra:tt)* }) => {
        #[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
        #[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
        pub enum $name {
            Role { role: AriaRole, name: String },
            Label { value: String },
            Placeholder { value: String },
            TestId { value: String },
            Text { value: String },
            Css { selector: String },
            Nth {
                within: Box<TaskNonFrameLocatorV1>,
                role: AriaRole,
                index: u64,
            },
            Within {
                scope: Box<TaskNonFrameLocatorV1>,
                target: Box<TaskNonFrameLocatorV1>,
            },
            $($extra)*
        }
    };
}

locator_enum!(TaskNonFrameLocatorV1 {});
locator_enum!(TaskLocatorV1 {
    WithinFrame {
        frame: TaskNonFrameLocatorV1,
        target: TaskNonFrameLocatorV1,
    },
});

#[cfg(test)]
mod tests {
    use super::{AriaRole, TaskLocatorV1, TaskNonFrameLocatorV1};
    use serde_json::{Value, json};

    fn examples() -> Vec<Value> {
        vec![
            json!({"kind":"role","role":"button","name":"Save"}),
            json!({"kind":"label","value":"Name"}),
            json!({"kind":"placeholder","value":"Name"}),
            json!({"kind":"testId","value":"name"}),
            json!({"kind":"text","value":"Save"}),
            json!({"kind":"css","selector":"#save"}),
            json!({"kind":"nth","within":{"kind":"css","selector":"main"},"role":"button","index":1}),
            json!({"kind":"within","scope":{"kind":"css","selector":"main"},"target":{"kind":"text","value":"Save"}}),
        ]
    }

    #[test]
    fn all_locator_variants_round_trip_without_rewriting() {
        for value in examples() {
            let outer: TaskLocatorV1 = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(outer).unwrap(), value);
            let inner: TaskNonFrameLocatorV1 = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(inner).unwrap(), value);
        }
        let value = json!({"kind":"withinFrame","frame":{"kind":"css","selector":"iframe"},"target":{"kind":"text","value":"Save"}});
        let locator: TaskLocatorV1 = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(locator).unwrap(), value);
        assert!(serde_json::from_value::<TaskNonFrameLocatorV1>(value).is_err());
    }

    #[test]
    fn locators_reject_unknown_keys_tags_and_nested_frames() {
        for mut value in examples() {
            value.as_object_mut().unwrap().insert("extra".into(), json!(true));
            assert!(serde_json::from_value::<TaskLocatorV1>(value.clone()).is_err());
            assert!(serde_json::from_value::<TaskNonFrameLocatorV1>(value).is_err());
        }
        let frame = json!({"kind":"withinFrame","frame":{"kind":"css","selector":"iframe"},"target":{"kind":"text","value":"Save"}});
        for value in [
            json!({"kind":"ref","ref":"e1"}),
            json!({"kind":"unknown"}),
            json!({"kind":"role","role":"unknown","name":"Save"}),
            json!({"kind":"nth","within":frame,"role":"button","index":1}),
            json!({"kind":"within","scope":frame,"target":{"kind":"text","value":"Save"}}),
            json!({"kind":"within","scope":{"kind":"text","value":"Save"},"target":frame}),
            json!({"kind":"withinFrame","frame":frame,"target":{"kind":"text","value":"Save"}}),
            json!({"kind":"withinFrame","frame":{"kind":"css","selector":"iframe"},"target":frame}),
            json!({"kind":"withinFrame","frame":{"kind":"css","selector":"iframe","extra":1},"target":{"kind":"text","value":"Save"}}),
            json!({"kind":"withinFrame","frame":{"kind":"css","selector":"iframe"},"target":{"kind":"text","value":"Save"},"extra":1}),
        ] {
            assert!(serde_json::from_value::<TaskLocatorV1>(value).is_err());
        }
    }

    #[test]
    fn aria_roles_match_the_pinned_capture_vocabulary() {
        let roles = "alert alertdialog application article banner blockquote button caption cell checkbox code columnheader combobox complementary contentinfo definition deletion dialog directory document emphasis feed figure form generic grid gridcell group heading img insertion link list listbox listitem log main marquee math meter menu menubar menuitem menuitemcheckbox menuitemradio navigation none note option paragraph presentation progressbar radio radiogroup region row rowgroup rowheader scrollbar search searchbox separator slider spinbutton status strong subscript superscript switch tab table tablist tabpanel term textbox time timer toolbar tooltip tree treegrid treeitem";
        for role in roles.split_whitespace() {
            let value = json!(role);
            let parsed: AriaRole = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), value);
        }
        for role in ["Button", "alertDialog", "image", "unknown", ""] {
            assert!(serde_json::from_value::<AriaRole>(json!(role)).is_err());
        }
    }
}
