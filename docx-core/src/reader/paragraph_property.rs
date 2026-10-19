use std::io::Read;
use std::str::FromStr;

use super::*;

use super::attributes::*;
use crate::types::*;

impl ElementReader for ParagraphProperty {
    fn read<R: Read>(
        r: &mut EventReader<R>,
        attrs: &[OwnedAttribute],
    ) -> Result<Self, ReaderError> {
        let mut p = ParagraphProperty::new();
        loop {
            let e = r.next_event();
            match e {
                Ok(XmlEvent::StartElement {
                    attributes, name, ..
                }) => {
                    let e = XMLElement::from_str(&name.local_name).unwrap();
                    match e {
                        XMLElement::Indent => {
                            let (start, end, special, start_chars, hanging_chars, first_line_chars) =
                                read_indent(&attributes)?;
                            p = p.indent(start, special, end, start_chars);

                            if let Some(chars) = hanging_chars {
                                p = p.hanging_chars(chars);
                            }
                            if let Some(chars) = first_line_chars {
                                p = p.first_line_chars(chars);
                            }
                            continue;
                        }
                        XMLElement::Spacing => {
                            if let Ok(spacing) =
                                attributes::line_spacing::read_line_spacing(&attributes)
                            {
                                p = p.line_spacing(spacing);
                            }
                            continue;
                        }
                        XMLElement::Justification => {
                            if let Ok(v) = AlignmentType::from_str(&attributes[0].value) {
                                p = p.align(v);
                            }
                            continue;
                        }
                        XMLElement::TextAlignment => {
                            if let Ok(v) = TextAlignmentType::from_str(&attributes[0].value) {
                                p = p.text_alignment(v);
                            }
                            continue;
                        }
                        XMLElement::AdjustRightInd => {
                            if let Some(val) = read_val(&attributes) {
                                if let Ok(v) = isize::from_str(&val) {
                                    p = p.adjust_right_ind(v);
                                }
                            }
                            continue;
                        }
                        XMLElement::ParagraphStyle => {
                            p = p.style(&attributes[0].value);
                            continue;
                        }
                        XMLElement::RunProperty => {
                            if let Ok(run_pr) = RunProperty::read(r, attrs) {
                                p.run_property = run_pr;
                            }
                            continue;
                        }
                        XMLElement::DivId => {
                            if let Some(val) = read_val(&attributes) {
                                p.div_id = Some(val)
                            }
                            continue;
                        }
                        XMLElement::NumberingProperty => {
                            if let Ok(num_pr) = NumberingProperty::read(r, attrs) {
                                p = p.numbering_property(num_pr);
                            }
                            continue;
                        }
                        XMLElement::OutlineLvl => {
                            if let Some(val) = read_val(&attributes) {
                                if let Ok(val) = usize::from_str(&val) {
                                    p = p.outline_lvl(val);
                                }
                            }
                            continue;
                        }
                        XMLElement::SnapToGrid => {
                            let v = read_bool(&attributes);
                            p.snap_to_grid = Some(v);
                        }
                        XMLElement::KeepNext => {
                            if read_bool(&attributes) {
                                p.keep_next = Some(true);
                            }
                        }
                        XMLElement::KeepLines => {
                            if read_bool(&attributes) {
                                p.keep_lines = Some(true);
                            }
                        }
                        XMLElement::PageBreakBefore => {
                            if read_bool(&attributes) {
                                p.page_break_before = Some(true);
                            }
                        }
                        XMLElement::WidowControl => {
                            if read_bool(&attributes) {
                                p.widow_control = Some(true);
                            }
                        }
                        XMLElement::WordWrap => {
                            // Recorded either way, unlike the flags above it:
                            // `w:val="0"` is the meaningful setting, asking for
                            // character-level breaking of East Asian text.
                            // `read_bool` already yields true for a bare
                            // `<w:wordWrap/>`, which is what OOXML means by it.
                            p.word_wrap = Some(read_bool(&attributes));
                        }
                        XMLElement::ParagraphPropertyChange => {
                            if let Ok(ppr_change) = ParagraphPropertyChange::read(r, &attributes) {
                                p.paragraph_property_change = Some(ppr_change);
                            }
                        }
                        XMLElement::SectionProperty => {
                            if let Ok(sp) = SectionProperty::read(r, &attributes) {
                                p.section_property = Some(sp);
                            }
                        }
                        XMLElement::FrameProperty => {
                            if let Ok(pr) = FrameProperty::read(r, &attributes) {
                                p.frame_property = Some(pr);
                            }
                        }
                        XMLElement::Shading => {
                            if let Ok(shd) = Shading::read(r, &attributes) {
                                p.shading = Some(shd);
                            }
                        }
                        XMLElement::ParagraphBorders => {
                            if let Ok(borders) = ParagraphBorders::read(r, &attributes) {
                                p = p.set_borders(borders);
                            }
                        }
                        XMLElement::Tabs => {
                            if let Ok(tabs) = Tabs::read(r, &attributes) {
                                for t in tabs.tabs {
                                    p = p.add_tab(t);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Ok(XmlEvent::EndElement { name, .. }) => {
                    let e = XMLElement::from_str(&name.local_name).unwrap();
                    if e == XMLElement::ParagraphProperty {
                        return Ok(p);
                    }
                }
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
    use std::io::Cursor;

    #[test]
    fn test_read_word_wrap() {
        // `w:val="0"` asks for character-level breaking of East Asian text.
        // It has to survive as `Some(false)`: dropping it is indistinguishable
        // from the property being absent (office2pdf#730).
        let xml = r#"<w:pPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:wordWrap w:val="0"/>
        </w:pPr>"#;
        let mut parser = EventReader::new(Cursor::new(xml));
        loop {
            if let Some(Ok(XmlEvent::StartElement { name, .. })) = parser.next() {
                if name.local_name == "pPr" {
                    break;
                }
            }
        }
        let p = ParagraphProperty::read(&mut parser, &[]).unwrap();
        assert_eq!(p.word_wrap, Some(false));
    }

    /// A bare `<w:wordWrap/>` means word-level breaking, and an absent one
    /// leaves the choice to the style chain — the two must not collapse.
    #[test]
    fn test_read_word_wrap_defaults() {
        let read = |body: &str| {
            let xml = format!(
                r#"<w:pPr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{body}</w:pPr>"#
            );
            let mut parser = EventReader::new(Cursor::new(xml));
            loop {
                if let Some(Ok(XmlEvent::StartElement { name, .. })) = parser.next() {
                    if name.local_name == "pPr" {
                        break;
                    }
                }
            }
            ParagraphProperty::read(&mut parser, &[]).unwrap().word_wrap
        };
        assert_eq!(read(r#"<w:wordWrap/>"#), Some(true));
        assert_eq!(read(r#"<w:wordWrap w:val="1"/>"#), Some(true));
        assert_eq!(read(""), None);
    }
}
