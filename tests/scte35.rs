// TODO: the manifest at https://refplayer-dev.cloud.digitaluk.co.uk/dynamic/stl-dashads-dartest.mpd
// has scte35:Binary content that doesn't parse as Base64, for some reason.

#[test]
#[cfg_attr(not(feature = "scte35"), ignore)]
fn test_scte35_binary() {
    use dash_mpd::parse;

    let bin1 = r#"<MPD><Period>
          <EventStream timescale="10000000" schemeIdUri="urn:scte:scte35:2014:xml+bin">
            <Event>
                <scte35:Signal>
                    <scte35:Binary>/DAnAAAAAAAAAP/wBQb+0cr/PQARAg9DVUVJAAA0Q3+/AAAjAAAG6c2q</scte35:Binary>
                </scte35:Signal>
            </Event>
          </EventStream>
     </Period></MPD>"#;
    let res = parse(bin1);
    assert!(res.is_ok());

    let bin2 = r#"<MPD><Period id="2470350023" start="PT27448.333589S">
    <EventStream timescale="90000" schemeIdUri="urn:scte:scte35:2014:xml+bin">
      <Event duration="21690000" id="1">
       <scte35:Signal>
         <scte35:Binary>
           /DAlAAAAAAAAAP/wFAUAAAAEf+/+kybGyP4BSvaQAAEBAQAArky/3g==
         </scte35:Binary>
       </scte35:Signal>
      </Event>
    </EventStream>
    </Period></MPD>"#;
    let res = parse(bin2);
    assert!(res.is_ok());

    // from DVB spec, https://dvb.org/wp-content/uploads/2022/08/A178-3_Dynamic-substitution-of-content-in-linear-broadcast_Part3_Signalling-in-DVB-DASH_Interim_Draft-TS-103-752-3v111_Aug-2022.pdf
    let bin3 = r#"<MPD><Period id="1519" start="PT451209H39M31.000S">
       <EventStream schemeIdUri="urn:scte:scte35:2014:xml+bin" timescale="1" presentationTimeOffset="1624354771">
         <Event presentationTime="1624354848" duration="19" id="760">
           <Signal xmlns="http://www.scte.org/schemas/35/2016">
             <Binary>/DAgAAAAAAAAAP/wDwUAAAL4f//+ABoXsMAAAAAAAPF20V0=</Binary>
           </Signal>
         </Event>
       </EventStream>
      </Period></MPD>"#;
    let res = parse(bin3);
    assert!(res.is_ok());

    let bin4 = r#"<MPD><Period start="PT444806.040S" id="123586" duration="PT15.000S">
    <EventStream schemeIdUri="urn:scte:scte35:2014:xml+bin" timescale="1">
      <Event presentationTime="1541436240" duration="24" id="29">
        <scte35:Signal xmlns="http://www.scte.org/schemas/35/2016">
          <scte35:Binary>/DAhAAAAAAAAAP/wEAUAAAHAf+9/fgAg9YDAAAAAAAA25aoh</scte35:Binary>
        </scte35:Signal>
      </Event>
      <Event presentationTime="1541436360" duration="24" id="30">
        <scte35:Signal xmlns="http://www.scte.org/schemas/35/2016">
          <scte35:Binary>QW5vdGhlciB0ZXN0IHN0cmluZyBmb3IgZW5jb2RpbmcgdG8gQmFzZTY0IGVuY29kZWQgYmluYXJ5Lg==</scte35:Binary>
        </scte35:Signal>
      </Event></EventStream>
    </Period></MPD>"#;
    assert!(parse(bin4).is_ok());

    let bin5 = r#"<MPD><Period><EventStream schemeIdUri="urn:scte:scte35:2014:xml+bin" timescale="1">
       <Event presentationTime="1540809120" id="1999">
          <Signal xmlns="http://www.scte.org/schemas/35/2016"><Binary>/DAhAAAAAAAAAP/wEAUAAAfPf+9/fgAg9YDAAAAAAAA/APOv</Binary>
          </Signal>
       </Event>
      </EventStream>
     </Period></MPD>"#;
    assert!(parse(bin5).is_ok());

    // from https://developers.broadpeak.io/docs/input-streaming-formats
    let bin6 = r#"<MPD><Period><EventStream
      schemeIdUri="urn:scte:scte35:2014:xml+bin" timescale="10000000">
      <Event presentationTime="15516962501159406" id="3999785549">
        <Signal xmlns="http://www.scte.org/schemas/35/2016"> 
           <Binary>/DAnAAAAAAAAAP/wBQb/Y/SedwARAg9DVUVJAAAAPH+/AAAjAQEGLc/Q
           </Binary>
        </Signal>
      </Event>
      </EventStream></Period></MPD>"#;
    assert!(parse(bin6).is_ok());

    let bin7 = r#"<MPD><Period><EventStream
      schemeIdUri="urn:scte:scte35:2014:xml+bin" timescale="10000000">
      <Event presentationTime="15516962501159406" id="2">
        <Signal xmlns="http://www.scte.org/schemas/35/2016">
           <Binary>/DAWAAAAAAAAAP/wBQb+AKmKxwAACzuu2Q==</Binary>
        </Signal>
        <Signal xmlns="http://www.scte.org/schemas/35/2016">
           <Binary>/DCtAAAAAAAAAP/wBQb+Tq9DwQCXAixDVUVJCUvhcH+fAR1QQ1IxXzEyMTYyMTE0MDBXQUJDUkFDSEFFTFJBWSEBAQIsQ1VFSQlL4W9/nwEdUENSMV8xMjE2MjExNDAwV0FCQ1JBQ0hBRUxSQVkRAQECGUNVRUkJTBwVf58BClRLUlIxNjA4NEEQAQECHkNVRUkJTBwWf98AA3clYAEKVEtSUjE2MDg0QSABAdHBXYA=</Binary>
        </Signal>
      </Event>
      </EventStream></Period></MPD>"#;
    assert!(parse(bin7).is_ok());
}

#[test]
#[cfg_attr(not(feature = "scte35"), ignore)]
fn test_scte35_elements() {
    use dash_mpd::parse;

    let elem1 = r#"<MPD><Period start="PT444806.040S" id="123586" duration="PT15.000S">
      <EventStream timescale="90000" schemeIdUri="urn:scte:scte35:2013:xml">
        <Event duration="1350000">
          <scte35:SpliceInfoSection protocolVersion="0" ptsAdjustment="180832" tier="4095">
            <scte35:SpliceInsert spliceEventId="4026531855" spliceEventCancelIndicator="false" outOfNetworkIndicator="true" spliceImmediateFlag="false" uniqueProgramId="1" availNum="1" availsExpected="1">
              <scte35:Program><scte35:SpliceTime ptsTime="5672624400"/></scte35:Program>
              <scte35:BreakDuration autoReturn="true" duration="1350000"/>
            </scte35:SpliceInsert>
          </scte35:SpliceInfoSection>
        </Event>
      </EventStream>
    </Period></MPD>"#;
    assert!(parse(elem1).is_ok());

    let elem2 = r#"<MPD><Period start="PT444806.040S" id="123586" duration="PT15.000S">
    <EventStream timescale="90000" schemeIdUri="urn:scte:scte35:2013:xml">
      <Event duration="1350000">
        <scte35:SpliceInfoSection protocolVersion="0" ptsAdjustment="180832" tier="4095">
          <scte35:SpliceInsert spliceEventId="4026531855" spliceEventCancelIndicator="false" outOfNetworkIndicator="true" spliceImmediateFlag="false" uniqueProgramId="1" availNum="1" availsExpected="1">
            <scte35:Program><scte35:SpliceTime ptsTime="5672624400"/></scte35:Program>
            <scte35:BreakDuration autoReturn="true" duration="1350000"/>
          </scte35:SpliceInsert>
        </scte35:SpliceInfoSection>
      </Event></EventStream>
    </Period></MPD>"#;
    assert!(parse(elem2).is_ok());

    let elem3 = r#"<MPD><Period start="PT346530.250S" id="178443" duration="PT61.561S">
    <EventStream timescale="90000" schemeIdUri="urn:scte:scte35:2013:xml">
      <Event duration="5310000">
        <scte35:SpliceInfoSection protocolVersion="0" ptsAdjustment="183003" tier="4095">
          <scte35:TimeSignal>
            <scte35:SpliceTime ptsTime="3442857000"/>
          </scte35:TimeSignal>
          <scte35:SegmentationDescriptor segmentationEventId="1414668" segmentationEventCancelIndicator="false" segmentationDuration="8100000">
            <scte35:DeliveryRestrictions webDeliveryAllowedFlag="false" noRegionalBlackoutFlag="false" archiveAllowedFlag="false" deviceRestrictions="3"/>
            <scte35:SegmentationUpid segmentationUpidType="12" segmentationUpidLength="2" segmentationTypeId="52" segmentNum="0" segmentsExpected="0">0100</scte35:SegmentationUpid>
          </scte35:SegmentationDescriptor>
        </scte35:SpliceInfoSection>
      </Event></EventStream>
     </Period></MPD>"#;
    assert!(parse(elem3).is_ok());

    let elem4 = r#"<MPD>  <Period start="PT346530.250S" id="178443" duration="PT61.561S">
    <EventStream timescale="90000" schemeIdUri="urn:scte:scte35:2013:xml">
      <Event duration="5310000">
        <scte35:SpliceInfoSection protocolVersion="0" ptsAdjustment="183003" tier="4095">
          <scte35:TimeSignal>
            <scte35:SpliceTime ptsTime="3442857000"/>
          </scte35:TimeSignal>
          <scte35:SegmentationDescriptor segmentationEventId="1414668" segmentationEventCancelIndicator="false" segmentationDuration="8100000">
            <scte35:DeliveryRestrictions webDeliveryAllowedFlag="false" noRegionalBlackoutFlag="false" archiveAllowedFlag="false" deviceRestrictions="3"/>
            <scte35:SegmentationUpid segmentationUpidType="12" segmentationUpidLength="2" segmentationTypeId="52" segmentNum="0" segmentsExpected="0">0100</scte35:SegmentationUpid>
          </scte35:SegmentationDescriptor>
        </scte35:SpliceInfoSection>
      </Event></EventStream></Period></MPD>"#;
    assert!(parse(elem4).is_ok());
}

