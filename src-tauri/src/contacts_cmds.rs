//! Contacts commands: a card's postal addresses, part by part.

use crate::{Data, UiResult};
use quince_core::contacts::PostalAddress;
use quince_icloud::contacts::address::{self, AddressEdit};
use tauri::State;

/// One card's addresses, each part on its own, as the last pass (or edit) stored them.
#[tauri::command]
pub async fn contact_addresses(
    guid: String,
    data: State<'_, Data>,
) -> UiResult<Vec<PostalAddress>> {
    data.0
        .read(|library| library.contact_addresses(&guid))
        .await
        .map_err(|e| e.to_string())
}

/// Change a card's addresses: set a part, set a label, add one or take one out — any number
/// at once, in order. `base` is the list the form opened (the stored list when absent). Shown
/// here at once and queued (outbox.rs), like every other contact write: it works offline, and
/// on a card made here that Apple does not have yet it goes after the card itself. Sent, the
/// edits are laid over Apple's addresses as they stand then, so a part changed on another
/// device stays. Answers the list as it now shows.
#[tauri::command]
pub async fn contact_address_edit(
    guid: String,
    base: Option<Vec<PostalAddress>>,
    edits: Vec<AddressEdit>,
    data: State<'_, Data>,
) -> UiResult<Vec<PostalAddress>> {
    let after = {
        let mut library = data.0.lock().await;
        let base = match base {
            Some(base) => base,
            None => library
                .contact_addresses(&guid)
                .map_err(|e| e.to_string())?,
        };
        // Checked before anything is queued: an edit naming nothing is said now.
        address::apply_edits(&base, &edits).map_err(|e| e.to_string())?;
        crate::outbox::record_contact_address(&mut library, &guid, base, edits)
            .map_err(|e| e.to_string())?
    };
    crate::outbox::kick();
    Ok(after)
}
