//! Videos played in the window. WebKitGTK's player (GStreamer) reads only http(s), blob and
//! file sources — never a custom scheme like `quince://` ("No URI handler implemented") — and
//! a page of ours may not load `file://`. So the disk cache's videos are served over HTTP on
//! the loopback interface: a port of the system's choosing, a fresh secret in every path, only
//! cached videos by asset id, in the ranges the player asks for, streamed from disk.
//!
//! Only the window can read it. The port is open to every program and every user on this
//! computer, and to any web page a browser here shows, so a request is answered only when it
//! carries the secret (122 random bits, made at each start and given to the window alone), is
//! addressed to `127.0.0.1:<port>` by name (a page that rebinds its own name to this computer
//! still says its own name), and comes from no web page (no `Origin` but the app's). No CORS
//! header is ever sent, so a page that guessed everything still could not read the answer.

use crate::photos::{self, Film, Size};
use std::sync::OnceLock;
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// `http://127.0.0.1:<port>/<secret>`, once the server is listening.
static BASE: OnceLock<String> = OnceLock::new();

/// Where the window's player fetches a video from, or `None` if the server could not start
/// (the viewer then offers the system's player).
#[tauri::command]
pub fn video_base() -> Option<String> {
    BASE.get().cloned()
}

/// What the window's player can decode on this computer, so a video it cannot play says why
/// rather than standing black: the GStreamer pieces WebKitGTK plays through, by the package that
/// brings each. `missing` lists those packages; empty when everything is there (and always
/// elsewhere than Linux, where the webview brings its own decoders).
#[derive(Debug, Clone, serde::Serialize)]
pub struct VideoSupport {
    pub h264: bool,
    pub hevc: bool,
    pub missing: Vec<&'static str>,
}

#[tauri::command]
pub fn video_support() -> VideoSupport {
    support()
}

#[cfg(target_os = "linux")]
fn support() -> VideoSupport {
    use gstreamer as gst;
    if gst::init().is_err() {
        return VideoSupport {
            h264: false,
            hevc: false,
            missing: vec!["gstreamer"],
        };
    }
    let any = |names: &[&str]| names.iter().any(|n| gst::ElementFactory::find(n).is_some());
    let h264 = any(&[
        "avdec_h264",
        "vah264dec",
        "vaapih264dec",
        "openh264dec",
        "nvh264dec",
        "v4l2slh264dec",
    ]);
    let hevc = any(&[
        "avdec_h265",
        "vah265dec",
        "vaapih265dec",
        "nvh265dec",
        "v4l2slh265dec",
    ]);
    let mut missing = Vec::new();
    if !any(&["playbin3", "videoconvert"]) {
        missing.push("gst-plugins-base");
    }
    if !any(&["qtdemux"]) {
        missing.push("gst-plugins-good");
    }
    if !h264 || !hevc {
        missing.push("gst-libav");
    }
    VideoSupport {
        h264,
        hevc,
        missing,
    }
}

#[cfg(not(target_os = "linux"))]
fn support() -> VideoSupport {
    VideoSupport {
        h264: true,
        hevc: true,
        missing: Vec::new(),
    }
}

/// Listens on the loopback interface for as long as the app runs.
pub fn serve(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)).await else {
            return;
        };
        let Ok(addr) = listener.local_addr() else {
            return;
        };
        let secret = uuid::Uuid::new_v4().simple().to_string();
        let _ = BASE.set(format!("http://127.0.0.1:{}/{secret}", addr.port()));
        listen(listener, secret, move |id, size| {
            let app = app.clone();
            async move { photos::film(&app, &id, size).await }
        })
        .await;
    });
}

/// Answers each connection on its own task; `find` gives a video: kept on disk, or landing.
async fn listen<F, Fut>(listener: TcpListener, secret: String, find: F)
where
    F: Fn(String, Size) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Film, String>> + Send,
{
    let Ok(addr) = listener.local_addr() else {
        return;
    };
    let host = format!("127.0.0.1:{}", addr.port());
    while let Ok((stream, peer)) = listener.accept().await {
        // Bound to loopback, so nothing else can connect; checked all the same.
        if !peer.ip().is_loopback() {
            continue;
        }
        let (find, secret, host) = (find.clone(), secret.clone(), host.clone());
        tauri::async_runtime::spawn(async move {
            let _ = answer(
                find,
                &Gate {
                    secret: &secret,
                    host: &host,
                },
                stream,
            )
            .await;
        });
    }
}

/// What one request asks for: the video and its size, and the Range header if any.
#[derive(Debug, PartialEq)]
struct Asked {
    id: String,
    size: Size,
    range: Option<String>,
    head: bool,
}

/// What a request must show to be answered: the secret in its path, and this server's own
/// address as its `Host`.
struct Gate<'a> {
    secret: &'a str,
    host: &'a str,
}

/// The same bytes, compared in a time that does not tell how many matched.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Every value a header is given, in order: a request that names two hosts is refused, not
/// read one way here and another elsewhere.
fn headers<'a>(lines: &'a [String], name: &str) -> Vec<&'a str> {
    lines
        .iter()
        .skip(1)
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then_some(v.trim())
        })
        .collect()
}

/// The request line and headers, read up to the blank line; anything not ours is `None`.
fn parse(gate: &Gate<'_>, lines: &[String]) -> Option<Asked> {
    let mut first = lines.first()?.split_whitespace();
    let method = first.next()?;
    let head = match method {
        "GET" => false,
        "HEAD" => true,
        _ => return None,
    };
    let path = first.next()?.split('?').next()?;
    // Addressed to this server by its own address: a web page whose name was rebound to this
    // computer (DNS rebinding) still sends its own name.
    match headers(lines, "host").as_slice() {
        [host] if same(host.as_bytes(), gate.host.as_bytes()) => {}
        _ => return None,
    }
    // Asked by the window's player, never by a web page: one sends an `Origin` of its own.
    if headers(lines, "origin")
        .iter()
        .any(|o| !matches!(*o, "tauri://localhost" | "http://tauri.localhost"))
    {
        return None;
    }
    let rest = path.strip_prefix('/')?;
    let (key, rest) = rest.split_at_checked(gate.secret.len())?;
    if !same(key.as_bytes(), gate.secret.as_bytes()) {
        return None;
    }
    let (id, size) = photos::requested(&format!("pic{rest}"))?;
    if !matches!(size, Size::Original | Size::Video) {
        return None;
    }
    let range = headers(lines, "range").first().map(|v| (*v).to_string());
    Some(Asked {
        id,
        size,
        range,
        head,
    })
}

async fn answer<F, Fut>(find: F, gate: &Gate<'_>, stream: TcpStream) -> std::io::Result<()>
where
    F: Fn(String, Size) -> Fut,
    Fut: std::future::Future<Output = Result<Film, String>>,
{
    let mut reader = BufReader::new(stream);
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        // Each line read only so far: an endless one is cut off, not held.
        let read = (&mut reader).take(8192).read_line(&mut line).await?;
        if read == 0 || lines.len() > 64 || !line.ends_with('\n') {
            return Ok(());
        }
        let line = line.trim_end().to_string();
        if line.is_empty() {
            break;
        }
        lines.push(line);
    }
    let mut stream = reader.into_inner();
    let Some(asked) = parse(gate, &lines) else {
        return status(&mut stream, "404 Not Found", "").await;
    };
    let Ok(film) = find(asked.id.clone(), asked.size).await else {
        return status(&mut stream, "404 Not Found", "").await;
    };
    // A film still landing is served as it lands: its length is known, and each piece is
    // waited for only as the player reaches it — it starts at once, not after the download.
    let (mut file, len, growing, path) = match film {
        Film::Whole(path) => {
            let file = tokio::fs::File::open(&path).await?;
            let len = file.metadata().await?.len();
            (file, len, None, path)
        }
        Film::Growing(g) => {
            // Just finished and renamed: then it is where it is kept.
            let file = match tokio::fs::File::open(&g.part).await {
                Ok(f) => f,
                Err(_) => tokio::fs::File::open(&g.kept).await?,
            };
            if g.wait_for(12.min(g.total)).await.is_none() {
                return status(&mut stream, "404 Not Found", "").await;
            }
            let kept = g.kept.clone();
            (file, g.total, Some(g), kept)
        }
    };
    let mut head = [0u8; 12];
    let _ = file.read(&mut head).await;
    let kind = photos::video_type(&head);
    let Some((first, last)) = photos::byte_range(asked.range.as_deref(), len, u64::MAX) else {
        let extra = format!("Content-Range: bytes */{len}\r\n");
        return status(&mut stream, "416 Range Not Satisfiable", &extra).await;
    };
    let n = last - first + 1;
    let line = if asked.range.is_some() {
        "206 Partial Content"
    } else {
        "200 OK"
    };
    let headers = format!(
        "HTTP/1.1 {line}\r\nContent-Type: {kind}\r\nContent-Length: {n}\r\nAccept-Ranges: bytes\r\n\
         Content-Range: bytes {first}-{last}/{len}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(headers.as_bytes()).await?;
    if !asked.head {
        file.seek(std::io::SeekFrom::Start(first)).await?;
        // The film's turn is drawn by the window (photo_video_turn), the same on every machine:
        // WebKitGTK honoured it in one way of drawing and not another, and a front-camera Live
        // Photo played sideways. So the film goes out upright — its matrix's 36 bytes, alone,
        // replaced as they pass. The file on disk is untouched.
        let at = match &growing {
            None => {
                let path = path.clone();
                tokio::task::spawn_blocking(move || {
                    quince_media::video_matrix(&path).ok().flatten()
                })
                .await
                .ok()
                .flatten()
            }
            Some(g) => g.matrix().await.ok().flatten(),
        }
        .filter(|(_, m)| *m != [1, 0, 0, 1])
        .map(|(at, _)| at);
        // Streamed from disk: a long film is never held in memory, and a player that stops
        // reading (a seek elsewhere) just closes the connection.
        let mut pos = first;
        let mut left = n;
        let mut chunk = vec![0u8; 64 * 1024];
        while left > 0 {
            let mut want = left.min(chunk.len() as u64);
            if let Some(g) = &growing {
                let Some(have) = g.wait_for(pos + want).await else {
                    break;
                };
                want = want.min(have.saturating_sub(pos));
            }
            let want = usize::try_from(want).unwrap_or(chunk.len());
            let got = file.read(&mut chunk[..want]).await?;
            if got == 0 {
                break;
            }
            if let Some(at) = at {
                upright(&mut chunk[..got], pos, at);
            }
            stream.write_all(&chunk[..got]).await?;
            pos += got as u64;
            left -= got as u64;
        }
    }
    stream.shutdown().await
}

/// The part of an upright matrix that falls in `chunk`, which starts at `pos` in the file whose
/// matrix starts at `at`.
fn upright(chunk: &mut [u8], pos: u64, at: u64) {
    let end = pos + chunk.len() as u64;
    let (from, to) = (at.max(pos), (at + 36).min(end));
    for i in from..to {
        chunk[usize::try_from(i - pos).unwrap_or(0)] =
            quince_media::UPRIGHT_MATRIX[usize::try_from(i - at).unwrap_or(0)];
    }
}

async fn status(stream: &mut TcpStream, line: &str, extra: &str) -> std::io::Result<()> {
    let text = format!("HTTP/1.1 {line}\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(text.as_bytes()).await?;
    stream.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One raw request to the server, and its whole answer: head and body.
    /// Sent as the window's player sends it, addressed to the server by its own address — unless
    /// the request names a host of its own.
    async fn get(port: u16, request: &str) -> (String, Vec<u8>) {
        let request = if request.to_ascii_lowercase().contains("\r\nhost:") {
            request.to_string()
        } else {
            request.replacen("\r\n", &format!("\r\nHost: 127.0.0.1:{port}\r\n"), 1)
        };
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(request.as_bytes()).await.unwrap();
        let mut all = Vec::new();
        s.read_to_end(&mut all).await.unwrap();
        let at = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        (
            String::from_utf8_lossy(&all[..at]).into_owned(),
            all[at + 4..].to_vec(),
        )
    }

    #[test]
    fn serves_a_cached_video_in_ranges() {
        tauri::async_runtime::block_on(async {
            let dir = std::env::temp_dir().join(format!("quince-video-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("clip");
            let bytes: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
            std::fs::write(&file, &bytes).unwrap();
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let found = file.clone();
            tauri::async_runtime::spawn(listen(listener, "k3y".into(), move |id, _| {
                let found = found.clone();
                async move {
                    if id == "v1" {
                        Ok(Film::Whole(found))
                    } else {
                        Err("none".into())
                    }
                }
            }));

            let (head, body) = get(port, "GET /k3y/v1/original HTTP/1.1\r\n\r\n").await;
            assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
            assert!(
                head.contains("Accept-Ranges: bytes") && head.contains("Content-Type: video/mp4")
            );
            assert_eq!(body, bytes);

            let (head, body) = get(
                port,
                "GET /k3y/v1/original HTTP/1.1\r\nRange: bytes=1000-1999\r\n\r\n",
            )
            .await;
            assert!(head.starts_with("HTTP/1.1 206 Partial Content"), "{head}");
            assert!(
                head.contains("Content-Range: bytes 1000-1999/200000"),
                "{head}"
            );
            assert_eq!(body, bytes[1000..2000]);

            let (head, body) = get(
                port,
                "GET /k3y/v1/original HTTP/1.1\r\nrange: bytes=-10\r\n\r\n",
            )
            .await;
            assert!(
                head.contains("Content-Range: bytes 199990-199999/200000"),
                "{head}"
            );
            assert_eq!(body, bytes[199_990..]);

            let (head, body) = get(port, "HEAD /k3y/v1/original HTTP/1.1\r\n\r\n").await;
            assert!(
                head.contains("Content-Length: 200000") && body.is_empty(),
                "{head}"
            );

            let (head, _) = get(
                port,
                "GET /k3y/v1/original HTTP/1.1\r\nRange: bytes=300000-\r\n\r\n",
            )
            .await;
            assert!(head.starts_with("HTTP/1.1 416"), "{head}");
            let (head, _) = get(port, "GET /nope/v1/original HTTP/1.1\r\n\r\n").await;
            assert!(head.starts_with("HTTP/1.1 404"), "{head}");
            let (head, _) = get(port, "GET /k3y/v2/original HTTP/1.1\r\n\r\n").await;
            assert!(head.starts_with("HTTP/1.1 404"), "{head}");
            // The secret alone is not enough: a page whose name was rebound to this computer,
            // a request with no host, a web page's own request — none is read.
            for refused in [
                "GET /k3y/v1/original HTTP/1.1\r\nHost: evil.example:80\r\n\r\n".to_string(),
                format!("GET /k3y/v1/original HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n"),
                format!(
                    "GET /k3y/v1/original HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
                     Host: evil.example\r\n\r\n"
                ),
                "GET /k3y/v1/original HTTP/1.0\r\nHost:\r\n\r\n".to_string(),
                "GET /k3y/v1/original HTTP/1.1\r\nOrigin: https://evil.example\r\n\r\n".to_string(),
            ] {
                let (head, body) = get(port, &refused).await;
                assert!(head.starts_with("HTTP/1.1 404"), "{refused}: {head}");
                assert!(
                    body.is_empty() && !head.contains("Access-Control"),
                    "{head}"
                );
            }
            let (head, _) = get(
                port,
                "GET /k3y/v1/original HTTP/1.1\r\nOrigin: tauri://localhost\r\n\r\n",
            )
            .await;
            assert!(head.starts_with("HTTP/1.1 200"), "{head}");
            std::fs::remove_dir_all(&dir).ok();
        });
    }

    /// A film still downloading plays at once: each range is answered as its bytes land, the
    /// end of it included, and the bytes are the film's whatever the pieces it lands in.
    #[test]
    fn serves_a_film_while_it_lands() {
        tauri::async_runtime::block_on(async {
            let dir = std::env::temp_dir().join(format!("quince-growing-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let (part, kept) = (dir.join("clip.part"), dir.join("clip"));
            let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 241) as u8).collect();
            std::fs::write(&part, b"").unwrap();
            let g = std::sync::Arc::new(photos::Growth::new(part.clone(), kept.clone(), 300_000));
            let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let shared = g.clone();
            tauri::async_runtime::spawn(listen(listener, "k".into(), move |_, _| {
                let g = shared.clone();
                async move { Ok(Film::Growing(g)) }
            }));
            // Lands in uneven pieces, a little at a time, then is renamed into place.
            let (lander, all) = (g.clone(), bytes.clone());
            tauri::async_runtime::spawn(async move {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&lander.part)
                    .unwrap();
                for piece in all.chunks(37_000) {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                    f.write_all(piece).unwrap();
                    f.flush().unwrap();
                    lander.landed(piece.len() as u64);
                }
                std::fs::rename(&lander.part, &lander.kept).unwrap();
                lander.ended(true);
            });
            let (head, body) = get(
                port,
                "GET /k/v/video HTTP/1.1\r\nRange: bytes=1000-1999\r\n\r\n",
            )
            .await;
            assert!(
                head.contains("Content-Range: bytes 1000-1999/300000"),
                "{head}"
            );
            assert_eq!(body, bytes[1000..2000]);
            let (head, body) = get(port, "GET /k/v/video HTTP/1.1\r\n\r\n").await;
            assert!(head.contains("Content-Length: 300000"), "{head}");
            assert_eq!(body, bytes);
            let (_, body) = get(port, "GET /k/v/video HTTP/1.1\r\nRange: bytes=-500\r\n\r\n").await;
            assert_eq!(body, bytes[299_500..]);
            std::fs::remove_dir_all(&dir).ok();
        });
    }

    /// A film is served upright whatever part of it is asked for: the matrix's bytes are
    /// replaced exactly where they fall, a range that starts or ends inside them included,
    /// and nothing else is touched.
    #[test]
    fn a_film_is_served_upright_in_any_range() {
        let film = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../crates/quince-media/tests/fixtures/flipturn.mp4");
        let bytes = std::fs::read(&film).unwrap();
        let (at, m) = quince_media::video_matrix(&film).unwrap().unwrap();
        assert_eq!(m, [0, 1, 1, 0]);
        let mut whole = bytes.clone();
        // Served in awkward pieces: one ends 10 bytes into the matrix.
        let cut = usize::try_from(at).unwrap() + 10;
        upright(&mut whole[..cut], 0, at);
        upright(&mut whole[cut..], cut as u64, at);
        let at = usize::try_from(at).unwrap();
        assert_eq!(&whole[at..at + 36], &quince_media::UPRIGHT_MATRIX[..]);
        assert_eq!(&whole[..at], &bytes[..at]);
        assert_eq!(&whole[at + 36..], &bytes[at + 36..]);
        let served = std::env::temp_dir().join("quince-upright.mp4");
        std::fs::write(&served, &whole).unwrap();
        assert_eq!(
            quince_media::video_matrix(&served).unwrap().map(|(_, m)| m),
            Some([1, 0, 0, 1])
        );
    }

    /// A request to a server whose secret is `s3cret`, addressed to it as the player does.
    fn ask(s: &[&str]) -> Option<Asked> {
        let mut l: Vec<String> = s.iter().map(|l| l.to_string()).collect();
        l.insert(1, "Host: 127.0.0.1:9".into());
        parse(
            &Gate {
                secret: "s3cret",
                host: "127.0.0.1:9",
            },
            &l,
        )
    }

    /// This computer's decoders, as the window is told them: H.264 at least wherever the
    /// player can play anything, and a package named for each piece that is missing.
    #[test]
    fn says_what_it_can_decode() {
        let s = support();
        assert_eq!(s.missing.contains(&"gst-libav"), !(s.h264 && s.hevc));
        println!("{s:?}");
    }

    #[test]
    fn only_our_secret_and_only_videos() {
        let ok = ask(&["GET /s3cret/abc-1/original HTTP/1.1", "Range: bytes=0-"]);
        assert_eq!(
            ok,
            Some(Asked {
                id: "abc-1".into(),
                size: Size::Original,
                range: Some("bytes=0-".into()),
                head: false
            })
        );
        let live = ask(&["HEAD /s3cret/abc/video HTTP/1.1"]).unwrap();
        assert!(live.head && live.size == Size::Video && live.range.is_none());
        // Someone else's guess, a picture, a path out of the cache, another method.
        assert_eq!(ask(&["GET /wrong/abc/original HTTP/1.1"]), None);
        assert_eq!(ask(&["GET /s3cret/abc/full HTTP/1.1"]), None);
        assert_eq!(ask(&["GET /s3cret/../x/original HTTP/1.1"]), None);
        assert_eq!(ask(&["GET /s3cret/a%2F..%2Fb/original HTTP/1.1"]), None);
        assert_eq!(ask(&["POST /s3cret/abc/original HTTP/1.1"]), None);
        assert_eq!(ask(&["GET /s3cretabc/original HTTP/1.1"]), None);
    }
}
