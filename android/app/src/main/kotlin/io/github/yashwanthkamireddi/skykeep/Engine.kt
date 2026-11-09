package io.github.yashwanthkamireddi.skykeep

import android.content.Context
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.start

/**
 * The engine, once per process: the app's screens and Android's sync adapters share one
 * signed-in Account, so a sync started by Android uses the same session the app shows.
 */
object Engine {
  @Volatile private var account: Account? = null

  fun account(context: Context): Account = account ?: synchronized(this) {
    account ?: run {
      val app = context.applicationContext
      start(KeystoreSecrets(app))
      Account(app.filesDir.resolve("account").path).also { account = it }
    }
  }
}
