#![allow(clippy::single_match)]

use std::io::Read;
use std::str::FromStr;

use super::Run;

use crate::escape::replace_escaped;
use crate::types::BreakType;
use crate::{reader::*, FieldCharType};

#[derive(PartialEq, Debug)]
enum TextState {
    Idle,
    Text,
    Delete,
}

fn read_field_char(attributes: &[OwnedAttribute]) -> Result<FieldChar, ReaderError> {
    let mut t: Option<FieldCharType> = None;
    let mut dirty = false;
    for a in attributes {
        let local_name = &a.name.local_name;
        match local_name.as_str() {
            "fldCharType" => {
                if let Ok(ty) = FieldCharType::from_str(&a.value) {
                    t = Some(ty);
                }
            }
            "dirty" => {
                dirty = !is_false(&a.value);
            }
            _ => {}
        }
    }

    if let Some(t) = t {
        let mut f = FieldChar::new(t);
        if dirty {
            f = f.dirty();
        }
        Ok(f)
    } else {
        Err(ReaderError::XMLReadError)
    }
}

// Namespace URIs identify run vocabularies; prefixes are arbitrary and scoped.
// Keep undeclared conventional prefixes for legacy XML-fragment callers.
fn has_namespace(name: &OwnedName, namespaces: &[&str], legacy_prefix: &str) -> bool {
    match name.namespace.as_deref() {
        Some(namespace) => namespaces.contains(&namespace),
        None => name.prefix.as_deref() == Some(legacy_prefix),
    }
}

pub(super) fn is_word_name(name: &OwnedName) -> bool {
    has_namespace(
        name,
        &[
            "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
            "http://purl.oclc.org/ooxml/wordprocessingml/main",
        ],
        "w",
    )
}

impl ElementReader for Run {
    fn read<R: Read>(
        r: &mut EventReader<R>,
        _attrs: &[OwnedAttribute],
    ) -> Result<Self, ReaderError> {
        let mut run = Run::new();
        let mut text_state = TextState::Idle;
        loop {
            let e = r.next_event();
            match e {
                Ok(XmlEvent::StartElement {
                    attributes, name, ..
                }) => {
                    match () {
                        _ if is_word_name(&name) => {
                            let e = XMLElement::from_str(&name.local_name).unwrap();

                            ignore::ignore_element(e.clone(), XMLElement::RunPropertyChange, r);

                            match e {
                                XMLElement::Tab => {
                                    run = run.add_tab();
                                }
                                XMLElement::PTab => {
                                    if let Ok(v) = PositionalTab::read(r, &attributes) {
                                        run = run.add_ptab(v);
                                    }
                                }
                                XMLElement::Sym => {
                                    if let Some(font) = read(&attributes, "font") {
                                        if let Some(char) = read(&attributes, "char") {
                                            let sym = Sym::new(font, char);
                                            run = run.add_sym(sym);
                                        }
                                    }
                                }
                                XMLElement::RunProperty => {
                                    let p = RunProperty::read(r, &attributes)?;
                                    run = run.set_property(p);
                                }
                                XMLElement::FootnoteReference => {
                                    // Namespace aliases are valid; foreign attributes with the
                                    // same local name must not alter reference semantics.
                                    let word_attribute = |local_name: &str| {
                                        attributes
                                            .iter()
                                            .find(|attribute| {
                                                attribute.name.local_name == local_name
                                                    && (is_word_name(&attribute.name)
                                                        || (attribute.name.namespace.is_none()
                                                            && attribute.name.prefix.is_none()))
                                            })
                                            .map(|attribute| attribute.value.as_str())
                                    };
                                    if let Some(id) = word_attribute("id")
                                        .and_then(|value| value.parse::<usize>().ok())
                                    {
                                        let mut reference = FootnoteReference::new(id);
                                        if let Some(value) = word_attribute("customMarkFollows") {
                                            reference =
                                                reference.custom_mark_follows(!is_false(value));
                                        }
                                        run.children.push(RunChild::FootnoteReference(reference));
                                    }
                                }
                                XMLElement::Text => text_state = TextState::Text,
                                XMLElement::DeleteText => text_state = TextState::Delete,
                                XMLElement::Break => {
                                    if let Some(attribute) = attributes.iter().find(|attribute| {
                                        attribute.name.local_name == "type"
                                            && (is_word_name(&attribute.name)
                                                || (attribute.name.namespace.is_none()
                                                    && attribute.name.prefix.is_none()))
                                    }) {
                                        run = run.add_break(BreakType::from_str(&attribute.value)?)
                                    } else {
                                        run = run.add_break(BreakType::TextWrapping)
                                    }
                                }
                                XMLElement::CarriageReturn => {
                                    run = run.add_carriage_return();
                                }
                                XMLElement::NoBreakHyphen => {
                                    run = run.add_no_break_hyphen();
                                }
                                XMLElement::SoftHyphen => {
                                    run = run.add_soft_hyphen();
                                }
                                XMLElement::Drawing => {
                                    if let Ok(drawing) = Drawing::read(r, &attributes) {
                                        run = run.add_drawing(drawing);
                                    }
                                }
                                XMLElement::FieldChar => {
                                    if let Ok(f) = read_field_char(&attributes) {
                                        run.children.push(RunChild::FieldChar(f));
                                    }
                                }
                                XMLElement::InstrText => loop {
                                    let e = r.next_event();
                                    match e {
                                        Ok(XmlEvent::Characters(c)) => {
                                            run.children.push(RunChild::InstrTextString(c));
                                            break;
                                        }
                                        Ok(XmlEvent::EndElement { name, .. })
                                            if is_word_name(&name) =>
                                        {
                                            let e = XMLElement::from_str(&name.local_name).unwrap();
                                            match e {
                                                XMLElement::Run => {
                                                    return Ok(run);
                                                }
                                                _ => {}
                                            }
                                        }
                                        Ok(XmlEvent::EndDocument) => {
                                            return Err(ReaderError::XMLReadError)
                                        }
                                        Err(_) => return Err(ReaderError::XMLReadError),
                                        _ => {}
                                    }
                                },
                                _ => {}
                            }
                        }
                        _ if has_namespace(
                            &name,
                            &["http://schemas.openxmlformats.org/markup-compatibility/2006"],
                            "mc",
                        ) =>
                        {
                            let e = McXMLElement::from_str(&name.local_name).unwrap();
                            match e {
                                McXMLElement::Fallback => {
                                    let _ = McFallback::read(r, &attributes)?;
                                }
                                _ => {}
                            }
                        }
                        _ if has_namespace(&name, &["urn:schemas-microsoft-com:vml"], "v") => {
                            let e = VXMLElement::from_str(&name.local_name).unwrap();
                            match e {
                                // Experimental For now support only imageData in shape
                                VXMLElement::Shape => {
                                    if let Ok(shape) = Shape::read(r, &attributes) {
                                        run.children.push(RunChild::Shape(Box::new(shape)));
                                    }
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    };
                }
                Ok(XmlEvent::Characters(c)) => match text_state {
                    TextState::Delete => {
                        run = run.add_delete_text_without_escape(replace_escaped(&c));
                    }
                    TextState::Text => {
                        run = run.add_text_without_escape(replace_escaped(&c));
                    }
                    _ => {}
                },
                Ok(XmlEvent::Whitespace(c)) => match text_state {
                    TextState::Delete => {
                        run = run.add_delete_text_without_escape(replace_escaped(&c));
                    }
                    TextState::Text => {
                        run = run.add_text_without_escape(replace_escaped(&c));
                    }
                    _ => {}
                },
                Ok(XmlEvent::EndElement { name, .. }) if is_word_name(&name) => {
                    let e = XMLElement::from_str(&name.local_name).unwrap();
                    match e {
                        XMLElement::Run => {
                            return Ok(run);
                        }
                        XMLElement::DeleteText | XMLElement::Text => text_state = TextState::Idle,
                        _ => {}
                    }
                }
                Ok(XmlEvent::EndDocument) => return Err(ReaderError::XMLReadError),
                Err(_) => return Err(ReaderError::XMLReadError),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    #[cfg(test)]
    use pretty_assertions::assert_eq;

    #[test]
    fn hyphen_runs_keep_nonbreaking_and_discretionary_positions() {
        let xml = r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:t>long</w:t><w:noBreakHyphen/><w:t>term co</w:t><w:softHyphen/><w:t>operate</w:t><w:noBreakHyphen/></w:r>"#;
        let run = Run::read(&mut EventReader::new(xml.as_bytes()), &[]).unwrap();
        let children = serde_json::to_value(&run).unwrap()["children"].clone();
        assert_eq!(
            children,
            serde_json::json!([
                {"type":"text", "data":{"text":"long", "preserveSpace":true}},
                {"type":"noBreakHyphen"},
                {"type":"text", "data":{"text":"term co", "preserveSpace":true}},
                {"type":"softHyphen"},
                {"type":"text", "data":{"text":"operate", "preserveSpace":true}},
                {"type":"noBreakHyphen"}
            ])
        );
    }

    #[test]
    fn hyphen_runs_follow_namespace_uris() {
        for namespace in [
            "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
            "http://purl.oclc.org/ooxml/wordprocessingml/main",
        ] {
            for prefix in ["w", "text", ""] {
                let colon = if prefix.is_empty() { "" } else { ":" };
                let declaration = if prefix.is_empty() {
                    "xmlns".to_string()
                } else {
                    format!("xmlns:{prefix}")
                };
                let xml = format!(
                    r#"<{prefix}{colon}p {declaration}="{namespace}"><{prefix}{colon}r><{prefix}{colon}noBreakHyphen/><{prefix}{colon}t>e</{prefix}{colon}t><{prefix}{colon}noBreakHyphen/><{prefix}{colon}t>mail</{prefix}{colon}t><{prefix}{colon}softHyphen/></{prefix}{colon}r></{prefix}{colon}p>"#
                );
                let paragraph =
                    Paragraph::read(&mut EventReader::new(xml.as_bytes()), &[]).unwrap();
                assert_eq!(
                    paragraph.raw_text(),
                    "\u{2011}e\u{2011}mail\u{00ad}",
                    "{namespace}, {prefix}"
                );
            }
        }
    }

    #[test]
    fn hyphen_runs_round_trip_as_elements() {
        use crate::documents::BuildXML;

        let xml = r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:t>well</w:t><w:noBreakHyphen/><w:t>known</w:t><w:softHyphen/></w:r>"#;
        let run = Run::read(&mut EventReader::new(xml.as_bytes()), &[]).unwrap();
        let output = String::from_utf8(run.build()).unwrap();
        assert!(output.contains("<w:noBreakHyphen />"), "{output}");
        assert!(output.contains("<w:softHyphen />"), "{output}");
        let wrapped = format!(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{output}</w:document>"#
        );
        assert_eq!(
            Run::read(&mut EventReader::new(wrapped.as_bytes()), &[]).unwrap(),
            run
        );
    }

    #[test]
    fn hyphen_runs_ignore_foreign_lookalikes() {
        let xml = r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:f="urn:metadata"><w:t>metadata</w:t><f:noBreakHyphen/><f:softHyphen/><w:t>only</w:t></w:r>"#;
        let run = Run::read(&mut EventReader::new(xml.as_bytes()), &[]).unwrap();
        assert_eq!(run, Run::new().add_text("metadata").add_text("only"));
    }

    #[test]
    fn read_word_containers_ignore_math_and_foreign_runs() {
        for container in ["p", "hyperlink", "ins", "del", "moveFrom", "moveTo", "sdt"] {
            for prefix in ["w", "text"] {
                let read_container =
                    |xml: &str| -> Result<serde_json::Value, ReaderError> {
                        let mut reader = EventReader::new(xml.as_bytes());
                        match container {
                            "p" => Paragraph::read(&mut reader, &[])
                                .map(|v| serde_json::to_value(v).unwrap()),
                            "hyperlink" => Hyperlink::read(&mut reader, &[])
                                .map(|v| serde_json::to_value(v).unwrap()),
                            "ins" => Insert::read(&mut reader, &[])
                                .map(|v| serde_json::to_value(v).unwrap()),
                            "del" => Delete::read(&mut reader, &[])
                                .map(|v| serde_json::to_value(v).unwrap()),
                            "moveFrom" => MoveFrom::read(&mut reader, &[])
                                .map(|v| serde_json::to_value(v).unwrap()),
                            "moveTo" => MoveTo::read(&mut reader, &[])
                                .map(|v| serde_json::to_value(v).unwrap()),
                            "sdt" => StructuredDataTag::read(&mut reader, &[])
                                .map(|v| serde_json::to_value(v).unwrap()),
                            _ => unreachable!(),
                        }
                    };
                let before =
                    format!("<{prefix}:r><{prefix}:t>Before equation</{prefix}:t></{prefix}:r>");
                let after =
                    format!("<{prefix}:r><{prefix}:t>After equation</{prefix}:t></{prefix}:r>");
                let wrap = |middle: &str| {
                    format!(
                        r#"<{prefix}:{container} xmlns:{prefix}="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:f="urn:metadata">{before}{middle}{after}</{prefix}:{container}>"#
                    )
                };
                let control = read_container(&wrap("")).unwrap();
                let math = r#"<m:oMath><m:r><m:t>E=mc</m:t></m:r><m:sSup><m:e><m:r><m:t>x</m:t></m:r></m:e><m:sup><m:r><m:t>2</m:t></m:r></m:sup></m:sSup></m:oMath><f:r><f:t>Metadata</f:t></f:r>"#;
                let tail = wrap("").replace(
                    &format!("</{prefix}:{container}>"),
                    &format!("{math}</{prefix}:{container}>"),
                );
                for xml in [wrap(math), tail] {
                    let actual = read_container(&xml).unwrap();
                    assert_eq!(actual, control, "container {container}, prefix {prefix}");
                }
            }
        }
    }

    #[test]
    fn read_run_keeps_page_breaks_with_local_namespace_declarations() {
        for prefix in ["text", "type"] {
            let xml = format!(
                r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:t>Before</w:t><{prefix}:br xmlns:{prefix}="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:f="urn:metadata" f:type="column" {prefix}:clear="all" {prefix}:type="page"/><w:t>After</w:t></w:r>"#
            );
            let mut reader = EventReader::new(xml.as_bytes());
            let run = Run::read(&mut reader, &[]).unwrap();
            assert_eq!(
                run,
                Run::new()
                    .add_text("Before")
                    .add_break(BreakType::Page)
                    .add_text("After")
            );
        }
    }

    #[test]
    fn read_run_local_declarations_do_not_replace_property_values() {
        let xml = r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:rPr><val:b xmlns:val="http://schemas.openxmlformats.org/wordprocessingml/2006/main" val:val="0"/></w:rPr><w:t>Regular report text</w:t></w:r>"#;
        let mut reader = EventReader::new(xml.as_bytes());
        assert_eq!(
            Run::read(&mut reader, &[]).unwrap(),
            Run::new().disable_bold().add_text("Regular report text")
        );
    }

    #[test]
    fn read_run_uses_wordprocessing_namespace_with_any_prefix() {
        for (opening, prefix) in [
            (
                "xmlns:ns0=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"",
                "ns0:",
            ),
            (
                "xmlns=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"",
                "",
            ),
            (
                "xmlns:text=\"http://purl.oclc.org/ooxml/wordprocessingml/main\"",
                "text:",
            ),
        ] {
            let xml = format!("<{prefix}r {opening}><{prefix}rPr><{prefix}b/></{prefix}rPr><{prefix}t>Quarterly report</{prefix}t><{prefix}tab/><{prefix}t>Approved</{prefix}t></{prefix}r>");
            let mut reader = EventReader::new(xml.as_bytes());
            let run = Run::read(&mut reader, &[]).unwrap();
            let expected = Run::new()
                .bold()
                .add_text("Quarterly report")
                .add_tab()
                .add_text("Approved");
            assert_eq!(run, expected, "prefix {prefix}");
        }
    }

    #[test]
    fn read_run_ignores_foreign_names_and_restores_scoped_bindings() {
        let xml = r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:f="urn:foreign">
          <w:t>Before</w:t><f:r><f:t>Foreign</f:t></f:r>
          <w:t xmlns:w="urn:foreign">Shadowed</w:t>
          <w:t>After</w:t>
        </w:r>"#;
        let mut reader = EventReader::new(xml.as_bytes());
        let run = Run::read(&mut reader, &[]).unwrap();
        assert_eq!(run, Run::new().add_text("Before").add_text("After"));
    }

    #[test]
    fn read_run_rejects_an_unrecognized_root_without_looping_at_eof() {
        let xml = r#"<f:r xmlns:f="urn:metadata"><f:t>Metadata</f:t></f:r>"#;
        let mut reader = EventReader::new(xml.as_bytes());
        assert!(Run::read(&mut reader, &[]).is_err());
    }

    #[test]
    fn test_read_footnote_reference_without_reference_style() {
        for (id, style) in [(7, ""), (29, "<w:rStyle w:val=\"CustomReference\"/>")] {
            let xml = format!(
                r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:rPr>{style}<w:b/></w:rPr><w:t>Before</w:t><w:footnoteReference w:customMarkFollows="0" w:id="{id}"/><w:t>After</w:t></w:r>"#
            );
            let run = Run::read(&mut EventReader::new(xml.as_bytes()), &[]).unwrap();
            assert_eq!(
                run.children,
                vec![
                    RunChild::Text(Text::new("Before")),
                    RunChild::FootnoteReference(
                        FootnoteReference::new(id).custom_mark_follows(false)
                    ),
                    RunChild::Text(Text::new("After"))
                ]
            );
            assert_eq!(run.run_property.bold, Some(Bold::new()));
        }
    }

    #[test]
    fn test_read_footnote_reference_retains_custom_mark_on_off() {
        for (value, expected) in [
            ("1", true),
            ("true", true),
            ("on", true),
            ("0", false),
            ("false", false),
            ("off", false),
        ] {
            let xml = format!(
                r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:t>Before</w:t><w:footnoteReference w:customMarkFollows="{value}" w:id="7"/><w:t>**</w:t></w:r>"#
            );
            let run = Run::read(&mut EventReader::new(xml.as_bytes()), &[]).unwrap();
            assert_eq!(run.children.len(), 3);
            let RunChild::FootnoteReference(reference) = &run.children[1] else {
                panic!("reference stays between its text siblings")
            };
            let value = serde_json::to_value(reference).unwrap();
            assert_eq!(value["customMarkFollows"], expected);
            assert_eq!(reference.id, 7);
            assert_eq!(run.children[2], RunChild::Text(Text::new("**")));
        }
    }

    #[test]
    fn test_read_footnote_reference_custom_mark_uses_word_namespace() {
        for (attributes, expected) in [
            (r#"q:customMarkFollows="on""#, Some(true)),
            (
                r#"x:customMarkFollows="on" q:customMarkFollows="off""#,
                Some(false),
            ),
            (r#"x:customMarkFollows="on""#, None),
            (r#"customMarkFollows="on""#, Some(true)),
        ] {
            let xml = format!(
                r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:q="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:x="urn:foreign"><w:footnoteReference {attributes} q:id="29"/></w:r>"#
            );
            let run = Run::read(&mut EventReader::new(xml.as_bytes()), &[]).unwrap();
            let RunChild::FootnoteReference(reference) = &run.children[0] else {
                panic!("reference")
            };
            let value = serde_json::to_value(reference).unwrap();
            assert_eq!(
                value.get("customMarkFollows").cloned(),
                expected.map(serde_json::Value::Bool)
            );
        }
    }

    #[test]
    fn test_read_custom_footnote_mark_with_aliased_and_strict_elements() {
        for namespace in [
            "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
            "http://purl.oclc.org/ooxml/wordprocessingml/main",
        ] {
            let xml = format!(
                r#"<p:r xmlns:p="{namespace}" xmlns:w="urn:foreign"><p:footnoteReference w:id="99" p:id="29" w:customMarkFollows="on" p:customMarkFollows="off"/><p:t>**</p:t></p:r>"#
            );
            let run = Run::read(&mut EventReader::new(xml.as_bytes()), &[]).unwrap();
            assert_eq!(
                run.children,
                vec![
                    RunChild::FootnoteReference(
                        FootnoteReference::new(29).custom_mark_follows(false)
                    ),
                    RunChild::Text(Text::new("**")),
                ]
            );
        }
    }

    #[test]
    fn test_read_footnote_reference_preserves_surrounding_text_with_invalid_id() {
        for attributes in ["", "w:id=\"invalid\"", "w:id=\"-1\""] {
            let xml = format!(
                r#"<w:r xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:t>Before</w:t><w:footnoteReference {attributes}/><w:t>After</w:t></w:r>"#
            );
            let run = Run::read(&mut EventReader::new(xml.as_bytes()), &[]).unwrap();
            assert_eq!(
                run.children,
                vec![
                    RunChild::Text(Text::new("Before")),
                    RunChild::Text(Text::new("After"))
                ]
            );
        }
    }

    #[test]
    fn test_read_size_color() {
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:rPr><w:color w:val="C9211E"/><w:sz w:val="30"/><w:szCs w:val="30"/></w:rPr><w:t>H</w:t></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![RunChild::Text(Text::new("H"))],
                run_property: RunProperty {
                    sz: Some(Sz::new(30)),
                    sz_cs: Some(SzCs::new(30)),
                    color: Some(Color::new("C9211E")),
                    ..RunProperty::default()
                },
            }
        );
    }

    #[test]
    fn test_read_theme_color() {
        // The reader must pick attributes by name (not position) and round-trip
        // w:themeColor / w:themeShade alongside the w:val fallback.
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:rPr><w:color w:val="2E74B5" w:themeColor="accent1" w:themeShade="BF"/></w:rPr><w:t>H</w:t></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![RunChild::Text(Text::new("H"))],
                run_property: RunProperty {
                    color: Some(
                        Color::new("2E74B5")
                            .theme_color(crate::ThemeColor::Accent1)
                            .theme_shade("BF")
                    ),
                    ..RunProperty::default()
                },
            }
        );
    }

    #[test]
    fn test_read_tab() {
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:tab /></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![RunChild::Tab(Tab::new())],
                run_property: RunProperty::default(),
            }
        );
    }

    #[test]
    fn test_read_br() {
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:br w:type="page" /></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![RunChild::Break(Break::new(BreakType::Page))],
                run_property: RunProperty::default(),
            }
        );
    }

    #[test]
    fn test_read_empty_br() {
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:br /></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![RunChild::Break(Break::new(BreakType::TextWrapping))],
                run_property: RunProperty::default(),
            }
        );
    }

    #[test]
    fn test_read_cr() {
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:cr /></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![RunChild::CarriageReturn(CarriageReturn::new())],
                run_property: RunProperty::default(),
            }
        );
    }

    #[test]
    fn test_read_hyphens() {
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:t>long</w:t><w:noBreakHyphen/><w:t>term</w:t><w:softHyphen/></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![
                    RunChild::Text(Text::new("long")),
                    RunChild::NoBreakHyphen(NoBreakHyphen::new()),
                    RunChild::Text(Text::new("term")),
                    RunChild::SoftHyphen(SoftHyphen::new()),
                ],
                run_property: RunProperty::default(),
            }
        );
    }

    #[test]
    fn test_read_italic_false() {
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:rPr>
    <w:b w:val="true"/>
    <w:i w:val="false"/>
  </w:rPr></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![],
                run_property: RunProperty {
                    bold: Some(Bold::new()),
                    bold_cs: Some(BoldCs::new()),
                    italic: Some(Italic::new().disable()),
                    italic_cs: Some(ItalicCs::new().disable()),
                    ..RunProperty::default()
                },
            }
        );
    }

    #[test]
    fn test_read_italic_0() {
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:rPr>
    <w:b w:val="1"/>
    <w:i w:val="0"/>
  </w:rPr></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![],
                run_property: RunProperty {
                    bold: Some(Bold::new()),
                    bold_cs: Some(BoldCs::new()),
                    italic: Some(Italic::new().disable()),
                    italic_cs: Some(ItalicCs::new().disable()),
                    ..RunProperty::default()
                },
            }
        );
    }

    #[test]
    fn test_read_on_off_values() {
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:rPr>
    <w:b w:val="off"/>
    <w:i w:val="on"/>
  </w:rPr></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![],
                run_property: RunProperty {
                    bold: Some(Bold::new().disable()),
                    bold_cs: Some(BoldCs::new().disable()),
                    italic: Some(Italic::new()),
                    italic_cs: Some(ItalicCs::new()),
                    ..RunProperty::default()
                },
            }
        );
    }

    #[test]
    fn test_read_fit_text() {
        let c = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:r><w:rPr>
    <w:fitText w:val="840" w:id="1266434317"/>
  </w:rPr></w:r>
</w:document>"#;
        let mut parser = EventReader::new(c.as_bytes());
        let run = Run::read(&mut parser, &[]).unwrap();
        assert_eq!(
            run,
            Run {
                children: vec![],
                run_property: RunProperty {
                    fit_text: Some(FitText::new(840).id(1266434317)),
                    ..RunProperty::default()
                },
            }
        );
    }
}
