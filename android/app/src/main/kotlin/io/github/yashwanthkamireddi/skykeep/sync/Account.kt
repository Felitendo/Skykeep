package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.accounts.AccountManager
import android.content.ContentResolver
import android.content.Context
import android.provider.CalendarContract
import android.provider.ContactsContract

/** The account type Android files Skykeep's contacts and calendars under. */
const val ACCOUNT_TYPE = "io.github.yashwanthkamireddi.skykeep.account"

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
  }
  return account
}
