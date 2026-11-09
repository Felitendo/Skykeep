package io.github.yashwanthkamireddi.skykeep

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.CircularProgressIndicator
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
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.start

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
              runCatching { start(KeystoreSecrets(applicationContext)) }
              val a = Account(filesDir.resolve("account").path)
              a to runCatching { a.resume() }.getOrDefault(false)
            }
            account = a
            phase = if (resumed) Phase.SignedIn else Phase.SignedOut
          }
          when (phase) {
            Phase.Starting -> CircularProgressIndicator(Modifier.padding(96.dp))
            Phase.SignedOut -> SignInScreen(account!!) { phase = Phase.SignedIn }
            Phase.SignedIn -> Column(Modifier.padding(32.dp).padding(top = 64.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
              Text("Signed in", style = MaterialTheme.typography.headlineSmall)
            }
          }
        }
      }
    }
  }
}
