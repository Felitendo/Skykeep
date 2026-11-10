package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.accounts.AccountManager
import android.content.ContentResolver
import android.content.Context
import android.provider.CalendarContract
import android.provider.ContactsContract

/** The account type Android files Skykeep's contacts and calendars under. */
const val ACCOUNT_TYPE = "io.github.yashwanthkamireddi.skykeep.account"

/** How often Android syncs Contacts and Calendar on its own: hourly, gentle on the battery. */
private const val SYNC_EVERY_SECONDS = 60L * 60

/**
 * The Skykeep account for an Apple Account, created once and made syncable for Contacts and
 * Calendar. No password is kept in it: the session lives sealed in the Keystore.
 */
fun ensureAccount(context: Context, appleId: String): Account {
  val account = Account(appleId, ACCOUNT_TYPE)
  val manager = AccountManager.get(context)
  if (manager.getAccountsByType(ACCOUNT_TYPE).none { it.name == appleId }) {
    manager.addAccountExplicitly(account, null, null)
  }
  for (authority in listOf(ContactsContract.AUTHORITY, CalendarContract.AUTHORITY)) {
    ContentResolver.setIsSyncable(account, authority, 1)
    ContentResolver.setSyncAutomatically(account, authority, true)
    // iCloud sends no push a third-party app can receive, so Android is asked to sync on a
    // schedule; it batches these with other apps' syncs and skips them without a network.
    ContentResolver.addPeriodicSync(account, authority, android.os.Bundle.EMPTY, SYNC_EVERY_SECONDS)
  }
  return account
}
