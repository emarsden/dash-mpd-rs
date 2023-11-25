//! Support for the SCTE-35 standard for insertion of alternate content
//
// Society of Cable Telecommunications Engineers (SCTE) standard 35 "Digital Program Insertion
// Cueing Message" concerns the messages that specify points in a content stream where alternate
// content (typically advertising or local programming) can be inserted. It is used for example for
// the digital broadcast of TV content or "replay" (VOD) media, and allows the content provider to
// specify timestamps where advertising can be inserted dynamically. The advertising content
// typically comes from a third party ad distributor (Google, Amazon for example) and can be
// customized based on the viewer's location, account, preferences guessed from prior viewing,
// viewing time, etc.
//
//      https://en.wikipedia.org/wiki/SCTE-35
//
// SCTE-35 messages can be included inside the media stream (for example in MPEG TS streams or
// fragmented MP4 segments), in an HLS manifest, or in a DASH manifest, where they are carried in
// DASH Event elements within an ElementStream element. This file provides definitions for the XML
// elements used for DASH support.
//
// You won't often find public DASH streams with SCTE-35 events; they are more often used for
// server-side ad insertion, which helps ensure that viewers benefit from the advertising content
// instead of blocking or skipping it. For this reason, these definitions have not been well tested.
//
// An XML Schema for this embedding is available at
// https://github.com/Comcast/scte35-go/blob/main/docs/scte_35_20220816.xsd


#![allow(non_snake_case)]
use serde::{Serialize, Deserialize};
use serde_with::skip_serializing_none;


pub fn serialize_scte35_ns<S>(os: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
where S: serde::Serializer {
    if let Some(s) = os {
        serializer.serialize_str(s)
    } else {
        serializer.serialize_str("http://www.scte.org/schemas/35/2016")
    }
}


#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct AvailDescriptor {
    #[serde(rename = "@providerAvailId")]
    pub provider_avail_id: u32,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct DTMFDescriptor {
    #[serde(rename = "@preroll")]
    pub preroll: Option<u8>,
    #[serde(rename = "@chars")]
    pub chars: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct TimeDescriptor {
    #[serde(rename = "@taiSeconds")]
    pub tai_seconds: Option<u64>,
    #[serde(rename = "@taiNs")]
    pub tai_ns: Option<u32>,
    #[serde(rename = "@utcOffset")]
    pub utc_offset: Option<u16>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct BreakDuration {
    // some buggy MPDs in the wild have this as a 0/1
    #[serde(rename = "@autoReturn")]
    pub auto_return: bool,
    #[serde(rename = "@duration")]
    pub duration: u64,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct ScteEvent {
    #[serde(rename = "@spliceEventId")]
    pub splice_event_id: Option<u32>,
    #[serde(rename = "@spliceEventCancelIndicator")]
    pub splice_event_cancel_indicator: Option<bool>,
    #[serde(rename = "@outOfNetworkIndicator")]
    pub out_of_network_indicator: Option<bool>,
    #[serde(rename = "@uniqueProgramId")]
    pub unique_program_id: Option<u16>,
    #[serde(rename = "@availNum")]
    pub avail_num: Option<u8>,
    #[serde(rename = "@availsExpected")]
    pub avails_expected: Option<u8>,
    #[serde(rename = "scte35:BreakDuration", alias="BreakDuration")]
    pub break_duration: Option<BreakDuration>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SpliceTime {
    #[serde(rename = "@xmlns")]
    pub xmlns: Option<String>,
    #[serde(rename = "@scte35:ptsTime", alias = "@ptsTime")]
    pub pts_time: Option<u64>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SpliceNull {
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SpliceSchedule {
    #[serde(rename = "scte35:Event", alias="Event")]
    pub scte_events: Vec<ScteEvent>,
    // TODO: SpliceCount?
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct BandwidthReservation {
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct EncryptedPacket {
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct PrivateBytes {
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct PrivateCommand {
    #[serde(rename = "@identifier")]
    pub identifier: u32,
    #[serde(rename = "scte35:PrivateBytes", alias="PrivateBytes")]
    pub private_bytes: Vec<PrivateBytes>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct TimeSignal {
    #[serde(rename = "scte35:SpliceTime", alias="SpliceTime")]
    pub splice_time: Vec<SpliceTime>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SegmentationUpid {
    #[serde(rename = "@xmlns")]
    pub xmlns: Option<String>,
    #[serde(rename = "@segmentationUpidType")]
    pub segmentation_upid_type: Option<u8>,
    #[serde(rename = "@formatIdentifier")]
    pub format_identifier: Option<u32>,
    #[serde(rename = "@segmentationUpidFormat")]
    pub segmentation_upid_format: Option<String>,
    #[serde(rename = "@format")]
    pub format: Option<String>,
    #[serde(rename = "$value")]
    pub content: Option<String>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SegmentationDescriptor {
    #[serde(rename = "@xmlns")]
    pub xmlns: Option<String>,
    #[serde(rename = "@segmentationEventId")]
    pub segmentation_event_id: Option<u32>,
    #[serde(rename = "@segmentationEventCancelIndicator")]
    pub segmentation_event_cancel_indicator: Option<bool>,
    #[serde(rename = "@spliceEventId")]
    pub splice_event_id: Option<u64>,
    #[serde(rename = "@segmentationTypeId")]
    pub segmentation_type_id: Option<u8>,
    #[serde(rename = "@segmentationDuration")]
    pub segmentation_duration: Option<u64>,
    #[serde(rename = "@segmentationUpidType")]
    pub segmentation_upid_type: Option<u8>,
    #[serde(rename = "@segmentationUpid")]
    pub segmentation_upid: Option<u64>,
    #[serde(rename = "@segmentNum")]
    pub segment_num: Option<u8>,
    #[serde(rename = "@segmentsExpected")]
    pub segments_expected: Option<u8>,
    #[serde(rename = "@subSegmentNum")]
    pub sub_segment_num: Option<u8>,
    #[serde(rename = "@subSegmentsExpected")]
    pub sub_segments_expected: Option<u8>,
    #[serde(rename = "scte35:SegmentationUpid", alias="SegmentationUpid")]
    pub segmentation_upids: Vec<SegmentationUpid>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Program {
    #[serde(rename = "scte35:SpliceTime", alias="SpliceTime")]
    pub splice_time: Vec<SpliceTime>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SpliceInsert {
    #[serde(rename = "@spliceEventId")]
    pub splice_event_id: Option<u32>,
    #[serde(rename = "@spliceEventCancelIndicator")]
    pub splice_event_cancel_indicator: Option<bool>,
    #[serde(rename = "@outOfNetworkIndicator")]
    pub out_of_network_indicator: Option<bool>,
    #[serde(rename = "@spliceImmediateFlag")]
    pub splice_immediate_flag: Option<bool>,
    #[serde(rename = "@uniqueProgramId")]
    pub unique_program_id: Option<u16>,
    #[serde(rename = "@availNum")]
    pub avail_num: Option<u8>,
    #[serde(rename = "@availsExpected")]
    pub avails_expected: Option<u8>,
    #[serde(rename = "scte35:BreakDuration", alias="BreakDuration")]
    pub break_duration: Option<BreakDuration>,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct SpliceInfoSection {
    #[serde(rename = "@xmlns")]
    pub xmlns: Option<String>,
    #[serde(rename = "@sapType")]
    pub sap_type: Option<u16>,
    #[serde(rename = "@preRollMilliSeconds")]
    pub pre_roll_milliseconds: Option<u32>,
    #[serde(rename = "@ptsAdjustment")]
    pub pts_adjustment: Option<u64>,
    #[serde(rename = "@protocolVersion")]
    pub protocol_version: Option<u8>,
    #[serde(rename = "@tier")]
    pub tier: Option<u16>,
    #[serde(rename = "scte35:TimeSignal", alias="TimeSignal")]
    pub time_signal: Option<TimeSignal>,
    #[serde(rename = "scte35:SegmentationDescriptor", alias="SegmentationDescriptor")]
    pub segmentation_descriptor: Option<SegmentationDescriptor>,
    #[serde(rename = "scte35:SpliceNull", alias="SpliceNull")]
    pub splice_null: Option<SpliceNull>,
    #[serde(rename = "scte35:SpliceInsert", alias="SpliceInsert")]
    pub splice_insert: Option<SpliceInsert>,
    #[serde(rename = "scte35:SpliceSchedule", alias="SpliceSchedule")]
    pub splice_schedule: Option<SpliceSchedule>,
    #[serde(rename = "scte35:BandwidthReservation", alias="BandwidthReservation")]
    pub bandwidth_reservation: Option<BandwidthReservation>,
    #[serde(rename = "scte35:PrivateCommand", alias="PrivateCommand")]
    pub private_command: Option<PrivateCommand>,
    #[serde(rename = "scte35:EncryptedPacket", alias="EncryptedPacket")]
    pub encrypted_packet: Option<EncryptedPacket>,
    #[serde(rename = "scte35:AvailDescriptor", alias="AvailDescriptor")]
    pub avail_descriptor: Option<AvailDescriptor>,
    #[serde(rename = "scte35:DTMFDescriptor", alias="DTMFDescriptor")]
    pub dtmf_descriptor: Option<DTMFDescriptor>,
    #[serde(rename = "scte35:TimeDescriptor", alias="TimeDescriptor")]
    pub time_descriptor: Option<TimeDescriptor>,
}

/// A binary representation of a SCTE 35 cue message. We don't attempt to decode these, but the
/// `scte35-reader` crate is able to parse a subset of the standard, and the `threefive` Python
/// library provides a full parser.
///
/// Basic messages may just be '/' + base64-encoded string
///   e.g. "/TWFpbiBDb250ZW50" -> "Main Content"
#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Binary {
    #[serde(rename = "@signalType")]
    pub signal_type: Option<String>,
    #[serde(rename = "$value")]
    pub content: String,
}

#[skip_serializing_none]
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct Signal {
    #[serde(rename = "@xmlns")]
    pub xmlns: Option<String>,
    #[serde(rename = "scte35:SpliceInfoSection", alias="SpliceInfoSection")]
    pub splice_info_section: Option<SpliceInfoSection>,
    #[serde(rename = "scte35:Binary", alias="Binary")]
    pub content: Option<Binary>,
}

