package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.content.ContentProviderOperation
import android.content.ContentResolver
import android.net.Uri
import android.provider.ContactsContract
import android.provider.ContactsContract.CommonDataKinds.Email
import android.provider.ContactsContract.CommonDataKinds.Event
import android.provider.ContactsContract.CommonDataKinds.Note
import android.provider.ContactsContract.CommonDataKinds.Organization
import android.provider.ContactsContract.CommonDataKinds.Phone
import android.provider.ContactsContract.CommonDataKinds.StructuredName
import android.provider.ContactsContract.CommonDataKinds.StructuredPostal
import android.provider.ContactsContract.CommonDataKinds.Website
import android.provider.ContactsContract.Data
import android.provider.ContactsContract.RawContacts
import uniffi.quince_mobile.Person

/**
 * iCloud's address book into Android's Contacts, one way: each card a raw contact under the
 * Skykeep account, found again by Apple's id (SOURCE_ID) and rewritten only when its etag
 * (SYNC1) changed; a card gone from iCloud is removed here. Writes go in batches, each card
 * whole in one batch, so a card is never left half-written.
 */
object ContactsSync {
  data class Result(val added: Int, val updated: Int, val removed: Int, val unchanged: Int)

  private fun Uri.asAdapter(a: Account) = buildUpon()
    .appendQueryParameter(ContactsContract.CALLER_IS_SYNCADAPTER, "true")
    .appendQueryParameter(RawContacts.ACCOUNT_NAME, a.name)
    .appendQueryParameter(RawContacts.ACCOUNT_TYPE, a.type)
    .build()

  private data class Local(val id: Long, val etag: String?)

  /**
   * One sync at a time in the process. Adding the account makes Android start its own sync at
   * once; run beside the app's, both read an empty book and every card was written twice.
   */
  fun sync(cr: ContentResolver, account: Account, people: List<Person>): Result = synchronized(this) {
    syncLocked(cr, account, people)
  }

  private fun syncLocked(cr: ContentResolver, account: Account, people: List<Person>): Result {
    val local = HashMap<String, Local>()
    // A card found twice (left by an earlier fault) keeps one copy; the others are removed.
    val extra = ArrayList<Long>()
    cr.query(
      RawContacts.CONTENT_URI.asAdapter(account),
      arrayOf(RawContacts._ID, RawContacts.SOURCE_ID, RawContacts.SYNC1),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { c ->
      while (c.moveToNext()) {
        val source = c.getString(1)
        if (source == null) continue
        if (source in local) { extra += c.getLong(0); continue }
        local[source] = Local(c.getLong(0), c.getString(2))
      }
    }
    var added = 0; var updated = 0; var unchanged = 0
    val ops = ArrayList<ContentProviderOperation>()
    fun flush() { if (ops.isNotEmpty()) { cr.applyBatch(ContactsContract.AUTHORITY, ops); ops.clear() } }

    for (p in people) {
      val have = local.remove(p.guid)
      if (have != null && have.etag != null && have.etag == p.etag) { unchanged++; continue }
      if (ops.size > 300) flush()
      val start = ops.size
      if (have == null) {
        ops += ContentProviderOperation.newInsert(RawContacts.CONTENT_URI.asAdapter(account))
          .withValue(RawContacts.ACCOUNT_NAME, account.name)
          .withValue(RawContacts.ACCOUNT_TYPE, account.type)
          .withValue(RawContacts.SOURCE_ID, p.guid)
          .withValue(RawContacts.SYNC1, p.etag)
          .build()
        rows(p).forEach { ops += it.withValueBackReference(Data.RAW_CONTACT_ID, start).build() }
        added++
      } else {
        ops += ContentProviderOperation.newUpdate(RawContacts.CONTENT_URI.asAdapter(account))
          .withSelection("${RawContacts._ID}=?", arrayOf(have.id.toString()))
          .withValue(RawContacts.SYNC1, p.etag)
          .build()
        ops += ContentProviderOperation.newDelete(Data.CONTENT_URI.asAdapter(account))
          .withSelection("${Data.RAW_CONTACT_ID}=?", arrayOf(have.id.toString()))
          .build()
        rows(p).forEach { ops += it.withValue(Data.RAW_CONTACT_ID, have.id).build() }
        updated++
      }
    }
    // Cards no longer in iCloud, and second copies.
    for (id in local.values.map { it.id } + extra) {
      if (ops.size > 300) flush()
      ops += ContentProviderOperation.newDelete(RawContacts.CONTENT_URI.asAdapter(account))
        .withSelection("${RawContacts._ID}=?", arrayOf(id.toString()))
        .build()
    }
    flush()
    return Result(added, updated, local.size + extra.size, unchanged)
  }

  /** A card's data rows, each still to be pointed at its raw contact. */
  private fun rows(p: Person): List<ContentProviderOperation.Builder> {
    val out = ArrayList<ContentProviderOperation.Builder>()
    fun row(mime: String) = ContentProviderOperation.newInsert(Data.CONTENT_URI).withValue(Data.MIMETYPE, mime)
    out += row(StructuredName.CONTENT_ITEM_TYPE)
      .withValue(StructuredName.GIVEN_NAME, p.firstName)
      .withValue(StructuredName.MIDDLE_NAME, p.middleName)
      .withValue(StructuredName.FAMILY_NAME, p.lastName)
      .apply { if (p.isCompany || (p.firstName == null && p.lastName == null)) withValue(StructuredName.DISPLAY_NAME, p.company) }
    p.company?.let { out += row(Organization.CONTENT_ITEM_TYPE).withValue(Organization.COMPANY, it) }
    p.notes?.let { out += row(Note.CONTENT_ITEM_TYPE).withValue(Note.NOTE, it) }
    p.birthday?.let {
      out += row(Event.CONTENT_ITEM_TYPE).withValue(Event.TYPE, Event.TYPE_BIRTHDAY).withValue(Event.START_DATE, it)
    }
    for (m in p.methods) {
      // Apple's own labels kept as written ("_$!<Mobile>!$_" style ones read plainly).
      val label = m.label?.removePrefix("_\$!<")?.removeSuffix(">!\$_")
      out += when (m.kind) {
        "phone" -> row(Phone.CONTENT_ITEM_TYPE).withValue(Phone.NUMBER, m.value)
          .withValue(Phone.TYPE, Phone.TYPE_CUSTOM).withValue(Phone.LABEL, label ?: "phone")
        "email" -> row(Email.CONTENT_ITEM_TYPE).withValue(Email.ADDRESS, m.value)
          .withValue(Email.TYPE, Email.TYPE_CUSTOM).withValue(Email.LABEL, label ?: "email")
        "url" -> row(Website.CONTENT_ITEM_TYPE).withValue(Website.URL, m.value)
          .withValue(Website.TYPE, Website.TYPE_CUSTOM).withValue(Website.LABEL, label ?: "url")
        "address" -> row(StructuredPostal.CONTENT_ITEM_TYPE).withValue(StructuredPostal.FORMATTED_ADDRESS, m.value)
          .withValue(StructuredPostal.TYPE, StructuredPostal.TYPE_CUSTOM).withValue(StructuredPostal.LABEL, label ?: "address")
        else -> continue
      }
    }
    return out
  }
}
