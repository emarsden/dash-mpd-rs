//! Support for the STPP subtitle format
//
// This module provides support for TTML (Timed Text Markup Language) subtitles that are encoded
// using the STPP codec and packed in fragmented MP4 segments. These subtitles are provided as a
// separate media stream of fMP4 segments, that the media player retrieves incrementally.
//
// This module implements:
//
//  - extracting the XML-formatted TTML fragments from an MP4 fragment
//
//  - parsing the TTML fragment to extract <style>, <region> and <p> elements, appending them to the
//  StppDocument object
//
//  - serializing to a single merged TTML subtitle file
//
// We only support the text-only IMSC1 profile for TTML subtitles ("stpp.ttml.im1t"); the image-only
// profile ("stpp.ttml.im1i") is not supported.
//
// An example of the XML content in a TTML/STPP fragment:
//
// <?xml version="1.0" encoding="utf-8"?>
// <tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata"
//     xmlns:tts="http://www.w3.org/ns/ttml#styling" xml:lang="fr">
//   <head>
//     <metadata><ttm:title></ttm:title><ttm:desc></ttm:desc><ttm:copyright></ttm:copyright></metadata>
//     <styling>
//       <style xml:id="basic" tts:backgroundColor="transparent" tts:color="white"
//              tts:fontFamily="proportionalSansSerif" tts:fontSize="16px" tts:textAlign="center" />
//     </styling>
//     <layout>
//       <region style="basic" xml:id="speaker" tts:displayAlign="center" tts:extent="80% 10%"
//              tts:origin="10% 85%" />
//     </layout>
//   </head>
//   <body>
//     <div xml:lang="fr">
//       <p begin="00:03:44.500" end="00:03:45.700" region="speaker">Rien d&apos;inquiétant.</p>
//       <p begin="00:03:46.000" end="00:03:47.000" region="speaker">Thom.</p>
//       <p begin="00:03:51.000" end="00:03:52.000" region="speaker">Elle est là.</p>
//     </div>
//   </body>
// </tt>
//
// Here an example fMP4 segment that contains TTML data
//  https://demo.unified-streaming.com/k8s/features/stable/video/tears-of-steel/tears-of-steel-ttml.ism/dash/tears-of-steel-ttml-textstream_fra=1000-52.m4s
//
// References:
//   https://en.wikipedia.org/wiki/Timed_Text_Markup_Language
//   https://www.w3.org/TR/ttml-imsc1.0.1/


use std::io::Cursor;
use xot::{Xot, output};
use xot::xmlname::NameStrInfo;
use xmlparser::{ElementEnd, Token, Tokenizer};
use tracing::{trace, warn, error};
use bytes::Bytes;
use crate::DashMpdError;


#[derive(Clone, Debug)]
pub struct StppDocument {
    xot: Xot,
    styles: Vec<xot::Node>,
    regions: Vec<xot::Node>,
    // A paragraph is a single subtitle cue.
    paragraphs: Vec<xot::Node>,
    warned_binary_contents: bool,
}

impl Default for StppDocument {
    fn default() -> Self {
        Self::new()
    }
}

// We need to extract and new styles and layouts and append them to the head of the final XML file,
// and also append all the <p> tags for the final XML.
impl StppDocument {
    #[must_use]
    pub fn new() -> StppDocument {
        StppDocument {
            xot: Xot::new(),
            styles: Vec::new(),
            regions: Vec::new(),
            paragraphs: Vec::new(),
            warned_binary_contents: false,
        }
    }


    // Extract XML content from a fragmented MP4 segment (argument bytes) and add its contents to
    // the content accumulated in the parent StppDocument. The content is typically in an mdat box,
    // or sometimes an stpp box.
    //
    // Decode boxes present in an fMP4 segment: https://media-analyzer.pro/analyzer
    pub fn add_from_mp4(&mut self, bytes: &Bytes) -> Result<(), DashMpdError> {
        use mp4_atom::ReadFrom;

        let mut buf = Cursor::new(bytes);
        loop {
            match Option::<mp4_atom::Any>::read_from(&mut buf) {
                Ok(maybe_atom) => {
                    match maybe_atom {
                        Some(mp4_atom::Any::Stpp(_stpp)) => (),
                        Some(mp4_atom::Any::Mdat(mdat)) => if let Ok(xml) = str::from_utf8(&mdat.data) {
                            self.add_content(xml)?;
                        },
                        Some(_) => (),
                        None => break,
                    }
                },
                Err(e) => warn!("Malformed MP4 box: {e:?}"),
            }
        }
        Ok(())
    }

    // Extract XML content from the binary data in bytes.
    pub fn add_bytes(&mut self, bytes: &Bytes) -> Result<(), DashMpdError> {
        if let Ok(xml) = str::from_utf8(bytes) {
            self.add_content(xml)?;
        } else {
            // This could be using the image-only profile, which we can't handle. Make sure we only
            // display this warning a single time for each subtitle stream.
            if !self.warned_binary_contents {
                warn!("Ignoring invalid XML in STPP subs: {}", String::from_utf8_lossy(bytes));
                self.warned_binary_contents = true;
            }
        }
        Ok(())
    }

    fn find_child_named(&mut self, node: xot::Node, name: &str) -> Option<xot::Node> {
        self.xot.children(node)
            .find(|n| self.xot.node_name_ref(*n)
                  .is_ok_and(
                      |nn| nn.is_some_and(
                          |nnn| nnn.local_name().eq(name))))
    }


    // Parse the TTML XML in xml and add its contents to the content accumulated in the parent
    // StppDocument. TTML fragments will often contain redundant style and region elements to allow
    // a media player to jump to a random point in the subtitle stream without requiring it to load
    // all the previous subtitle segments. We filter these out.
    pub fn add_content(&mut self, xml: &str) -> Result<(), DashMpdError> {
        trace!("adding STPP content {xml}");
        let mut clean_xml = xml;
        let epos = identify_xml_endpos(xml)
            .map_err(|_| DashMpdError::Parsing(String::from("calculating XML endpos")))?;
        if epos < xml.len() {
            clean_xml = &clean_xml[0..epos];
        }
        let root = self.xot.parse(clean_xml)
            .map_err(|e| {
                error!("Failure parsing STPP XML: {e:?}");
                error!("Failing XML: {xml}");
                DashMpdError::Parsing(String::from("parsing STPP XML"))
            })?;
        let tt = self.xot.document_element(root)
            .map_err(|_| DashMpdError::Parsing(String::from("extracting STPP XML root")))?;
        if !self.xot.element(tt).is_some_and(
            |n| self.xot.name_ns_str(n.name()).0.eq("tt")) {
            warn!("Missing tt root element in STPP XML: {xml}");
            return Ok(());
        }
        let xml_ns = self.xot.add_namespace("http://www.w3.org/XML/1998/namespace");
        let id_name = self.xot.add_name_ns("id", xml_ns);
        if let Some(head) = self.find_child_named(tt, "head") {
            for d in self.xot.descendants(head) {
                if self.xot.element(d).is_some_and(|n| self.xot.name_ns_str(n.name()).0.eq("style")) {
                    // Only add the style if it's not already defined (filter based on xml:id attribute)
                    if let Some(new_id) = self.xot.attributes(d).get(id_name) {
                        if !self.styles.iter().any(|s| self.xot.attributes(*s)
                                                   .get(id_name)
                                                   .is_some_and(|id| id.eq(new_id))) {
                            self.styles.push(d);
                        }
                    } else {
                        // don't attempt to dedup if there is no @id
                        self.styles.push(d);
                    }
                }
            }
            for d in self.xot.descendants(head) {
                if self.xot.element(d).is_some_and(|n| self.xot.name_ns_str(n.name()).0.eq("region")) {
                    if let Some(new_id) = self.xot.attributes(d).get(id_name) {
                        if !self.regions.iter().any(|s| self.xot.attributes(*s)
                                                    .get(id_name).is_some_and(|id| id.eq(new_id))) {
                            self.regions.push(d);
                        }
                    } else {
                        // don't attempt to dedup if there is no @id
                        self.regions.push(d);
                    }
                }
            }
        }
        if let Some(body) = self.find_child_named(tt, "body") {
            // Add all children of the body: these might be <div> nodes or <p> nodes.
            for d in self.xot.children(body) {
                self.paragraphs.push(d);
            }
        }
        Ok(())
    }

    // Generate a complete TTML document corresponding to the merge of all the fragments seen so
    // far. Note that we can't implement this using the fmt::Display trait for StppDocument, because
    // we need a mutable reference to self, which is not available for Display.
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&mut self) -> String {
        let empty_xml = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xmlns:tts="http://www.w3.org/ns/ttml#styling" xmlns:xml="http://www.w3.org/XML/1998/namespace" xml:lang="fr"></tt>"#;
        let ttml_ns = self.xot.add_namespace("http://www.w3.org/ns/ttml");
        let root = self.xot.parse(empty_xml).unwrap();
        let tt = self.xot.document_element(root).unwrap();
        let head_name = self.xot.add_name_ns("head", ttml_ns);
        let head = self.xot.new_element(head_name);
        self.xot.create_missing_prefixes(head).unwrap();
        let _ = self.xot.append(tt, head);
        let body_name = self.xot.add_name_ns("body", ttml_ns);
        let body = self.xot.new_element(body_name);
        self.xot.create_missing_prefixes(body).unwrap();
        let _ = self.xot.append(tt, body);
        let div_name = self.xot.add_name_ns("div", ttml_ns);
        let div = self.xot.new_element(div_name);
        let _ = self.xot.append(body, div);
        let styling_name = self.xot.add_name_ns("styling", ttml_ns);
        let styling = self.xot.new_element(styling_name);
        self.xot.create_missing_prefixes(styling).unwrap();
        let _ = self.xot.append(head, styling);
        for s in &self.styles {
            let new = self.xot.clone_with_prefixes(*s);
            let _ = self.xot.append(styling, new);
        }
        let layout_name = self.xot.add_name_ns("layout", ttml_ns);
        let layout = self.xot.new_element(layout_name);
        self.xot.create_missing_prefixes(layout).unwrap();
        let _ = self.xot.append(head,layout);
        for r in &self.regions {
            let new = self.xot.clone_with_prefixes(*r);
            let _ = self.xot.append(layout, new);
        }
        for p in &self.paragraphs {
            let new = self.xot.clone_with_prefixes(*p);
            let _ = self.xot.append(div, new);
        }
        self.xot.create_missing_prefixes(tt).unwrap();
        self.xot.deduplicate_namespaces(tt);
        self.xot.serialize_xml_string(output::xml::Parameters {
            declaration: Some(output::xml::Declaration {
                encoding: Some("UTF-8".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }, tt).unwrap()
    }
}


// Argument str contains a well-formed XML document potentially followed by trailing content. Return
// the string position corresponding to the end of XML content.
fn identify_xml_endpos(input: &str) -> Result<usize, xmlparser::Error> {
    let mut depth = 0;
    let mut xml_end;
    for token in Tokenizer::from(input) {
        let token = token?;
        xml_end = token.span().end();
        match token {
            Token::ElementStart { .. } => depth += 1,
            Token::ElementEnd { end: ElementEnd::Close(..), .. } => {
                depth -= 1;
                if depth == 0 {
                    // We've closed the root element.
                    return Ok(xml_end);
                }
            },
            Token::ElementEnd { end: ElementEnd::Empty, .. } => {
                depth -= 1;
                if depth == 0 {
                    // Root element was <root/>.
                    return Ok(xml_end);
                }
            },
            _ => {}
        }
    }
    Err(xmlparser::Error::UnknownToken(
        xmlparser::TextPos::new(1, 1),
    ))
}

