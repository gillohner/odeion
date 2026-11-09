//! A small MPEG-DASH reader for single-representation audio manifests (TIDAL's unofficial
//! playback answers with one): the initialization segment, then every media segment in order,
//! read as one fragmented-MP4 stream (`segmented::SegmentReader`). Each segment's start comes from
//! the `SegmentTimeline`, so a seek can start from the segment that holds the position. Live
//! manifests, several representations and encryption are not handled; an encrypted stream is
//! refused before it gets here.

use quick_xml::events::Event;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashManifest {
    pub codecs: String,
    pub mime_type: String,
    pub sample_rate: Option<u32>,
    /// The initialization segment, then the media segments in play order.
    pub urls: Vec<String>,
    /// Where each media segment starts, in ms from the first.
    pub starts_ms: Vec<u64>,
    /// Where the last one ends.
    pub end_ms: u64,
}

impl DashManifest {
    /// The segments with their times, for a seekable stream.
    pub fn segments(&self) -> crate::segmented::Segments {
        crate::segmented::Segments {
            init: self.urls.first().cloned(),
            media: self.urls.iter().skip(1).cloned().collect(),
            starts_ms: Some(self.starts_ms.clone()),
            end_ms: Some(self.end_ms),
        }
    }
}

fn attr(e: &quick_xml::events::BytesStart<'_>, name: &str) -> Option<String> {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| v.into_owned())
        })
}

/// Parse a manifest: the first representation's `SegmentTemplate` with its `SegmentTimeline`
/// (each `S` is one segment of `d` plus `r` repeats, starting at `t` when given, else where the
/// one before ended; in units of the template's `timescale`).
pub fn parse(xml: &str) -> Result<DashManifest, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let (mut codecs, mut mime, mut rate) = (String::new(), String::new(), None);
    let (mut init, mut media, mut start) = (None::<String>, None::<String>, 1u64);
    let mut segments: u64 = 0;
    let mut timescale: u64 = 1;
    // Segment starts in timescale units; `now` is where the next one starts
    let (mut starts, mut now) = (Vec::<u64>::new(), None::<u64>);
    let mut in_rep = false;
    let mut done_rep = false;
    loop {
        match reader.read_event().map_err(|e| format!("manifest: {e}"))? {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name();
                match name.as_ref() {
                    "AdaptationSet" if mime.is_empty() => {
                        mime = attr(&e, "mimeType").unwrap_or_default();
                    }
                    "Representation" if !done_rep => {
                        in_rep = true;
                        if let Some(c) = attr(&e, "codecs") {
                            codecs = c;
                        }
                        if let Some(m) = attr(&e, "mimeType") {
                            mime = m;
                        }
                        rate = attr(&e, "audioSamplingRate").and_then(|v| v.parse().ok());
                    }
                    "SegmentTemplate" if in_rep || !done_rep => {
                        init = attr(&e, "initialization");
                        media = attr(&e, "media");
                        start = attr(&e, "startNumber").and_then(|v| v.parse().ok()).unwrap_or(1);
                        timescale = attr(&e, "timescale")
                            .and_then(|v| v.parse().ok())
                            .filter(|t| *t > 0)
                            .unwrap_or(1);
                    }
                    "S" if in_rep || !done_rep => {
                        let r: i64 = attr(&e, "r").and_then(|v| v.parse().ok()).unwrap_or(0);
                        let d: u64 = attr(&e, "d").and_then(|v| v.parse().ok()).unwrap_or(0);
                        let mut at = attr(&e, "t").and_then(|v| v.parse().ok()).or(now).unwrap_or(0);
                        for _ in 0..1 + r.max(0) as u64 {
                            starts.push(at);
                            at = at.saturating_add(d);
                        }
                        now = Some(at);
                        segments += 1 + r.max(0) as u64;
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                if e.local_name().as_ref() == "Representation" {
                    in_rep = false;
                    done_rep = true;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let init = init.ok_or("manifest without an initialization segment")?;
    let media = media.ok_or("manifest without a media template")?;
    if segments == 0 {
        return Err("manifest without segments".into());
    }
    let mut urls = vec![init];
    for n in start..start + segments {
        urls.push(media.replace("$Number$", &n.to_string()));
    }
    let first = starts.first().copied().unwrap_or(0);
    let ms = |t: u64| (t.saturating_sub(first) as u128 * 1000 / timescale as u128) as u64;
    Ok(DashManifest {
        codecs,
        mime_type: mime,
        sample_rate: rate,
        urls,
        starts_ms: starts.iter().map(|t| ms(*t)).collect(),
        end_ms: ms(now.unwrap_or(first)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tidal_style_manifest_lists_its_segments() {
        let xml = r#"<?xml version='1.0' encoding='UTF-8'?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" mediaPresentationDuration="PT3M2.1S" type="static">
 <Period id="0">
  <AdaptationSet id="0" contentType="audio" mimeType="audio/mp4" segmentAlignment="true">
   <Representation id="FLAC,96000,24" codecs="flac" bandwidth="3000000" audioSamplingRate="96000">
    <SegmentTemplate timescale="96000" initialization="https://cdn.example/init.mp4?t=1" media="https://cdn.example/$Number$.mp4?t=1" startNumber="1">
     <SegmentTimeline>
      <S d="383936" r="44"/>
      <S d="214080"/>
     </SegmentTimeline>
    </SegmentTemplate>
   </Representation>
  </AdaptationSet>
 </Period>
</MPD>"#;
        let m = parse(xml).unwrap();
        assert_eq!(m.codecs, "flac");
        assert_eq!(m.sample_rate, Some(96_000));
        assert_eq!(m.urls.len(), 1 + 46);
        assert_eq!(m.urls[0], "https://cdn.example/init.mp4?t=1");
        assert_eq!(m.urls[1], "https://cdn.example/1.mp4?t=1");
        assert_eq!(m.urls[46], "https://cdn.example/46.mp4?t=1");
        // 383936 / 96000 s each (3.999 s), the last one 2.23 s: 182.2 s in all
        assert_eq!(m.starts_ms.len(), 46);
        assert_eq!(&m.starts_ms[..3], &[0, 3999, 7998]);
        assert_eq!(m.starts_ms[45], 45 * 383_936 * 1000 / 96_000);
        assert_eq!(m.end_ms, 182_200);
        let segs = m.segments();
        assert_eq!(segs.at(8_000), Some((2, 7998)));
        assert_eq!(
            segs.urls_from(45),
            vec![
                "https://cdn.example/init.mp4?t=1",
                "https://cdn.example/46.mp4?t=1"
            ]
        );
        assert!(parse("<MPD></MPD>").is_err());
    }

    #[test]
    fn segment_times_follow_t_and_the_timescale() {
        // A timeline that starts late and gives a `t` midway: times count from the first segment
        let xml = r#"<MPD><Period><AdaptationSet mimeType="audio/mp4"><Representation codecs="mp4a.40.2"><SegmentTemplate timescale="1000" initialization="i.mp4" media="$Number$.m4s" startNumber="0"><SegmentTimeline><S t="5000" d="2000" r="1"/><S t="10000" d="1500"/></SegmentTimeline></SegmentTemplate></Representation></AdaptationSet></Period></MPD>"#;
        let m = parse(xml).unwrap();
        assert_eq!(m.urls, vec!["i.mp4", "0.m4s", "1.m4s", "2.m4s"]);
        assert_eq!(m.starts_ms, vec![0, 2000, 5000]);
        assert_eq!(m.end_ms, 6500);
    }
}
