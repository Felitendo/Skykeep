//! What a note holds besides text and tables: pictures, scans, PDFs, recordings, films and
//! other files — read so a note can show them where they sit and open them.
//!
//! Measured on a real account: an `Attachment` record names its kind (`UTIEncrypted`, e.g.
//! `public.jpeg`, `com.apple.paper.doc.pdf`, `com.apple.m4a-audio`) and points by its `Media`
//! reference at a `Media` record, whose `Asset` is the file itself (`downloadURL`) and whose
//! `FilenameEncrypted` is its name. Pictures, scans, PDFs and films also carry
//! `PreviewImages` — small renderings, a few at different sizes — with `PreviewWidths` /
//! `PreviewHeights`, and pictures their `Width` / `Height`. Every asset is served as
//! `application/octet-stream`, so what a file is gets read from its own bytes.
//!
//! Asset addresses expire, so the records are looked up fresh each time a file is fetched.

use crate::cloudkit::ZoneClient;
use crate::{Error, Result};
use serde::Serialize;
use serde_json::Value;

const CONTAINER: &str = "com.apple.notes";
const ZONE: &str = "Notes";

/// What an attachment is, to draw it in its note.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInfo {
    pub id: String,
    pub uti: String,
    /// `image`, `drawing`, `table`, `document` (PDF, scan), `audio`, `video`, `inline` (a
    /// tag or a mention, drawn as its text), or `file`.
    pub kind: &'static str,
    pub filename: Option<String>,
    pub title: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    pub size: Option<i64>,
    /// Whether Apple keeps a rendering of it to show in place of the file.
    pub has_preview: bool,
}

/// The kind of thing a type identifier names, as Notes draws it.
#[must_use]
pub fn kind_of(uti: &str) -> &'static str {
    let u = uti.to_ascii_lowercase();
    // A drawing made in the note (iOS 17's inline drawings, and the older drawing kinds):
    // shown as Apple renders it, never as a file.
    if matches!(
        u.as_str(),
        "com.apple.paper" | "com.apple.drawing" | "com.apple.drawing.2"
    ) {
        return "drawing";
    }
    // Text set inside a line — a #tag (`…inlinetextattachment.hashtag`) or an @mention: drawn
    // as the words it shows, never as a file.
    if u.starts_with("com.apple.notes.inlinetextattachment") {
        return "inline";
    }
    // A table: drawn as its grid, never offered as a file to open.
    if u == crate::table::UTI {
        return "table";
    }
    if matches!(
        u.as_str(),
        "public.jpeg"
            | "public.png"
            | "public.heic"
            | "public.heif"
            | "com.compuserve.gif"
            | "public.tiff"
            | "org.webmproject.webp"
            | "public.image"
    ) {
        "image"
    } else if u.contains("pdf") || u.contains("scan") {
        "document"
    } else if u.contains("audio") || u.ends_with(".mp3") || u.contains("m4a") {
        "audio"
    } else if u.contains("movie")
        || u.contains("video")
        || u.contains("mpeg-4") && !u.contains("audio")
    {
        "video"
    } else {
        "file"
    }
}

fn decoded(record: &Value, field: &str) -> Option<String> {
    use base64::Engine;
    let v = record
        .pointer(&format!("/fields/{field}/value"))?
        .as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(v).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let t = text.trim().to_string();
    (!t.is_empty()).then_some(t)
}

fn number(record: &Value, field: &str) -> Option<f64> {
    record.pointer(&format!("/fields/{field}/value"))?.as_f64()
}

/// What the attachment is, read from Apple now.
pub async fn info(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    id: &str,
) -> Result<AttachmentInfo> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let record = client
        .lookup_record(ZONE, id)
        .await?
        .ok_or(Error::NotFound)?;
    let uti = decoded(&record, "UTIEncrypted")
        .or_else(|| {
            record
                .pointer("/fields/UTI/value")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    if kind_of(&uti) == "inline" {
        return Ok(inline_info(id, uti, &record));
    }
    let media = match record
        .pointer("/fields/Media/value/recordName")
        .and_then(Value::as_str)
    {
        Some(m) => client.lookup_record(ZONE, m).await?,
        None => None,
    };
    // A paper document's file is its PDF, named from its title.
    let paper = media.is_none() && record.pointer("/fields/FallbackPDF/value").is_some();
    let filename = media
        .as_ref()
        .and_then(|m| decoded(m, "FilenameEncrypted"))
        .or_else(|| {
            paper.then(|| {
                format!(
                    "{}.pdf",
                    decoded(&record, "TitleEncrypted").unwrap_or_else(|| "Document".into())
                )
            })
        });
    Ok(AttachmentInfo {
        id: id.to_string(),
        kind: kind_of(&uti),
        filename,
        title: decoded(&record, "TitleEncrypted"),
        width: number(&record, "Width"),
        height: number(&record, "Height"),
        size: record
            .pointer("/fields/FileSize/value")
            .and_then(Value::as_i64)
            .or_else(|| {
                media
                    .as_ref()
                    .and_then(|m| m.pointer("/fields/Asset/value/size"))
                    .and_then(Value::as_i64)
            })
            .or_else(|| {
                record
                    .pointer("/fields/FallbackPDF/value/size")
                    .and_then(Value::as_i64)
            }),
        has_preview: record
            .pointer("/fields/PreviewImages/value")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty()),
        uti,
    })
}

/// A tag or mention, with the words it shows as its title. Notes keeps those words as the
/// attachment's alt text, cased as typed, and a tag's normalised name as its token (upper
/// case, no `#`); either is read, plain or in its `…Encrypted` form.
fn inline_info(id: &str, uti: String, record: &Value) -> AttachmentInfo {
    let field = |name: &str| {
        decoded(record, &format!("{name}Encrypted")).or_else(|| {
            record
                .pointer(&format!("/fields/{name}/value"))
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
    };
    let hashtag = uti.to_ascii_lowercase().ends_with(".hashtag");
    let title = field("AltText").or_else(|| {
        field("TokenContentIdentifier").map(|t| {
            if hashtag && !t.starts_with('#') {
                format!("#{t}")
            } else {
                t
            }
        })
    });
    AttachmentInfo {
        id: id.to_string(),
        kind: "inline",
        filename: None,
        title,
        width: None,
        height: None,
        size: None,
        has_preview: false,
        uti,
    }
}

/// Which bytes of an attachment to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The file itself.
    File,
    /// Apple's largest rendering of it.
    Preview,
    /// A drawing as Apple renders it: its fallback image (twice its size, ink on white), or
    /// its largest preview when it has none.
    Drawing,
}

async fn download(http: &reqwest::Client, url: &str) -> Result<Vec<u8>> {
    // CloudKit's asset addresses carry a `${f}` for a filename of the caller's choosing.
    let response = http.get(url.replace("${f}", "file")).send().await?;
    if !response.status().is_success() {
        return Err(Error::Protocol(format!(
            "Apple would not hand over that attachment ({})",
            response.status()
        )));
    }
    Ok(response.bytes().await?.to_vec())
}

/// An attachment's bytes, fetched from Apple now.
pub async fn bytes(
    http: &reqwest::Client,
    ckdatabasews_root: &str,
    params: Vec<(String, String)>,
    id: &str,
    part: Part,
) -> Result<Vec<u8>> {
    let client = ZoneClient::new(http, ckdatabasews_root, CONTAINER, params);
    let record = client
        .lookup_record(ZONE, id)
        .await?
        .ok_or(Error::NotFound)?;
    if part == Part::Drawing {
        if let Some(url) = record
            .pointer("/fields/FallbackImage/value/downloadURL")
            .and_then(Value::as_str)
        {
            return download(http, url).await;
        }
    }
    match part {
        Part::Preview | Part::Drawing => {
            let previews = record
                .pointer("/fields/PreviewImages/value")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let widths: Vec<f64> = record
                .pointer("/fields/PreviewWidths/value")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_f64).collect())
                .unwrap_or_default();
            // The largest Apple keeps.
            let best = (0..previews.len()).max_by(|&a, &b| {
                widths
                    .get(a)
                    .unwrap_or(&0.0)
                    .partial_cmp(widths.get(b).unwrap_or(&0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            let url = best
                .and_then(|i| previews.get(i))
                .and_then(|p| p.get("downloadURL"))
                .and_then(Value::as_str)
                .ok_or(Error::NotFound)?;
            download(http, url).await
        }
        Part::File => {
            // A PDF or scan made in Notes ("paper") keeps no Media record: its own format is in
            // `PaperDatabase`, and a ready PDF of it in `FallbackPDF` — what any other app opens.
            if let Some(url) = record
                .pointer("/fields/FallbackPDF/value/downloadURL")
                .and_then(Value::as_str)
            {
                if record.pointer("/fields/Media/value/recordName").is_none() {
                    return download(http, url).await;
                }
            }
            let media = record
                .pointer("/fields/Media/value/recordName")
                .and_then(Value::as_str)
                .ok_or(Error::NotFound)?;
            let media = client
                .lookup_record(ZONE, media)
                .await?
                .ok_or(Error::NotFound)?;
            // An attachment Apple keeps at size 0 (an old drawing from 2016) has no bytes, and
            // its address answers 400 (coddingtonbear/icloud-md#55): it is empty, not refused.
            if media
                .pointer("/fields/Asset/value/size")
                .and_then(Value::as_u64)
                == Some(0)
            {
                return Ok(Vec::new());
            }
            let url = media
                .pointer("/fields/Asset/value/downloadURL")
                .and_then(Value::as_str)
                .ok_or(Error::NotFound)?;
            download(http, url).await
        }
    }
}

#[cfg(test)]
mod empty_asset_tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// An attachment whose file Apple keeps at size 0 opens empty; asked for, its address
    /// answers 400, and opening it said Apple would not hand it over.
    #[tokio::test]
    async fn an_empty_attachment_is_empty_not_refused() {
        let server = MockServer::start().await;
        let lookup = |name: &str, record: serde_json::Value| {
            Mock::given(method("POST"))
                .and(path_regex(r".*/records/lookup$"))
                .and(body_string_contains(name.to_string()))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({ "records": [record] })),
                )
        };
        lookup(
            "\"ATT\"",
            serde_json::json!({ "recordName": "ATT", "recordType": "Attachment", "recordChangeTag": "t1",
                "fields": { "Media": { "value": { "recordName": "MEDIA" } } } }),
        )
        .mount(&server)
        .await;
        lookup(
            "\"MEDIA\"",
            serde_json::json!({ "recordName": "MEDIA", "recordType": "Media", "recordChangeTag": "t2",
                "fields": { "Asset": { "value": { "size": 0,
                    "downloadURL": format!("{}/asset/${{f}}", server.uri()) } } } }),
        )
        .mount(&server)
        .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&server)
            .await;
        let got = bytes(
            &reqwest::Client::new(),
            &server.uri(),
            vec![],
            "ATT",
            Part::File,
        )
        .await
        .expect("an empty file, not a refusal");
        assert!(got.is_empty());
        let asked = server.received_requests().await.unwrap();
        assert!(
            asked.iter().all(|r| r.method.as_str() != "GET"),
            "nothing downloaded"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kinds measured on a real account, each drawn the way Notes draws it.
    #[test]
    fn each_kind_measured_is_drawn_as_notes_draws_it() {
        assert_eq!(kind_of("public.jpeg"), "image");
        assert_eq!(kind_of("com.apple.paper.doc.pdf"), "document");
        assert_eq!(kind_of("com.apple.paper.doc.scan"), "document");
        // Measured: an iOS 17 drawing in his "Colors" note is com.apple.paper, with a
        // FallbackImage (1536×1242 for a 768×621 drawing) and two previews.
        assert_eq!(kind_of("com.apple.paper"), "drawing");
        assert_eq!(kind_of("com.apple.drawing.2"), "drawing");
        // A table is never a file to open, even one the table reader cannot read.
        assert_eq!(kind_of("com.apple.notes.table"), "table");
        assert_eq!(kind_of("com.apple.m4a-audio"), "audio");
        assert_eq!(kind_of("public.mpeg-4-audio"), "audio");
        assert_eq!(kind_of("com.apple.quicktime-movie"), "video");
        assert_eq!(kind_of("org.openxmlformats.spreadsheetml.sheet"), "file");
        assert_eq!(
            kind_of("dyn.ah62d4rv4ge80455crf0gn3dfs3y0g3puqfy1e4psq6"),
            "file"
        );
    }

    /// A tag shows the words it was typed as; with only its token, the token behind a `#`.
    #[test]
    fn a_tag_is_drawn_as_its_words() {
        use base64::Engine;
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
        let uti = "com.apple.notes.inlinetextattachment.hashtag";
        assert_eq!(kind_of(uti), "inline");
        assert_eq!(
            kind_of("com.apple.notes.inlinetextattachment.mention"),
            "inline"
        );
        let typed = serde_json::json!({ "fields": {
            "AltTextEncrypted": { "value": b64("#Recipes") },
            "TokenContentIdentifierEncrypted": { "value": b64("RECIPES") },
        }});
        let info = inline_info("A", uti.into(), &typed);
        assert_eq!(
            (info.kind, info.title.as_deref()),
            ("inline", Some("#Recipes"))
        );
        let token = serde_json::json!({ "fields": {
            "TokenContentIdentifier": { "value": "RECIPES" },
        }});
        assert_eq!(
            inline_info("A", uti.into(), &token).title.as_deref(),
            Some("#RECIPES")
        );
        let bare = serde_json::json!({ "fields": {} });
        assert_eq!(inline_info("A", uti.into(), &bare).title, None);
    }
}
