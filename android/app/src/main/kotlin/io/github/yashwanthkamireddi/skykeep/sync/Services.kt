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

/** Where Android asks for a sync. Phase 1 fills these in with the engine's CardDAV and CalDAV. */
private class Adapter(context: Context) : AbstractThreadedSyncAdapter(context, true) {
  override fun onPerformSync(a: Account, e: Bundle, authority: String, p: ContentProviderClient, r: SyncResult) = Unit
}

class ContactsSyncService : Service() {
  private val adapter by lazy { Adapter(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

class CalendarSyncService : Service() {
  private val adapter by lazy { Adapter(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}
