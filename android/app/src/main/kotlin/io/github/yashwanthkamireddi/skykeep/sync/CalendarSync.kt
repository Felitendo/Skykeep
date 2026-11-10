package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.content.ContentProviderOperation
import android.content.ContentResolver
import android.content.ContentValues
import android.graphics.Color
import android.net.Uri
import android.provider.CalendarContract
import android.provider.CalendarContract.Calendars
import android.provider.CalendarContract.Events
import java.time.LocalDate
import java.time.ZoneId
import java.util.TimeZone
import uniffi.quince_mobile.Cal
import uniffi.quince_mobile.CalendarWindow
import uniffi.quince_mobile.Ev

/**
 * iCloud's calendars and their events into Android's Calendar, one way, for the span the
 * engine read: each calendar a calendar under the Skykeep account (found again by Apple's id,
 * `_SYNC_ID`), each event or occurrence an event found again the same way, rewritten only
 * when its etag (`SYNC_DATA1`) changed, and removed when it is gone from iCloud. Every event
 * iCloud sends is matched by its id against all of the account's events, wherever they start —
 * an event that began before the span but runs into it comes back too, and matched only within
 * the span it was put in again on every sync. Only removal keeps to the span: outside it
 * nothing was read, so nothing is known.
 */
object CalendarSync {
  data class Result(val calendars: Int, val added: Int, val updated: Int, val removed: Int, val unchanged: Int)

  /** The span read: a month back and a year ahead, both days included, in the phone's zone. */
  fun span(today: LocalDate = LocalDate.now()): Pair<String, String> =
    today.minusMonths(1).toString() to today.plusMonths(12).toString()

  private fun Uri.asAdapter(a: Account) = buildUpon()
    .appendQueryParameter(CalendarContract.CALLER_IS_SYNCADAPTER, "true")
    .appendQueryParameter(Calendars.ACCOUNT_NAME, a.name)
    .appendQueryParameter(Calendars.ACCOUNT_TYPE, a.type)
    .build()

  fun sync(cr: ContentResolver, account: Account, window: CalendarWindow, from: String, to: String): Result =
    synchronized(this) { syncLocked(cr, account, window, from, to) }

  private fun syncLocked(cr: ContentResolver, account: Account, window: CalendarWindow, from: String, to: String): Result {
    val ids = calendars(cr, account, window.calendars)
    val zone = ZoneId.systemDefault()
    // The span's bounds as moments: its first day's start to the day after its last. An
    // all-day event is kept at UTC midnight, so it is held to the span by its date in UTC.
    val first = LocalDate.parse(from)
    val after = LocalDate.parse(to).plusDays(1)
    val start = first.atStartOfDay(zone).toInstant().toEpochMilli()
    val end = after.atStartOfDay(zone).toInstant().toEpochMilli()
    val startUtc = first.atStartOfDay(java.time.ZoneOffset.UTC).toInstant().toEpochMilli()
    val endUtc = after.atStartOfDay(java.time.ZoneOffset.UTC).toInstant().toEpochMilli()

    // Every event of this account here, by Apple's id; a second row with the same id is extra.
    data class Local(val id: Long, val etag: String?, val starts: Long, val allDay: Boolean)
    val local = HashMap<String, Local>()
    val extra = ArrayList<Long>()
    cr.query(
      Events.CONTENT_URI.asAdapter(account),
      arrayOf(Events._ID, Events._SYNC_ID, Events.SYNC_DATA1, Events.DTSTART, Events.ALL_DAY),
      "${Events.ACCOUNT_TYPE}=? AND ${Events.ACCOUNT_NAME}=?",
      arrayOf(account.type, account.name), null,
    )?.use { c ->
      while (c.moveToNext()) {
        val sync = c.getString(1) ?: continue
        if (sync in local) { extra += c.getLong(0); continue }
        local[sync] = Local(c.getLong(0), c.getString(2), c.getLong(3), c.getInt(4) != 0)
      }
    }
    fun inSpan(l: Local) = if (l.allDay) l.starts in startUtc until endUtc else l.starts in start until end

    var added = 0; var updated = 0; var unchanged = 0
    val ops = ArrayList<ContentProviderOperation>()
    fun flush() { if (ops.isNotEmpty()) { cr.applyBatch(CalendarContract.AUTHORITY, ops); ops.clear() } }
    for (e in window.events) {
      val calendarId = ids[e.calendarGuid] ?: continue
      val have = local.remove(e.guid)
      if (have != null && have.etag != null && have.etag == e.etag) { unchanged++; continue }
      if (ops.size > 300) flush()
      val values = values(e, calendarId)
      ops += if (have == null) {
        added++
        ContentProviderOperation.newInsert(Events.CONTENT_URI.asAdapter(account)).withValues(values).build()
      } else {
        updated++
        ContentProviderOperation.newUpdate(Events.CONTENT_URI.asAdapter(account))
          .withSelection("${Events._ID}=?", arrayOf(have.id.toString())).withValues(values).build()
      }
    }
    // Gone from iCloud: those in the span that iCloud did not send. Outside it, left alone.
    val gone = local.values.filter(::inSpan).map { it.id }
    for (id in gone + extra) {
      if (ops.size > 300) flush()
      ops += ContentProviderOperation.newDelete(Events.CONTENT_URI.asAdapter(account))
        .withSelection("${Events._ID}=?", arrayOf(id.toString())).build()
    }
    flush()
    return Result(ids.size, added, updated, gone.size + extra.size, unchanged)
  }

  /** The account's calendars made to match iCloud's; Apple's id → Android's row id. */
  private fun calendars(cr: ContentResolver, account: Account, wanted: List<Cal>): Map<String, Long> {
    val have = HashMap<String, Long>()
    cr.query(
      Calendars.CONTENT_URI.asAdapter(account), arrayOf(Calendars._ID, Calendars._SYNC_ID),
      "${Calendars.ACCOUNT_TYPE}=? AND ${Calendars.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { c -> while (c.moveToNext()) c.getString(1)?.let { have[it] = c.getLong(0) } }
    val out = HashMap<String, Long>()
    for (k in wanted) {
      val values = ContentValues().apply {
        put(Calendars.CALENDAR_DISPLAY_NAME, k.title)
        put(Calendars.CALENDAR_COLOR, k.colour?.let { runCatching { Color.parseColor(it) }.getOrNull() } ?: Color.rgb(0x1E, 0x88, 0xE5))
        // Read here for now: changes are made in iCloud until two-way sync.
        put(Calendars.CALENDAR_ACCESS_LEVEL, Calendars.CAL_ACCESS_READ)
        put(Calendars.VISIBLE, 1)
        put(Calendars.SYNC_EVENTS, 1)
      }
      val id = have.remove(k.guid)
      if (id != null) {
        cr.update(Calendars.CONTENT_URI.asAdapter(account), values, "${Calendars._ID}=?", arrayOf(id.toString()))
        out[k.guid] = id
      } else {
        values.put(Calendars.ACCOUNT_NAME, account.name)
        values.put(Calendars.ACCOUNT_TYPE, account.type)
        values.put(Calendars.NAME, k.guid)
        values.put(Calendars._SYNC_ID, k.guid)
        values.put(Calendars.OWNER_ACCOUNT, account.name)
        values.put(Calendars.CALENDAR_TIME_ZONE, TimeZone.getDefault().id)
        cr.insert(Calendars.CONTENT_URI.asAdapter(account), values)?.lastPathSegment?.toLongOrNull()?.let { out[k.guid] = it }
      }
    }
    // A calendar gone from iCloud goes, its events with it.
    for (id in have.values) {
      cr.delete(Calendars.CONTENT_URI.asAdapter(account), "${Calendars._ID}=?", arrayOf(id.toString()))
    }
    return out
  }

  private fun values(e: Ev, calendarId: Long) = ContentValues().apply {
    put(Events.CALENDAR_ID, calendarId)
    put(Events._SYNC_ID, e.guid)
    put(Events.SYNC_DATA1, e.etag)
    put(Events.TITLE, e.title)
    put(Events.EVENT_LOCATION, e.location)
    put(Events.DESCRIPTION, e.notes)
    if (e.allDay) {
      // The engine gives an all-day event as midnight on this phone's clock; Android keeps
      // them as the same dates at midnight UTC, or they show a day early west of Greenwich.
      fun utcMidnight(seconds: Long): Long {
        val day = java.time.Instant.ofEpochSecond(seconds).atZone(ZoneId.systemDefault()).toLocalDate()
        return day.atStartOfDay(java.time.ZoneOffset.UTC).toInstant().toEpochMilli()
      }
      val startDay = utcMidnight(e.startsAt)
      put(Events.DTSTART, startDay)
      put(Events.DTEND, maxOf(utcMidnight(e.endsAt), startDay + 86_400_000))
      put(Events.ALL_DAY, 1)
      put(Events.EVENT_TIMEZONE, "UTC")
    } else {
      put(Events.DTSTART, e.startsAt * 1000)
      put(Events.DTEND, e.endsAt * 1000)
      put(Events.ALL_DAY, 0)
      put(Events.EVENT_TIMEZONE, e.timezone ?: TimeZone.getDefault().id)
    }
  }
}
