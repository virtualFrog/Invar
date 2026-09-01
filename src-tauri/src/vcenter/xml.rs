//! A minimal XML tree.
//!
//! vim25 responses are deeply nested and polymorphic, so every table parses the
//! same shape: walk to a node, read a child's text, or iterate typed array
//! elements. Parsing once into a tree keeps that logic in one place instead of
//! hand-rolling a quick-xml state machine per table.

use quick_xml::events::Event;
use quick_xml::Reader;

#[derive(Debug, Clone, Default)]
pub struct Element {
    pub name: String,
    /// `xsi:type` when present. In vim25 this — not the element name — is what
    /// identifies the concrete type of an array member.
    pub xsi_type: Option<String>,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub children: Vec<Element>,
}

impl Element {
    /// First direct child with this element name.
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }

    /// All direct children with this element name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Text of a descendant addressed by a slash-separated path of element names.
    pub fn text_at(&self, path: &str) -> Option<String> {
        let mut node = self;
        for seg in path.split('/') {
            node = node.child(seg)?;
        }
        Some(node.text.clone())
    }

    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Depth-first search for the first descendant with this name.
    pub fn find(&self, name: &str) -> Option<&Element> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(name))
    }
}

/// Parse a document into its root element.
pub fn parse(xml: &str) -> Result<Element, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut stack: Vec<Element> = vec![Element {
        name: "#document".into(),
        ..Default::default()
    }];
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => stack.push(element_from(&e, &reader)?),
            Ok(Event::Empty(e)) => {
                let el = element_from(&e, &reader)?;
                stack
                    .last_mut()
                    .expect("stack always holds the document root")
                    .children
                    .push(el);
            }
            Ok(Event::End(_)) => {
                if stack.len() > 1 {
                    let el = stack.pop().expect("checked len > 1");
                    stack
                        .last_mut()
                        .expect("stack always holds the document root")
                        .children
                        .push(el);
                }
            }
            Ok(Event::Text(t)) => {
                let s = t
                    .unescape()
                    .map_err(|e| format!("XML text decode failed: {e}"))?
                    .into_owned();
                stack
                    .last_mut()
                    .expect("stack always holds the document root")
                    .text
                    .push_str(&s);
            }
            Ok(Event::CData(t)) => {
                let s = String::from_utf8_lossy(&t).into_owned();
                stack.last_mut().expect("root").text.push_str(&s);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("XML parse error at byte {}: {e}", reader.buffer_position())),
        }
        buf.clear();
    }

    let mut root = stack.remove(0);
    if root.children.len() == 1 {
        Ok(root.children.remove(0))
    } else {
        Ok(root)
    }
}

fn element_from(
    e: &quick_xml::events::BytesStart,
    reader: &Reader<&[u8]>,
) -> Result<Element, String> {
    let _ = reader;
    // Namespace prefixes carry no information for us — vim25 responses use a
    // single default namespace — so strip them and match on local names.
    let raw = String::from_utf8_lossy(e.name().as_ref()).into_owned();
    let name = raw.rsplit(':').next().unwrap_or(&raw).to_string();

    let mut attrs = Vec::new();
    let mut xsi_type = None;
    for a in e.attributes() {
        let a = a.map_err(|err| format!("XML attribute error: {err}"))?;
        let key_raw = String::from_utf8_lossy(a.key.as_ref()).into_owned();
        let key = key_raw.rsplit(':').next().unwrap_or(&key_raw).to_string();
        let val = a
            .unescape_value()
            .map_err(|err| format!("XML attribute decode failed: {err}"))?
            .into_owned();
        if key == "type" && key_raw.starts_with("xsi:") {
            xsi_type = Some(val.clone());
        }
        attrs.push((key, val));
    }

    Ok(Element {
        name,
        xsi_type,
        attrs,
        text: String::new(),
        children: Vec::new(),
    })
}
