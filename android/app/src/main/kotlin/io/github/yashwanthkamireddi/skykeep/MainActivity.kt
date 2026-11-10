package io.github.yashwanthkamireddi.skykeep

import android.Manifest
import android.content.ContentResolver
import android.content.pm.PackageManager
import android.provider.CalendarContract
import android.provider.ContactsContract
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import io.github.yashwanthkamireddi.skykeep.sync.Outcome
import io.github.yashwanthkamireddi.skykeep.sync.ensureAccount
import io.github.yashwanthkamireddi.skykeep.ui.theme.SkykeepTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.Account

private enum class Phase { Starting, SignedOut, SignedIn }

class MainActivity : ComponentActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    enableEdgeToEdge()
    setContent {
      SkykeepTheme {
        Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
          var phase by remember { mutableStateOf(Phase.Starting) }
          var account by remember { mutableStateOf<Account?>(null) }
          LaunchedEffect(Unit) {
            val (a, resumed) = withContext(Dispatchers.IO) {
              val a = Engine.account(applicationContext)
              a to runCatching { a.resume() }.getOrDefault(false)
            }
            account = a
            phase = if (resumed) Phase.SignedIn else Phase.SignedOut
          }
          when (phase) {
            Phase.Starting -> CircularProgressIndicator(Modifier.padding(96.dp))
            Phase.SignedOut -> SignInScreen(account!!) { phase = Phase.SignedIn }
            Phase.SignedIn -> SignedIn(account!!)
          }
        }
      }
    }
  }
}

/**
 * Signed in: iCloud's contacts and calendars in Android's own apps. The button asks Android to
 * sync now — the adapters (sync/Services.kt) are the one place a sync runs, so a tap never runs
 * a second one beside Android's — and the screen follows Android's sync while it runs, then
 * says what each last did (sync/Outcome.kt).
 */
@Composable
private fun SignedIn(account: Account) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  var status by remember { mutableStateOf<String?>(null) }
  var busy by remember { mutableStateOf(false) }
  val authorities = listOf(ContactsContract.AUTHORITY, CalendarContract.AUTHORITY)
  // Made by an earlier tap: its last outcomes shown at once.
  var androidAccount by remember {
    mutableStateOf(android.accounts.AccountManager.get(context).getAccountsByType(io.github.yashwanthkamireddi.skykeep.sync.ACCOUNT_TYPE).firstOrNull())
  }
  // When this screen last asked for a sync: busy until both have answered since, or two minutes
  // pass. Not `isSyncPending`, which counts the hourly syncs Android always holds as pending —
  // the button stayed disabled for good.
  var asked by remember { mutableStateOf(0L) }
  // Contacts iCloud no longer lists, held on the phone by the circuit breaker until confirmed.
  var held by remember { mutableStateOf(0) }
  fun refresh() {
    val a = androidAccount ?: return
    val waiting = asked > 0 && System.currentTimeMillis() - asked < 120_000 &&
      authorities.any { (Outcome.last(context, it)?.at ?: 0) < asked }
    busy = waiting || authorities.any { ContentResolver.isSyncActive(a, it) }
    status = authorities.mapNotNull { Outcome.last(context, it)?.said }.joinToString("\n").ifEmpty { null }
    held = Outcome.held(context, ContactsContract.AUTHORITY)
  }
  DisposableEffect(androidAccount) {
    val handle = ContentResolver.addStatusChangeListener(
      ContentResolver.SYNC_OBSERVER_TYPE_ACTIVE or ContentResolver.SYNC_OBSERVER_TYPE_PENDING,
    ) { scope.launch { refresh() } }
    val unwatch = Outcome.watch(context) { scope.launch { refresh() } }
    refresh()
    onDispose {
      ContentResolver.removeStatusChangeListener(handle)
      unwatch()
    }
  }
  fun syncNow() {
    busy = true
    asked = System.currentTimeMillis()
    scope.launch {
      val a = withContext(Dispatchers.IO) { ensureAccount(context, account.appleId() ?: "iCloud") }
      androidAccount = a
      val now = Bundle().apply {
        putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
        putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
      }
      for (authority in authorities) ContentResolver.requestSync(a, authority, now)
      refresh()
      // Never left waiting on an answer that does not come.
      kotlinx.coroutines.delay(120_000)
      refresh()
    }
  }
  val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
    if (granted.values.all { it }) syncNow() else status = "Contacts and Calendar access is needed to show iCloud's in them."
  }
  val needed = arrayOf(
    Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS,
    Manifest.permission.READ_CALENDAR, Manifest.permission.WRITE_CALENDAR,
  )
  val allowed = needed.all { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }
  Column(Modifier.padding(32.dp).padding(top = 64.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
    Text("Signed in", style = MaterialTheme.typography.headlineSmall)
    account.appleId()?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
    Button(
      enabled = !busy,
      onClick = {
        if (allowed) syncNow()
        else ask.launch(needed)
      },
    ) { Text("Show iCloud in Contacts and Calendar") }
    if (held > 0 && !busy) {
      OutlinedButton(onClick = {
        Outcome.confirmMassRemoval(context, ContactsContract.AUTHORITY)
        syncNow()
      }) { Text("Confirm $held held contact changes") }
    }
    if (busy) CircularProgressIndicator()
    status?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
  }
}
