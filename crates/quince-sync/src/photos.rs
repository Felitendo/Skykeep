//! Photos: a pass from Apple's library stored a few pages at a time, as the desktop reads it
//! (src-tauri/src/refresh.rs): each round stored before the next is asked for, so a library of
//! any size is never held in memory whole; a library Apple is still preparing is not read (a
//! whole read of one part-made takes what it doesn't see as gone); a pass from the start marked,
//! so the next ones carry on from Apple's token.

use crate::library_lock::LibraryLock;
use quince_icloud::photos::{self as p, Links};
use quince_icloud::Error;
use std::collections::HashMap;

/// What a pass came to.
pub struct Pass {
    /// Records Apple sent in all its rounds.
    pub records: usize,
    /// Apple has more than this pass's rounds took: the next pass carries on.
    pub more: bool,
    /// Apple is still preparing the library: nothing was read, try again later.
    pub preparing: bool,
    /// Download links Apple sent with the records, by master id: good for a while.
    pub links: HashMap<String, Links>,
}

/// One round's records stored; a round of a pass from the start marks the library as having had
/// one. Answers how many records it held and their links.
pub fn apply(
    library: &mut quince_core::Library,
    fetched: p::PhotosFetch,
    full: bool,
) -> Result<(usize, HashMap<String, Links>), Error> {
    let records = fetched.records.len();
    let (_, links) = p::apply(library, fetched)?;
    if full {
        library.mark_full_sync(p::SERVICE)?;
    }
    Ok((records, links))
}

/// A pass from Apple: up to [`p::PASS_ROUNDS`] rounds, each stored before the next is asked for.
///
/// # Errors
/// When Apple can't be reached or refuses, or the library can't be written.
pub async fn refresh(
    library: &LibraryLock,
    http: &reqwest::Client,
    root: &str,
    params: Vec<(String, String)>,
) -> Result<Pass, Error> {
    let mut pass = Pass {
        records: 0,
        more: false,
        preparing: false,
        links: HashMap::new(),
    };
    for round in 1..=p::PASS_ROUNDS {
        let since = library.lock().await.sync_token(p::SERVICE)?;
        let full = since.is_none();
        if full && round == 1 {
            if let Ok(p::Readiness::Preparing { .. }) =
                p::readiness(http, root, params.clone()).await
            {
                pass.preparing = true;
                return Ok(pass);
            }
        }
        let fetched = p::fetch(http, root, params.clone(), since.as_deref()).await?;
        let complete = fetched.complete;
        let (records, links) = {
            let mut held = library.lock().await;
            apply(&mut held, fetched, full)?
        };
        pass.records += records;
        pass.links.extend(links);
        if complete {
            return Ok(pass);
        }
    }
    pass.more = true;
    Ok(pass)
}
