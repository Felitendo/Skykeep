//! A real FUSE mount of a made-up iCloud Drive, used the way any program uses a folder:
//! listing, stat, reading whole files and parts of them, a folder never listed fetched once,
//! and writes refused (read-only for now). Skipped where FUSE is not available.
#![cfg(target_os = "linux")]

use quince_core::drive::DriveItem;
use quince_vfs::{fuse, Cache, Fut, Remote, Store, CHUNK, ROOT};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn item(id: &str, parent: &str, name: &str, ext: Option<&str>, kind: &str, size: i64) -> DriveItem {
    DriveItem {
        drivewsid: id.into(),
        parent_id: Some(parent.into()),
        name: name.into(),
        extension: ext.map(Into::into),
        kind: kind.into(),
        size: Some(size),
        child_count: None,
        created_at: Some(1_700_000_000),
        modified_at: Some(1_700_000_100),
        etag: Some("e1".into()),
    }
}

struct Apple {
    folders: HashMap<String, Vec<DriveItem>>,
    files: HashMap<String, Vec<u8>>,
    lists: Mutex<Vec<String>>,
    reads: Mutex<Vec<(u64, u64)>>,
}
impl Remote for Apple {
    fn list(&self, folder: &str) -> Fut<Vec<DriveItem>> {
        self.lists.lock().unwrap().push(folder.to_string());
        let out = self.folders.get(folder).cloned().unwrap_or_default();
        Box::pin(async move { Ok(out) })
    }
    fn read(&self, item: &DriveItem, offset: u64, len: u64) -> Fut<Vec<u8>> {
        self.reads.lock().unwrap().push((offset, len));
        let data = &self.files[&item.drivewsid];
        let out = data[offset as usize..(offset + len) as usize].to_vec();
        Box::pin(async move { Ok(out) })
    }
}
/// The library: the root is known, "Work" never listed.
struct Library(HashMap<String, Vec<DriveItem>>);
impl Store for Library {
    fn children(&self, folder: &str) -> Option<Vec<DriveItem>> {
        self.0.get(folder).cloned()
    }
}

#[test]
fn a_mounted_drive_reads_like_a_folder() {
    if !std::path::Path::new("/dev/fuse").exists() {
        eprintln!("no FUSE here; skipped");
        return;
    }
    let film: Vec<u8> = (0..(CHUNK * 2 + 1234)).map(|i| (i % 253) as u8).collect();
    let note = b"hello from iCloud\n".to_vec();
    let mut work = item("FOLDER::z::work", ROOT, "Work", None, "FOLDER", 0);
    work.child_count = Some(1);
    let root = vec![
        work,
        item(
            "FILE::z::note",
            ROOT,
            "Note",
            Some("txt"),
            "FILE",
            note.len() as i64,
        ),
        item(
            "FILE::z::film",
            ROOT,
            "Film",
            Some("mov"),
            "FILE",
            film.len() as i64,
        ),
    ];
    let deck = item(
        "FILE::z::deck",
        "FOLDER::z::work",
        "Deck",
        Some("key"),
        "FILE",
        5,
    );
    let apple = Arc::new(Apple {
        folders: HashMap::from([("FOLDER::z::work".to_string(), vec![deck.clone()])]),
        files: HashMap::from([
            ("FILE::z::note".to_string(), note.clone()),
            ("FILE::z::film".to_string(), film.clone()),
            ("FILE::z::deck".to_string(), b"slide".to_vec()),
        ]),
        lists: Mutex::new(Vec::new()),
        reads: Mutex::new(Vec::new()),
    });
    let library = Arc::new(Library(HashMap::from([(ROOT.to_string(), root)])));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let cache_dir = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    let fs = fuse::DriveFs::new(
        apple.clone(),
        library,
        Arc::new(Cache::new(cache_dir.path()).unwrap()),
        rt.handle().clone(),
    );
    let session = fuse::mount(fs, at.path()).expect("mounted");

    let mut names: Vec<String> = std::fs::read_dir(at.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        ["Film.mov", "Note.txt", "Work"],
        "the root, from the library"
    );
    assert!(
        apple.lists.lock().unwrap().is_empty(),
        "nothing asked of Apple to list a known folder"
    );

    let meta = std::fs::metadata(at.path().join("Film.mov")).unwrap();
    assert_eq!(meta.len(), film.len() as u64);
    assert!(
        apple.reads.lock().unwrap().is_empty(),
        "a stat downloads nothing"
    );

    assert_eq!(
        std::fs::read(at.path().join("Note.txt")).unwrap(),
        note,
        "a small file reads whole"
    );

    // A part of a large file: only the chunk it is in is fetched.
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(at.path().join("Film.mov")).unwrap();
    f.seek(SeekFrom::Start(CHUNK + 10)).unwrap();
    let mut buf = [0u8; 100];
    f.read_exact(&mut buf).unwrap();
    assert_eq!(
        &buf[..],
        &film[(CHUNK + 10) as usize..(CHUNK + 110) as usize]
    );
    assert!(
        apple.reads.lock().unwrap().contains(&(CHUNK, CHUNK))
            && !apple.reads.lock().unwrap().contains(&(0, CHUNK)),
        "only the chunk read was fetched: {:?}",
        apple.reads.lock().unwrap()
    );
    assert_eq!(
        std::fs::read(at.path().join("Film.mov")).unwrap(),
        film,
        "and the whole film reads right"
    );

    // A folder never listed: fetched once from Apple, then known.
    let inner: Vec<String> = std::fs::read_dir(at.path().join("Work"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(inner, ["Deck.key"]);
    assert_eq!(
        std::fs::read(at.path().join("Work/Deck.key")).unwrap(),
        b"slide"
    );

    // Read-only, for now: a write is refused, never half-done.
    let err = std::fs::write(at.path().join("Note.txt"), b"x").unwrap_err();
    assert_eq!(err.raw_os_error(), Some(libc::EROFS), "{err}");
    assert_eq!(std::fs::read(at.path().join("Note.txt")).unwrap(), note);

    drop(session);
    assert_eq!(
        std::fs::read_dir(at.path()).unwrap().count(),
        0,
        "unmounted when the session ends"
    );
}
