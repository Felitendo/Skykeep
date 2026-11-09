package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.content.ContentProviderOperation
import android.content.ContentValues
import android.content.Context
import android.graphics.Color
import android.net.Uri
import android.provider.CalendarContract
import android.provider.ContactsContract
import java.util.TimeZone

/**
 * Phase 0 only: writes one contact and one calendar with one event into Android's own stores, as
 * the sync adapter will, and reads them back. Removed once Phase 1 syncs real iCloud data.
 */
object Proof {
  private fun Uri.asAdapter(account: Account) = buildUpon()
    .appendQueryParameter(ContactsContract.CALLER_IS_SYNCADAPTER, "true")
    .appendQueryParameter(CalendarContract.Calendars.ACCOUNT_NAME, account.name)
    .appendQueryParameter(CalendarContract.Calendars.ACCOUNT_TYPE, account.type)
    .build()

  fun run(context: Context, account: Account): String {
    val cr = context.contentResolver
    // A contact, its name and phone, in one batch so it is written whole or not at all.
    val ops = arrayListOf(
      ContentProviderOperation.newInsert(ContactsContract.RawContacts.CONTENT_URI.asAdapter(account))
        .withValue(ContactsContract.RawContacts.ACCOUNT_NAME, account.name)
        .withValue(ContactsContract.RawContacts.ACCOUNT_TYPE, account.type)
        .withValue(ContactsContract.RawContacts.SOURCE_ID, "proof-contact")
        .build(),
      ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI.asAdapter(account))
        .withValueBackReference(ContactsContract.Data.RAW_CONTACT_ID, 0)
        .withValue(ContactsContract.Data.MIMETYPE, ContactsContract.CommonDataKinds.StructuredName.CONTENT_ITEM_TYPE)
        .withValue(ContactsContract.CommonDataKinds.StructuredName.DISPLAY_NAME, "Skykeep Proof")
        .build(),
      ContentProviderOperation.newInsert(ContactsContract.Data.CONTENT_URI.asAdapter(account))
        .withValueBackReference(ContactsContract.Data.RAW_CONTACT_ID, 0)
        .withValue(ContactsContract.Data.MIMETYPE, ContactsContract.CommonDataKinds.Phone.CONTENT_ITEM_TYPE)
        .withValue(ContactsContract.CommonDataKinds.Phone.NUMBER, "+1 555 0100")
        .withValue(ContactsContract.CommonDataKinds.Phone.TYPE, ContactsContract.CommonDataKinds.Phone.TYPE_MOBILE)
        .build(),
    )
    cr.applyBatch(ContactsContract.AUTHORITY, ops)

    // A calendar owned by the account, then an event in it, tomorrow at ten for an hour.
    val calendar = cr.insert(
      CalendarContract.Calendars.CONTENT_URI.asAdapter(account),
      ContentValues().apply {
        put(CalendarContract.Calendars.ACCOUNT_NAME, account.name)
        put(CalendarContract.Calendars.ACCOUNT_TYPE, account.type)
        put(CalendarContract.Calendars.NAME, "proof-calendar")
        put(CalendarContract.Calendars.CALENDAR_DISPLAY_NAME, "Skykeep Proof")
        put(CalendarContract.Calendars.CALENDAR_COLOR, Color.rgb(0x1E, 0x88, 0xE5))
        put(CalendarContract.Calendars.CALENDAR_ACCESS_LEVEL, CalendarContract.Calendars.CAL_ACCESS_OWNER)
        put(CalendarContract.Calendars.OWNER_ACCOUNT, account.name)
        put(CalendarContract.Calendars.VISIBLE, 1)
        put(CalendarContract.Calendars.SYNC_EVENTS, 1)
        put(CalendarContract.Calendars.CALENDAR_TIME_ZONE, TimeZone.getDefault().id)
      },
    )!!
    val start = java.util.Calendar.getInstance().apply {
      add(java.util.Calendar.DAY_OF_YEAR, 1); set(java.util.Calendar.HOUR_OF_DAY, 10)
      set(java.util.Calendar.MINUTE, 0); set(java.util.Calendar.SECOND, 0); set(java.util.Calendar.MILLISECOND, 0)
    }.timeInMillis
    cr.insert(
      CalendarContract.Events.CONTENT_URI.asAdapter(account),
      ContentValues().apply {
        put(CalendarContract.Events.CALENDAR_ID, calendar.lastPathSegment!!.toLong())
        put(CalendarContract.Events.TITLE, "Skykeep proof event")
        put(CalendarContract.Events.DTSTART, start)
        put(CalendarContract.Events.DTEND, start + 3_600_000)
        put(CalendarContract.Events.EVENT_TIMEZONE, TimeZone.getDefault().id)
        put(CalendarContract.Events._SYNC_ID, "proof-event")
      },
    )

    // Read back from Android's stores, as any app (the dialler, Google Calendar) would.
    val contacts = cr.query(
      ContactsContract.RawContacts.CONTENT_URI, arrayOf(ContactsContract.RawContacts._ID),
      "${ContactsContract.RawContacts.ACCOUNT_TYPE}=?", arrayOf(account.type), null,
    )!!.use { it.count }
    val events = cr.query(
      CalendarContract.Events.CONTENT_URI, arrayOf(CalendarContract.Events._ID),
      "${CalendarContract.Events.ACCOUNT_TYPE}=?", arrayOf(account.type), null,
    )!!.use { it.count }
    return "Contacts: $contacts in Android · Events: $events in Android"
  }
}
