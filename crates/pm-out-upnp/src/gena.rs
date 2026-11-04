//! UPnP eventing (GENA): subscribing to a renderer's volume changes and reading its NOTIFY
//! bodies, so a change made on the device, its remote or its own app shows at once.
//!
//! Two sources, the first one a renderer has wins: the OpenHome `Volume` service (properties
//! `Volume`, `Mute`, `VolumeMax`; a Linn's, and what Roon follows) and RenderingControl, whose one
//! evented variable `LastChange` carries an escaped `<Event>` document with `Volume` and `Mute`.
//! The node answers every NOTIFY with 200; the subscription is renewed before it runs out and
//! made again when the renderer forgets it (412) or restarts.

use crate::UpnpError;
use std::time::Duration;

/// Which service a subscription follows; it decides how a NOTIFY body reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    OpenHomeVolume,
    RenderingControl,
}

/// What one NOTIFY said about the volume; a field it didn't carry is `None`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VolumeChange {
    /// 0–100, already scaled by OpenHome's `VolumeMax`.
    pub percent: Option<u8>,
    pub muted: Option<bool>,
    /// OpenHome's `VolumeMax` when this event carried it (a later `Volume` scales by it).
    pub volume_max: Option<u32>,
}

/// A subscription the renderer granted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub sid: String,
    /// What the renderer granted (`TIMEOUT: Second-N`); renew before it runs out.
    pub timeout: Duration,
}

const ASK_FOR: &str = "Second-300";

fn client() -> Result<reqwest::blocking::Client, UpnpError> {
    Ok(reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()?)
}

fn method(name: &str) -> reqwest::Method {
    reqwest::Method::from_bytes(name.as_bytes()).expect("a GENA method name")
}

fn granted(resp: &reqwest::blocking::Response) -> Result<Subscription, UpnpError> {
    let header = |k: &str| {
        resp.headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let sid = header("SID").ok_or_else(|| UpnpError::Soap {
        action: "SUBSCRIBE".into(),
        detail: "no SID in the answer".into(),
    })?;
    Ok(Subscription {
        sid,
        timeout: header("TIMEOUT")
            .map(|t| parse_timeout(&t))
            .unwrap_or(Duration::from_secs(300)),
    })
}

/// `Second-300` → 300 s; `infinite` (or nonsense) → 300 s, renewed like any other.
pub fn parse_timeout(t: &str) -> Duration {
    t.trim()
        .strip_prefix("Second-")
        .and_then(|n| n.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(300))
}

/// Subscribe `callback` (the node's own URL) to the events at `event_url`. Blocking.
pub fn subscribe(event_url: &str, callback: &str) -> Result<Subscription, UpnpError> {
    let resp = client()?
        .request(method("SUBSCRIBE"), event_url)
        .header("CALLBACK", format!("<{callback}>"))
        .header("NT", "upnp:event")
        .header("TIMEOUT", ASK_FOR)
        .send()?;
    if !resp.status().is_success() {
        return Err(UpnpError::Soap {
            action: "SUBSCRIBE".into(),
            detail: format!("the renderer answered {}", resp.status()),
        });
    }
    granted(&resp)
}

/// Renew `sid`. `Ok(None)` when the renderer no longer knows it (412): subscribe again.
pub fn renew(event_url: &str, sid: &str) -> Result<Option<Subscription>, UpnpError> {
    let resp = client()?
        .request(method("SUBSCRIBE"), event_url)
        .header("SID", sid)
        .header("TIMEOUT", ASK_FOR)
        .send()?;
    if resp.status() == reqwest::StatusCode::PRECONDITION_FAILED {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(UpnpError::Soap {
            action: "SUBSCRIBE".into(),
            detail: format!("renew answered {}", resp.status()),
        });
    }
    granted(&resp).map(Some)
}

/// End `sid` (the zone went away); a failure is only logged by the caller.
pub fn unsubscribe(event_url: &str, sid: &str) -> Result<(), UpnpError> {
    client()?
        .request(method("UNSUBSCRIBE"), event_url)
        .header("SID", sid)
        .send()?;
    Ok(())
}

/// The text of each `<e:property>` child in a NOTIFY body, by local name (unescaped).
fn properties(body: &str) -> Vec<(String, String)> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut r = Reader::from_str(body);
    r.config_mut().trim_text(true);
    let mut out = Vec::new();
    let mut depth_in_property = 0usize;
    let mut current: Option<(String, String)> = None;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                if name == "property" {
                    depth_in_property = 1;
                } else if depth_in_property == 1 {
                    current = Some((name, String::new()));
                    depth_in_property = 2;
                } else if depth_in_property > 1 {
                    depth_in_property += 1;
                }
            }
            Ok(Event::Text(t)) => {
                if let Some((_, v)) = current.as_mut() {
                    v.push_str(&t.xml_content(quick_xml::XmlVersion::Implicit1_0));
                }
            }
            Ok(Event::CData(t)) => {
                if let Some((_, v)) = current.as_mut() {
                    v.push_str(t.as_ref());
                }
            }
            // quick-xml reports `&lt;` and friends apart from the text: an escaped LastChange
            // document arrives in pieces, put back together here
            Ok(Event::GeneralRef(r)) => {
                if let Some((_, v)) = current.as_mut() {
                    v.push_str(&entity(r.as_ref()));
                }
            }
            Ok(Event::End(e)) => {
                let name = e.local_name().as_ref().to_string();
                if name == "property" {
                    depth_in_property = 0;
                } else if depth_in_property == 2 {
                    if let Some(p) = current.take() {
                        out.push(p);
                    }
                    depth_in_property = 1;
                } else if depth_in_property > 2 {
                    depth_in_property -= 1;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

/// The text an entity reference stands for (`lt` → `<`, `#38` → `&`); an unknown one kept as written.
fn entity(name: &str) -> String {
    let c = match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        n => n
            .strip_prefix("#x")
            .or_else(|| n.strip_prefix("#X"))
            .and_then(|h| u32::from_str_radix(h, 16).ok())
            .or_else(|| n.strip_prefix('#').and_then(|d| d.parse::<u32>().ok()))
            .and_then(char::from_u32),
    };
    c.map(String::from).unwrap_or_else(|| format!("&{name};"))
}

fn truthy(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

/// What a NOTIFY body from `source` says about the volume. `volume_max` is the last OpenHome
/// `VolumeMax` seen for this renderer (100 when none yet).
pub fn read_notify(source: Source, body: &str, volume_max: u32) -> VolumeChange {
    let mut c = VolumeChange::default();
    match source {
        Source::OpenHomeVolume => {
            let props = properties(body);
            let max = props
                .iter()
                .find(|(k, _)| k == "VolumeMax")
                .and_then(|(_, v)| v.trim().parse::<u32>().ok())
                .filter(|m| *m > 0);
            c.volume_max = max;
            let max = max.unwrap_or(volume_max.max(1));
            for (k, v) in &props {
                match k.as_str() {
                    "Volume" => {
                        if let Ok(n) = v.trim().parse::<u32>() {
                            c.percent = Some(((n.min(max) * 100 + max / 2) / max).min(100) as u8);
                        }
                    }
                    "Mute" => c.muted = truthy(v),
                    _ => {}
                }
            }
        }
        Source::RenderingControl => {
            for (k, v) in properties(body) {
                if k == "LastChange" {
                    let (p, m) = last_change(&v);
                    c.percent = p.or(c.percent);
                    c.muted = m.or(c.muted);
                }
            }
        }
    }
    c
}

/// `Volume` and `Mute` of the master channel (or of no channel named) in a LastChange `<Event>`.
fn last_change(doc: &str) -> (Option<u8>, Option<bool>) {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut r = Reader::from_str(doc);
    r.config_mut().trim_text(true);
    let (mut vol, mut mute) = (None, None);
    loop {
        match r.read_event() {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                if name != "Volume" && name != "Mute" {
                    continue;
                }
                let mut channel = None;
                let mut val = None;
                for a in e.attributes().flatten() {
                    let key = a.key.local_name().as_ref().to_string();
                    let v = a.value.to_string();
                    match key.as_str() {
                        "channel" => channel = Some(v),
                        "val" => val = Some(v),
                        _ => {}
                    }
                }
                if channel
                    .as_deref()
                    .is_some_and(|c| !c.eq_ignore_ascii_case("Master"))
                {
                    continue;
                }
                let Some(v) = val else { continue };
                if name == "Volume" {
                    vol = v.trim().parse::<u32>().ok().map(|n| n.min(100) as u8);
                } else {
                    mute = truthy(&v);
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (vol, mute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_control_last_change_gives_volume_and_mute() {
        // As a Linn (and most renderers) send it: the Event document escaped inside LastChange
        let body = r#"<?xml version="1.0"?><e:propertyset xmlns:e="urn:schemas-upnp-org:event-1-0"><e:property><LastChange>&lt;Event xmlns="urn:schemas-upnp-org:metadata-1-0/RCS/"&gt;&lt;InstanceID val="0"&gt;&lt;Volume channel="LF" val="12"/&gt;&lt;Volume channel="Master" val="37"/&gt;&lt;Mute channel="Master" val="1"/&gt;&lt;/InstanceID&gt;&lt;/Event&gt;</LastChange></e:property></e:propertyset>"#;
        let c = read_notify(Source::RenderingControl, body, 100);
        assert_eq!((c.percent, c.muted), (Some(37), Some(true)));
        // Only the mute changed: no volume in it
        let body = r#"<e:propertyset xmlns:e="urn:schemas-upnp-org:event-1-0"><e:property><LastChange>&lt;Event&gt;&lt;InstanceID val="0"&gt;&lt;Mute channel="Master" val="0"/&gt;&lt;/InstanceID&gt;&lt;/Event&gt;</LastChange></e:property></e:propertyset>"#;
        let c = read_notify(Source::RenderingControl, body, 100);
        assert_eq!((c.percent, c.muted), (None, Some(false)));
    }

    #[test]
    fn openhome_volume_is_scaled_by_its_maximum() {
        let body = r#"<?xml version="1.0"?><e:propertyset xmlns:e="urn:schemas-upnp-org:event-1-0"><e:property><Volume>66</Volume></e:property><e:property><Mute>false</Mute></e:property><e:property><VolumeMax>100</VolumeMax></e:property></e:propertyset>"#;
        let c = read_notify(Source::OpenHomeVolume, body, 100);
        assert_eq!(
            (c.percent, c.muted, c.volume_max),
            (Some(66), Some(false), Some(100))
        );
        // A renderer whose scale ends at 80: 40 of 80 is 50 %; later events use the stored max
        let body = r#"<e:propertyset xmlns:e="urn:schemas-upnp-org:event-1-0"><e:property><VolumeMax>80</VolumeMax></e:property><e:property><Volume>40</Volume></e:property></e:propertyset>"#;
        assert_eq!(read_notify(Source::OpenHomeVolume, body, 100).percent, Some(50));
        let body = r#"<e:propertyset xmlns:e="urn:schemas-upnp-org:event-1-0"><e:property><Volume>20</Volume></e:property></e:propertyset>"#;
        assert_eq!(read_notify(Source::OpenHomeVolume, body, 80).percent, Some(25));
        let body = r#"<e:propertyset xmlns:e="urn:schemas-upnp-org:event-1-0"><e:property><Mute>1</Mute></e:property></e:propertyset>"#;
        assert_eq!(read_notify(Source::OpenHomeVolume, body, 80).muted, Some(true));
    }

    #[test]
    fn timeouts_read_and_fall_back() {
        assert_eq!(parse_timeout("Second-1800"), Duration::from_secs(1800));
        assert_eq!(parse_timeout("infinite"), Duration::from_secs(300));
        assert_eq!(parse_timeout("Second-0"), Duration::from_secs(300));
    }
}
