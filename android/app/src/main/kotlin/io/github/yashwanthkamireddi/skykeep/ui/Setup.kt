package io.github.yashwanthkamireddi.skykeep.ui

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.CalendarContract
import android.provider.ContactsContract
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.res.painterResource
import io.github.yashwanthkamireddi.skykeep.R
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import io.github.yashwanthkamireddi.skykeep.sync.ensureAccount
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The access the sync needs: the phone's Contacts and Calendar. */
val NEEDED = arrayOf(
  Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS,
  Manifest.permission.READ_CALENDAR, Manifest.permission.WRITE_CALENDAR,
)

fun allowed(context: Context) = NEEDED.all { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }

/** First run, before signing in: what Skykeep does, and that it can be trusted with it. */
@Composable
fun Welcome(onContinue: () -> Unit) {
  Column(Modifier.fillMaxSize().padding(28.dp).padding(top = 56.dp, bottom = 12.dp)) {
    Image(
      painterResource(R.mipmap.ic_launcher_foreground), contentDescription = null,
      modifier = Modifier.size(112.dp).offset(x = (-20).dp),
    )
    Spacer(Modifier.height(20.dp))
    Text("Your iCloud, working on this phone", style = MaterialTheme.typography.headlineMedium)
    Spacer(Modifier.height(16.dp))
    Text(
      "Skykeep puts your iCloud contacts and calendars into the contacts and calendar apps already on this phone, and keeps them in sync both ways.",
      style = MaterialTheme.typography.bodyLarge,
    )
    Spacer(Modifier.height(12.dp))
    Text(
      "Skykeep is free and open source. Your password is never stored.",
      style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.weight(1f))
    // Within the thumb's reach, as onboarding screens place their one action.
    Button(onClick = onContinue, modifier = Modifier.fillMaxWidth().height(52.dp)) { Text("Get Started") }
  }
}

/**
 * After signing in: access to the phone's Contacts and Calendar, asked with its reason and a
 * Not now; then the first sync, with what has arrived so far; then where to find it all.
 */
@Composable
fun Setup(onDone: () -> Unit) {
  val context = LocalContext.current
  var step by remember { mutableStateOf(if (allowed(context)) 1 else 0) }
  var asked by remember { mutableLongStateOf(0L) }
  val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
    if (granted.values.all { it }) step = 1 else onDone()
  }
  Column(Modifier.fillMaxSize().padding(28.dp).padding(top = 72.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
    when (step) {
      0 -> {
        Text("Contacts and Calendar", style = MaterialTheme.typography.headlineSmall)
        Text("So Skykeep can add your iCloud contacts and calendars to this phone.", style = MaterialTheme.typography.bodyLarge)
        Button(onClick = { ask.launch(NEEDED) }) { Text("Allow") }
        TextButton(onClick = onDone) { Text("Not Now") }
      }
      else -> {
        var account by remember { mutableStateOf<android.accounts.Account?>(null) }
        LaunchedEffect(Unit) {
          val made = withContext(Dispatchers.IO) {
            ensureAccount(context, io.github.yashwanthkamireddi.skykeep.Engine.account(context).appleId() ?: "iCloud")
          }
          account = made
          asked = System.currentTimeMillis()
          syncNow(made)
        }
        val sync = rememberSyncView(account, asked)
        val arrived = AUTHORITIES.all { (sync.last[it]?.at ?: 0) >= asked && asked > 0 }
        if (!arrived) {
          Text("Adding your contacts and calendars", style = MaterialTheme.typography.headlineSmall)
          CircularProgressIndicator(Modifier.size(32.dp))
          for (authority in AUTHORITIES) {
            sync.last[authority]?.takeIf { it.at >= asked }?.let { Text(it.said, style = MaterialTheme.typography.bodyMedium) }
          }
          Text("You can use your phone while this finishes.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
          TextButton(onClick = onDone) { Text("Continue in the Background") }
        } else {
          Text("All set", style = MaterialTheme.typography.headlineSmall)
          Text("Your contacts are in your contacts app and your calendars are in your calendar app.", style = MaterialTheme.typography.bodyLarge)
          FilledTonalButton(onClick = { openApp(context, contacts = true) }) { Text("Open Contacts") }
          FilledTonalButton(onClick = { openApp(context, contacts = false) }) { Text("Open Calendar") }
          Button(onClick = onDone) { Text("Explore Skykeep") }
        }
      }
    }
  }
}

/** The phone's own Contacts or Calendar app. */
fun openApp(context: Context, contacts: Boolean) {
  val intent = if (contacts) {
    Intent(Intent.ACTION_VIEW, ContactsContract.Contacts.CONTENT_URI)
  } else {
    Intent(Intent.ACTION_VIEW, CalendarContract.CONTENT_URI.buildUpon().appendPath("time").build())
  }
  runCatching { context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
}
