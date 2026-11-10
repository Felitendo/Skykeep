package io.github.yashwanthkamireddi.skykeep.ui

import android.accounts.Account
import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.CalendarContract
import android.provider.CalendarContract.Calendars
import android.provider.ContactsContract
import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.sync.Outcome
import io.github.yashwanthkamireddi.skykeep.sync.SignOut
import io.github.yashwanthkamireddi.skykeep.sync.setSyncEvery
import io.github.yashwanthkamireddi.skykeep.sync.syncEvery
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The bottom bar: five places, as Material's navigation bar holds (three to five). Contacts and
 * Calendar live in the phone's own apps; their settings open from their cards on Home.
 */
private enum class Tab(val label: String, val icon: Int) {
  Home("Home", R.drawable.ic_home),
  Reminders("Reminders", R.drawable.ic_reminders),
  Notes("Notes", R.drawable.ic_notes),
  Photos("Photos", R.drawable.ic_photos),
  Settings("Settings", R.drawable.ic_settings),
}

/** A page opened from Home rather than the bar: Contacts' or Calendar's settings. */
private enum class Page { Contacts, Calendar }

/**
 * Skykeep once set up. The using happens in the phone's own Contacts and Calendar apps; here are
 * the account, how syncing is going, and what syncs — management screens, as the design has it,
 * and saying so plainly.
 */
@Composable
fun MainScreens(engine: uniffi.quince_mobile.Account, onSignedOut: () -> Unit) {
  val context = LocalContext.current
  var tab by remember { mutableStateOf(Tab.Home) }
  var page by remember { mutableStateOf<Page?>(null) }
  androidx.activity.compose.BackHandler(enabled = page != null) { page = null }
  var asked by remember { mutableLongStateOf(0L) }
  val account = remember { phoneAccount(context) }
  val sync = rememberSyncView(account, asked)
  fun now() { account?.let { asked = System.currentTimeMillis(); syncNow(it) } }
  Scaffold(
    bottomBar = {
      NavigationBar {
        for (t in Tab.entries) {
          NavigationBarItem(
            selected = tab == t,
            onClick = { tab = t; page = null },
            icon = { Icon(painterResource(t.icon), contentDescription = null) },
            label = { Text(t.label) },
          )
        }
      }
    },
  ) { inner ->
    // Reminders, Notes and Photos scroll their own lists (they can be long); the rest are one column.
    if (tab == Tab.Reminders || tab == Tab.Notes || tab == Tab.Photos) {
      Box(Modifier.padding(inner).fillMaxSize()) {
        when (tab) {
          Tab.Notes -> NotesPage(engine)
          Tab.Photos -> PhotosPage(engine)
          else -> RemindersPage(engine)
        }
      }
    } else {
      Box(Modifier.padding(inner).fillMaxSize().verticalScroll(rememberScrollState())) {
        Column(Modifier.padding(horizontal = 20.dp, vertical = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
          when {
            page == Page.Contacts -> ContactsPage(account)
            page == Page.Calendar -> CalendarPage(account)
            tab == Tab.Home -> Home(engine, account, sync, onSync = ::now, onOpen = { page = it })
            tab == Tab.Settings -> SettingsPage(engine, account, onSignedOut)
          }
        }
      }
    }
  }
}

@Composable
private fun Home(engine: uniffi.quince_mobile.Account, account: Account?, sync: SyncView, onSync: () -> Unit, onOpen: (Page) -> Unit) {
  val context = LocalContext.current
  val id = remember { runCatching { engine.appleId() }.getOrNull() }
  Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
    Box(
      Modifier.size(48.dp).background(MaterialTheme.colorScheme.primaryContainer, CircleShape),
      contentAlignment = Alignment.Center,
    ) {
      Text(
        (id?.firstOrNull() ?: 'i').uppercase(),
        style = MaterialTheme.typography.titleLarge,
        color = MaterialTheme.colorScheme.onPrimaryContainer,
      )
    }
    Column {
      Text("Signed in", style = MaterialTheme.typography.titleMedium)
      id?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) }
    }
  }
  if (!allowed(context)) {
    val ask = androidx.activity.compose.rememberLauncherForActivityResult(
      androidx.activity.result.contract.ActivityResultContracts.RequestMultiplePermissions(),
    ) { granted -> if (granted.values.all { it }) onSync() }
    Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.tertiaryContainer)) {
      Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text("Allow Contacts and Calendar access so Skykeep can add your iCloud contacts and calendars to this phone.", style = MaterialTheme.typography.bodyMedium)
        Button(onClick = { ask.launch(NEEDED) }) { Text("Allow") }
      }
    }
  }
  for ((authority, name) in listOf(
    ContactsContract.AUTHORITY to "Contacts", CalendarContract.AUTHORITY to "Calendar",
    io.github.yashwanthkamireddi.skykeep.sync.REMINDERS_AUTHORITY to "Reminders",
    io.github.yashwanthkamireddi.skykeep.sync.NOTES_AUTHORITY to "Notes",
    io.github.yashwanthkamireddi.skykeep.sync.PHOTOS_AUTHORITY to "Photos",
  )) {
    // Contacts' and Calendar's cards open their settings (they have no tab of their own).
    val opens = when (name) { "Contacts" -> Page.Contacts; "Calendar" -> Page.Calendar; else -> null }
    Health(name, sync.last[authority], sync.busy, onClick = opens?.let { { onOpen(it) } })
  }
  // Phone makers' battery savers pause apps that aren't opened often, and a sync with them. Said
  // only when it has happened: nothing synced cleanly for two days, though Skykeep has synced
  // before.
  val lastGood = sync.last.values.mapNotNull { it?.okAt?.takeIf { t -> t > 0 } }.maxOrNull()
  if (lastGood != null && System.currentTimeMillis() - lastGood > 48 * 3_600_000L) {
    val brand = android.os.Build.MANUFACTURER.replaceFirstChar { it.uppercase() }
    Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.tertiaryContainer)) {
      Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(
          "Skykeep hasn't synced for two days. $brand phones can pause apps that aren't opened often. Allow Skykeep to run in the background so your contacts and calendars stay up to date.",
          style = MaterialTheme.typography.bodyMedium,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
          Button(onClick = {
            runCatching {
              context.startActivity(Intent(android.provider.Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
          }) { Text("Battery Settings") }
          TextButton(onClick = {
            context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://dontkillmyapp.com/${android.os.Build.MANUFACTURER.lowercase()}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
          }) { Text("How on $brand") }
        }
      }
    }
  }
  if (sync.held > 0 && !sync.busy) {
    Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.tertiaryContainer)) {
      Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(
          "${sync.held} changes are waiting for you. More than usual changed at once, so Skykeep held them to be safe.",
          style = MaterialTheme.typography.bodyMedium,
        )
        Button(onClick = {
          AUTHORITIES.forEach { Outcome.confirmMassRemoval(context, it) }
          onSync()
        }) { Text("Apply Them") }
      }
    }
  }
  Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
    Button(onClick = onSync, enabled = account != null && !sync.busy) { Text("Sync Now") }
    if (sync.busy) CircularProgressIndicator(Modifier.size(24.dp).align(Alignment.CenterVertically), strokeWidth = 2.dp)
  }
  Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
    FilledTonalButton(onClick = { openApp(context, contacts = true) }) { Text("Open Contacts") }
    FilledTonalButton(onClick = { openApp(context, contacts = false) }) { Text("Open Calendar") }
  }
}

/** One service's sync health, in the design's words. */
@Composable
private fun Health(name: String, last: Outcome.Last?, busy: Boolean, onClick: (() -> Unit)? = null) {
  val (line, detail, trouble) = when {
    last == null && busy -> Triple("Setting up", "The first sync can take a little while. You can keep using your phone.", false)
    last == null -> Triple("Not synced yet", null, false)
    last.ok -> Triple("Up to date", "Synced ${ago(last.at)}", false)
    last.said.contains("session", ignoreCase = true) || last.said.contains("sign in", ignoreCase = true) ->
      Triple("Sign in again", "Your Apple sign-in session has expired. Sign in again to resume sync. Nothing is lost, and changes waiting to send are kept.", true)
    else -> Triple(
      "Will try again",
      "Apple did not answer. Skykeep will try again on its own." + (if (last.okAt > 0) " Last successful sync: ${ago(last.okAt)}." else ""),
      true,
    )
  }
  Card(Modifier.fillMaxWidth().then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)) {
    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
      Row(verticalAlignment = Alignment.CenterVertically) {
        Text(name, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
        Text(
          line, style = MaterialTheme.typography.labelLarge,
          color = if (trouble) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.primary,
        )
      }
      detail?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant) }
      if (last?.ok == true) Text(last.said, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
      if (last != null && last.onlyHere > 0) {
        val what = if (name == "Contacts") "contact" else "event"
        Text(
          (if (last.onlyHere == 1) "1 $what is only on this phone." else "${last.onlyHere} ${what}s are only on this phone.") +
            (last.why?.let { " $it" } ?: ""),
          style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error,
        )
      }
    }
  }
}

@Composable
private fun ContactsPage(account: Account?) {
  val context = LocalContext.current
  Text("Contacts", style = MaterialTheme.typography.headlineSmall)
  Text(
    "Your iCloud contacts are in this phone's Contacts app, under Skykeep. Change them there, and the change reaches iCloud and your other devices.",
    style = MaterialTheme.typography.bodyMedium,
  )
  val count = remember {
    account?.let {
      context.contentResolver.query(
        ContactsContract.RawContacts.CONTENT_URI, arrayOf(ContactsContract.RawContacts._ID),
        "${ContactsContract.RawContacts.ACCOUNT_TYPE}=? AND ${ContactsContract.RawContacts.ACCOUNT_NAME}=? AND ${ContactsContract.RawContacts.DELETED}=0",
        arrayOf(it.type, it.name), null,
      )?.use { c -> c.count }
    } ?: 0
  }
  SyncSwitch(account, ContactsContract.AUTHORITY, "Sync contacts", "$count contacts on this phone")
  Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant)) {
    Text(
      "New contacts you create may go to a different account. In your contacts app, choose Skykeep as where new contacts are saved, so they reach iCloud.",
      style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(16.dp),
    )
  }
  FilledTonalButton(onClick = { openApp(context, contacts = true) }) { Text("Open Contacts") }
}

private data class PhoneCalendar(val id: Long, val name: String, val colour: Int, val visible: Boolean, val readOnly: Boolean)

@Composable
private fun CalendarPage(account: Account?) {
  val context = LocalContext.current
  Text("Calendar", style = MaterialTheme.typography.headlineSmall)
  Text(
    "Your iCloud calendars are in this phone's Calendar app. Events you add or change there reach iCloud and your other devices.",
    style = MaterialTheme.typography.bodyMedium,
  )
  SyncSwitch(account, CalendarContract.AUTHORITY, "Sync calendars", "A month back and a year ahead")
  val calendars = remember { mutableStateListOf<PhoneCalendar>() }
  LaunchedEffect(account) {
    calendars.clear()
    calendars += withContext(Dispatchers.IO) { account?.let { phoneCalendars(context, it) } ?: emptyList() }
  }
  if (calendars.isNotEmpty()) {
    Text("Shown on this phone", style = MaterialTheme.typography.titleSmall)
    for ((i, k) in calendars.withIndex()) {
      ListItem(
        headlineContent = { Text(k.name) },
        supportingContent = if (k.readOnly) ({ Text("Read only") }) else null,
        leadingContent = { Box(Modifier.size(14.dp).background(Color(k.colour), CircleShape)) },
        trailingContent = {
          Switch(checked = k.visible, onCheckedChange = { on ->
            calendars[i] = k.copy(visible = on)
            showCalendar(context, account!!, k.id, on)
          })
        },
      )
    }
    Text(
      "Which calendars show is set here for this phone. Colours come from iCloud.",
      style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
  }
  FilledTonalButton(onClick = { openApp(context, contacts = false) }) { Text("Open Calendar") }
}

/** Whether Android syncs this authority on its own (and on a change made on the phone). */
@Composable
private fun SyncSwitch(account: Account?, authority: String, title: String, detail: String) {
  var on by remember { mutableStateOf(account?.let { ContentResolver.getSyncAutomatically(it, authority) } ?: false) }
  ListItem(
    headlineContent = { Text(title) },
    supportingContent = { Text(detail) },
    trailingContent = {
      Switch(checked = on, enabled = account != null, onCheckedChange = { v ->
        on = v
        account?.let { ContentResolver.setSyncAutomatically(it, authority, v) }
      })
    },
  )
}

@Composable
private fun SettingsPage(engine: uniffi.quince_mobile.Account, account: Account?, onSignedOut: () -> Unit) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  Text("Settings", style = MaterialTheme.typography.headlineSmall)
  // Off unless turned on; from that moment on, never the whole camera roll at once.
  var backup by remember { mutableStateOf(io.github.yashwanthkamireddi.skykeep.sync.PhotoBackup.on(context)) }
  val media = if (android.os.Build.VERSION.SDK_INT >= 33) arrayOf(android.Manifest.permission.READ_MEDIA_IMAGES, android.Manifest.permission.READ_MEDIA_VIDEO)
    else arrayOf(android.Manifest.permission.READ_EXTERNAL_STORAGE)
  val askMedia = androidx.activity.compose.rememberLauncherForActivityResult(androidx.activity.result.contract.ActivityResultContracts.RequestMultiplePermissions()) { granted ->
    if (granted.values.all { it }) { backup = true; io.github.yashwanthkamireddi.skykeep.sync.PhotoBackup.set(context, true) }
  }
  Row(verticalAlignment = Alignment.CenterVertically) {
    Column(Modifier.weight(1f)) {
      Text("Back up new photos", style = MaterialTheme.typography.titleSmall)
      Text("Photos and videos you take on this phone from now on go to iCloud Photos.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
    androidx.compose.material3.Switch(backup, { on ->
      if (on) askMedia.launch(media) else { backup = false; io.github.yashwanthkamireddi.skykeep.sync.PhotoBackup.set(context, false) }
    })
  }
  Text("Sync every", style = MaterialTheme.typography.titleSmall)
  var every by remember { mutableLongStateOf(syncEvery(context)) }
  for ((seconds, label) in listOf(3_600L to "Hour", 3 * 3_600L to "3 hours", 6 * 3_600L to "6 hours", 12 * 3_600L to "12 hours", 86_400L to "Day")) {
    Row(
      Modifier.fillMaxWidth().selectable(selected = every == seconds, role = Role.RadioButton, onClick = {
        every = seconds
        setSyncEvery(context, account, seconds)
      }).padding(vertical = 4.dp),
      verticalAlignment = Alignment.CenterVertically,
    ) {
      RadioButton(selected = every == seconds, onClick = null)
      Spacer(Modifier.width(12.dp))
      Text(label, style = MaterialTheme.typography.bodyLarge)
    }
  }
  Text(
    "A change made on this phone is sent at once, whatever this is set to.",
    style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
  )
  HorizontalDivider()
  Text("Privacy", style = MaterialTheme.typography.titleSmall)
  Text(
    "Your password is used to sign in with Apple and is never stored. Skykeep keeps a sign-in session in this phone's secure keystore and talks only to Apple.",
    style = MaterialTheme.typography.bodyMedium,
  )
  HorizontalDivider()
  Text("About", style = MaterialTheme.typography.titleSmall)
  Text("Skykeep is free and open source.", style = MaterialTheme.typography.bodyMedium)
  TextButton(onClick = {
    context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://github.com/YashwanthKamireddi/Skykeep")))
  }) { Text("Source Code on GitHub") }
  HorizontalDivider()
  var leaving by remember { mutableStateOf(false) }
  var unsent by remember { mutableIntStateOf(0) }
  var working by remember { mutableStateOf(false) }
  var trouble by remember { mutableStateOf<String?>(null) }
  OutlinedButton(
    enabled = !working,
    onClick = {
      scope.launch {
        unsent = withContext(Dispatchers.IO) { account?.let { SignOut.unsent(context.contentResolver, it) } ?: 0 }
        leaving = true
      }
    },
  ) { Text("Sign Out") }
  trouble?.let { Text(it, color = MaterialTheme.colorScheme.error) }
  if (leaving) {
    AlertDialog(
      onDismissRequest = { leaving = false },
      title = { Text("Sign out of iCloud on this phone?") },
      text = {
        Text(
          (if (unsent > 0) "$unsent changes made on this phone haven't reached iCloud yet and won't be sent.\n\n" else "") +
            "Keep a copy and your contacts and calendars stay on this phone as its own, no longer synced. iCloud itself isn't changed either way.",
        )
      },
      // Three choices, stacked as Material lays out more than two: the safe one first, the one
      // that removes in the error colour, Cancel last.
      confirmButton = {
        Column(horizontalAlignment = Alignment.End) {
          for ((label, keep) in listOf("Keep a Copy" to true, "Remove from Phone" to false)) {
            TextButton(
              onClick = {
                leaving = false
                working = true
                scope.launch {
                  val done = withContext(Dispatchers.IO) {
                    runCatching {
                      account?.let { a ->
                        if (keep) SignOut.keepCopy(context.contentResolver, a)
                        SignOut.removeAccount(context, a)
                      }
                      engine.signOut()
                    }
                  }
                  working = false
                  done.onSuccess { onSignedOut() }.onFailure { trouble = "Couldn't sign out: ${it.message}" }
                }
              },
              colors = if (keep) ButtonDefaults.textButtonColors() else ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
            ) { Text(label) }
          }
          TextButton(onClick = { leaving = false }) { Text("Cancel") }
        }
      },
    )
  }
}

private fun phoneCalendars(context: Context, account: Account): List<PhoneCalendar> {
  val out = ArrayList<PhoneCalendar>()
  context.contentResolver.query(
    Calendars.CONTENT_URI,
    arrayOf(Calendars._ID, Calendars.CALENDAR_DISPLAY_NAME, Calendars.CALENDAR_COLOR, Calendars.VISIBLE, Calendars.CALENDAR_ACCESS_LEVEL),
    "${Calendars.ACCOUNT_TYPE}=? AND ${Calendars.ACCOUNT_NAME}=?", arrayOf(account.type, account.name),
    "${Calendars.CALENDAR_DISPLAY_NAME} COLLATE NOCASE",
  )?.use { c ->
    while (c.moveToNext()) {
      out += PhoneCalendar(c.getLong(0), c.getString(1) ?: "", c.getInt(2), c.getInt(3) != 0, c.getInt(4) < Calendars.CAL_ACCESS_CONTRIBUTOR)
    }
  }
  return out
}

/** A calendar shown or hidden on this phone: a setting of the phone's, never sent to iCloud. */
private fun showCalendar(context: Context, account: Account, id: Long, visible: Boolean) {
  val uri = Calendars.CONTENT_URI.buildUpon()
    .appendQueryParameter(CalendarContract.CALLER_IS_SYNCADAPTER, "true")
    .appendQueryParameter(Calendars.ACCOUNT_NAME, account.name)
    .appendQueryParameter(Calendars.ACCOUNT_TYPE, account.type)
    .build()
  context.contentResolver.update(uri, ContentValues().apply { put(Calendars.VISIBLE, if (visible) 1 else 0) }, "${Calendars._ID}=?", arrayOf(id.toString()))
}

/** "5 minutes ago", in the phone's language. */
/** "just now" under a minute (Android's own words would be "0 minutes ago"), then "5 minutes ago". */
private fun ago(ms: Long): String =
  if (System.currentTimeMillis() - ms < DateUtils.MINUTE_IN_MILLIS) "just now"
  else DateUtils.getRelativeTimeSpanString(ms, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString().lowercase()
