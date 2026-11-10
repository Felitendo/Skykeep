package io.github.yashwanthkamireddi.skykeep.sync

import android.content.Context
import android.content.SharedPreferences

/**
 * What the last sync of Contacts and of Calendar did, kept for the app's screen: Android runs
 * the syncs (the adapters in Services.kt) and says only whether one is running, never how it
 * went. The adapters write it here; the screen reads it and is told when it changes.
 */
object Outcome {
  private const val FILE = "sync-outcome"

  /** `okAt`: the last time it synced cleanly, which the error wording names. */
  data class Last(val at: Long, val ok: Boolean, val said: String, val okAt: Long)

  private fun prefs(context: Context): SharedPreferences =
    context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)

  fun record(context: Context, authority: String, ok: Boolean, said: String, held: Int = 0) {
    prefs(context).edit()
      .putLong("$authority.at", System.currentTimeMillis())
      .putBoolean("$authority.ok", ok)
      .putString("$authority.said", said)
      .putInt("$authority.held", held)
      .apply { if (ok) putLong("$authority.okAt", System.currentTimeMillis()) }
      .apply()
  }

  /** How many cards the last sync held back (iCloud listed far fewer than the phone has). */
  fun held(context: Context, authority: String): Int = prefs(context).getInt("$authority.held", 0)

  /** The person confirmed those removals: the next sync makes them, once. */
  fun confirmMassRemoval(context: Context, authority: String) {
    prefs(context).edit().putBoolean("$authority.confirmed", true).apply()
  }

  /** Whether removals were confirmed — and, asked, the confirmation is spent. */
  fun massRemovalConfirmed(context: Context, authority: String): Boolean {
    val p = prefs(context)
    val yes = p.getBoolean("$authority.confirmed", false)
    if (yes) p.edit().putBoolean("$authority.confirmed", false).apply()
    return yes
  }

  fun last(context: Context, authority: String): Last? {
    val p = prefs(context)
    val said = p.getString("$authority.said", null) ?: return null
    return Last(p.getLong("$authority.at", 0), p.getBoolean("$authority.ok", false), said, p.getLong("$authority.okAt", 0))
  }

  /** Told of every new outcome until the returned function is called. */
  fun watch(context: Context, changed: () -> Unit): () -> Unit {
    val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> changed() }
    val p = prefs(context)
    p.registerOnSharedPreferenceChangeListener(listener)
    return { p.unregisterOnSharedPreferenceChangeListener(listener) }
  }
}
