package io.github.yashwanthkamireddi.skykeep

import android.Manifest
import android.content.pm.PackageManager
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
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
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
import io.github.yashwanthkamireddi.skykeep.sync.ContactsSync
import io.github.yashwanthkamireddi.skykeep.sync.ensureAccount
import io.github.yashwanthkamireddi.skykeep.ui.theme.SkykeepTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.EngineException

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

/** Signed in: iCloud contacts into Android's Contacts, once Contacts access is allowed. */
@Composable
private fun SignedIn(account: Account) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  var status by remember { mutableStateOf<String?>(null) }
  var busy by remember { mutableStateOf(false) }
  fun syncNow() {
    busy = true
    scope.launch {
      status = withContext(Dispatchers.IO) {
        try {
          val id = account.appleId() ?: "iCloud"
          val androidAccount = ensureAccount(context, id)
          val people = account.contacts()
          val r = ContactsSync.sync(context.contentResolver, androidAccount, people)
          "${people.size} contacts in Android's Contacts · ${r.added} added, ${r.updated} updated, ${r.removed} removed"
        } catch (e: EngineException.Failed) { e.reason }
      }
      busy = false
    }
  }
  val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
    if (granted.values.all { it }) syncNow() else status = "Contacts access is needed to show iCloud contacts in Contacts."
  }
  val allowed = ContextCompat.checkSelfPermission(context, Manifest.permission.WRITE_CONTACTS) == PackageManager.PERMISSION_GRANTED
  Column(Modifier.padding(32.dp).padding(top = 64.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
    Text("Signed in", style = MaterialTheme.typography.headlineSmall)
    account.appleId()?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
    Button(
      enabled = !busy,
      onClick = {
        if (allowed) syncNow()
        else ask.launch(arrayOf(Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS))
      },
    ) { Text("Show iCloud contacts in Contacts") }
    if (busy) CircularProgressIndicator()
    status?.let { Text(it, style = MaterialTheme.typography.bodyMedium) }
  }
}
