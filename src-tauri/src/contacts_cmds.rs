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
    let library = data.0.lock().await;
    library.contact_addresses(&guid).map_err(|e| e.to_string())
}

/// Change a card's addresses: set a part, set a label, add one or take one out — any number
/// at once, in order. `base` is the list the form opened (the stored list when absent); the
/// edits are laid over Apple's addresses as they stand at the moment of writing, so a part
/// changed on another device stays. Shown here at once; on a refusal Apple's card is read
/// back, so what is on screen is never something Apple does not have. Answers the list.
#[tauri::command]
pub async fn contact_address_edit(
    guid: String,
    base: Option<Vec<PostalAddress>>,
    edits: Vec<AddressEdit>,
    auth: State<'_, crate::Auth>,
    data: State<'_, Data>,
) -> UiResult<Vec<PostalAddress>> {
    let _one = crate::CONTACTS_PASS.lock().await;
    let (http, root, params) = crate::contacts_context(&auth).await?;
    let base = match base {
        Some(base) => base,
        None => {
            let library = data.0.lock().await;
            library
                .contact_addresses(&guid)
                .map_err(|e| e.to_string())?
        }
    };
    // Checked, and shown, before anything is sent.
    let (after, _) = address::apply_edits(&base, &edits).map_err(|e| e.to_string())?;
    {
        let mut library = data.0.lock().await;
        library
            .set_contact_addresses(&guid, &after)
            .map_err(|e| e.to_string())?;
    }
    match address::edit(&http, &root, params.clone(), &guid, &base, &edits).await {
        Ok(written) => {
            crate::settle(crate::resync_contacts(&http, &root, params, &data).await)?;
            Ok(written)
        }
        Err(e) => {
            // Put back what Apple has.
            let _ = crate::resync_contacts(&http, &root, params, &data).await;
            Err(e.to_string())
        }
    }
}
