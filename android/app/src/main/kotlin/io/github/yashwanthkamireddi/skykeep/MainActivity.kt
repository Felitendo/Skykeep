package io.github.yashwanthkamireddi.skykeep

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import io.github.yashwanthkamireddi.skykeep.ui.theme.SkykeepTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.appleReachable
import uniffi.quince_mobile.engineVersion
import uniffi.quince_mobile.start

class MainActivity : ComponentActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    enableEdgeToEdge()
    setContent {
      SkykeepTheme {
        Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
          var status by remember { mutableStateOf("Checking…") }
          LaunchedEffect(Unit) {
            status = withContext(Dispatchers.IO) {
              val secrets = KeystoreSecrets(applicationContext)
              runCatching { start(secrets) }
              // Phase 0: a value sealed by the Keystore and read back.
              secrets.set("proof", "check", "sealed")
              val kept = secrets.get("proof", "check") == "sealed"
              secrets.delete("proof", "check")
              val gone = secrets.get("proof", "check") == null
              val synced = runCatching {
                val account = io.github.yashwanthkamireddi.skykeep.sync.ensureAccount(applicationContext, "proof@example.com")
                io.github.yashwanthkamireddi.skykeep.sync.Proof.run(applicationContext, account)
              }.getOrElse { "Contacts/Calendar: ${it.message}" }
              synced + "\n" + (if (kept && gone) "Keystore: sealed, read, deleted\n" else "Keystore: FAILED\n") +
              runCatching { if (appleReachable()) "iCloud reachable" else "iCloud not reachable" }
                .getOrElse { "Engine error: ${it.message}" }
            }
          }
          Column(Modifier.padding(32.dp).padding(top = 64.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Text("Skykeep", style = MaterialTheme.typography.headlineLarge)
            Text("Engine ${engineVersion()}", style = MaterialTheme.typography.bodyLarge)
            Text(status, style = MaterialTheme.typography.bodyLarge)
          }
        }
      }
    }
  }
}
