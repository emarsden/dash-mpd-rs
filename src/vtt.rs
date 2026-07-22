//! Support for the VTT subtitle format
//
// This module provides support for VTT subtitles that are distributed in fragmented MP4 segments.
// These subtitles are provided as a separate media stream of fMP4 segments, that the media player
// retrieves incrementally.
//
// This module implements:
//
//  - extracting the VTT fragments from an MP4 fragment
//
//  - appending them to the VttDocument object
//
//  - serializing to a single merged VTT subtitle file
//

use tracing::{trace, warn};
use bytes::Bytes;
use crate::DashMpdError;


#[derive(Clone, Debug)]
pub struct VttDocument {
    contents: Vec<String>,
    warned_binary_contents: bool,
}

impl Default for VttDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl VttDocument {
    #[must_use]
    pub fn new() -> VttDocument {
        VttDocument {
            contents: Vec::new(),
            warned_binary_contents: false,
        }
    }

    // Extract VTT content from the binary data in bytes.
    pub fn add_bytes(&mut self, bytes: &Bytes) -> Result<(), DashMpdError> {
        if let Ok(s) = str::from_utf8(bytes) {
            self.add_content(s)?;
        } else {
            if !self.warned_binary_contents {
                warn!("Ignoring invalid UTF-8 in VTT subs: {}", String::from_utf8_lossy(bytes));
                self.warned_binary_contents = true;
            }
        }
        Ok(())
    }

    pub fn add_content(&mut self, content: &str) -> Result<(), DashMpdError> {
        trace!("adding VTT content {content}");
        self.contents.push(content.to_string());
        Ok(())
    }

    // Generate a complete VTT document corresponding to the merge of all the fragments seen so
    // far. Note that we can't implement this using the fmt::Display trait for VttDocument, because
    // we need a mutable reference to self, which is not available for Display.
    #[allow(clippy::inherent_to_string)]
    pub fn to_string(&mut self) -> String {
        self.contents.concat()
    }
}

