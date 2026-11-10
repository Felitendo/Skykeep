package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.AbstractAccountAuthenticator
import android.accounts.Account
import android.accounts.AccountAuthenticatorResponse
import android.app.Service
import android.content.AbstractThreadedSyncAdapter
import android.content.ContentProviderClient
import android.content.Context
import android.content.Intent
import android.content.SyncResult
import android.os.Bundle
import android.os.IBinder

/**
 * Skykeep's account as Android's account system sees it. Accounts are added by the app after
 * signing in, never from Settings' Add account, so every request here is declined plainly.
 */
private class Authenticator(context: Context) : AbstractAccountAuthenticator(context) {
  override fun editProperties(r: AccountAuthenticatorResponse?, t: String?) = null
  override fun addAccount(r: AccountAuthenticatorResponse?, t: String?, a: String?, f: Array<out String>?, o: Bundle?) = null
  override fun confirmCredentials(r: AccountAuthenticatorResponse?, a: Account?, o: Bundle?) = null
  override fun getAuthToken(r: AccountAuthenticatorResponse?, a: Account?, t: String?, o: Bundle?) = null
  override fun getAuthTokenLabel(t: String?) = null
  override fun updateCredentials(r: AccountAuthenticatorResponse?, a: Account?, t: String?, o: Bundle?) = null
  override fun hasFeatures(r: AccountAuthenticatorResponse?, a: Account?, f: Array<out String>?) =
    Bundle().apply { putBoolean(android.accounts.AccountManager.KEY_BOOLEAN_RESULT, false) }
}

class AuthenticatorService : Service() {
  private val authenticator by lazy { Authenticator(this) }
  override fun onBind(intent: Intent): IBinder = authenticator.iBinder
}

/**
 * Android asking for Calendar: iCloud's calendars and a year of events written in. The only
 * place a calendar sync runs — the app's button asks Android for one rather than running its
 * own beside it, so Apple is asked once, in turn, with Android's backoff after a failure.
 */
private class Adapter(context: Context) : AbstractThreadedSyncAdapter(context, true) {
  override fun onPerformSync(a: Account, e: Bundle, authority: String, p: ContentProviderClient, r: SyncResult) {
    try {
      val (from, to) = CalendarSync.span()
      val window = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
        .calendarWindow(from, to, java.util.TimeZone.getDefault().id)
      val done = CalendarSync.sync(context.contentResolver, a, window, from, to)
      r.stats.numInserts += done.added.toLong()
      r.stats.numUpdates += done.updated.toLong()
      r.stats.numDeletes += done.removed.toLong()
      Outcome.record(context, authority, true,
        "${done.calendars} calendars, ${window.events.size} events · ${done.added} added, ${done.updated} updated, ${done.removed} removed")
    } catch (x: uniffi.quince_mobile.EngineException) {
      // Apple busy or unreachable, or the session gone: Android tries again later, backing off.
      r.stats.numIoExceptions++
      Outcome.record(context, authority, false, (x as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "Calendar couldn't be read.")
    }
  }
}

/** Android asking for Contacts: iCloud's address book read through the engine and written in. */
private class ContactsAdapter(context: Context) : AbstractThreadedSyncAdapter(context, true) {
  override fun onPerformSync(a: Account, e: Bundle, authority: String, p: ContentProviderClient, r: SyncResult) {
    try {
      val people = io.github.yashwanthkamireddi.skykeep.Engine.account(context).contacts()
      val done = ContactsSync.sync(context.contentResolver, a, people)
      r.stats.numInserts += done.added.toLong()
      r.stats.numUpdates += done.updated.toLong()
      r.stats.numDeletes += done.removed.toLong()
      Outcome.record(context, authority, true,
        "${people.size} contacts · ${done.added} added, ${done.updated} updated, ${done.removed} removed")
    } catch (x: uniffi.quince_mobile.EngineException) {
      // Apple busy or unreachable, or the session gone: Android tries again later, backing off.
      r.stats.numIoExceptions++
      Outcome.record(context, authority, false, (x as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "Contacts couldn't be read.")
    }
  }
}

class ContactsSyncService : Service() {
  private val adapter by lazy { ContactsAdapter(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

class CalendarSyncService : Service() {
  private val adapter by lazy { Adapter(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}
