package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.content.ContentProviderOperation
import android.content.ContentResolver
import android.content.ContentValues
import android.net.Uri
import android.provider.ContactsContract
import android.provider.ContactsContract.Groups
import uniffi.quince_mobile.CList
import uniffi.quince_mobile.ListChange
import uniffi.quince_mobile.ListOutcome

/**
 * iCloud's contacts lists as the Contacts app's labels, both ways. A list is a group row (its
 * id in SOURCE_ID); who is in it is a membership row on each contact, written with the contact
 * (ContactsSync.rows) and read back with it (ContactsSync.readPerson).
 *
 * From the phone: a label made, renamed or deleted is that list made, renamed or deleted; a
 * contact added to or taken out of a label is a member added or removed, laid over the list as
 * iCloud has it when sent, so members added on another device meanwhile stay.
 */
object GroupsSync {
  private fun Uri.asAdapter(a: Account) = buildUpon()
    .appendQueryParameter(ContactsContract.CALLER_IS_SYNCADAPTER, "true")
    .appendQueryParameter(Groups.ACCOUNT_NAME, a.name)
    .appendQueryParameter(Groups.ACCOUNT_TYPE, a.type)
    .build()

  private data class Row(val id: Long, val guid: String?, val title: String?, val dirty: Boolean, val deleted: Boolean)

  private fun rows(cr: ContentResolver, account: Account): List<Row> =
    cr.query(
      Groups.CONTENT_URI.asAdapter(account),
      arrayOf(Groups._ID, Groups.SOURCE_ID, Groups.TITLE, Groups.DIRTY, Groups.DELETED),
      "${Groups.ACCOUNT_TYPE}=? AND ${Groups.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { c ->
      buildList { while (c.moveToNext()) add(Row(c.getLong(0), c.getString(1), c.getString(2), c.getInt(3) != 0, c.getInt(4) != 0)) }
    } ?: emptyList()

  /** Each label's row id to its list's id, for reading a contact's memberships. */
  fun guids(cr: ContentResolver, account: Account): Map<Long, String> =
    rows(cr, account).mapNotNull { r -> r.guid?.let { r.id to it } }.toMap()

  /** iCloud's lists written as labels, before the contacts are (their memberships name them). */
  fun download(cr: ContentResolver, account: Account, lists: List<CList>) {
    val have = rows(cr, account).filter { it.guid != null }.associateBy { it.guid!! }
    val ops = ArrayList<ContentProviderOperation>()
    for (l in lists) {
      val row = have[l.guid]
      if (row == null) {
        ops += ContentProviderOperation.newInsert(Groups.CONTENT_URI.asAdapter(account))
          .withValue(Groups.ACCOUNT_NAME, account.name).withValue(Groups.ACCOUNT_TYPE, account.type)
          .withValue(Groups.SOURCE_ID, l.guid).withValue(Groups.TITLE, l.name)
          .withValue(Groups.GROUP_VISIBLE, 1).withValue(Groups.SHOULD_SYNC, 1)
          .build()
      } else if (!row.dirty && !row.deleted && row.title != l.name) {
        ops += ContentProviderOperation.newUpdate(Groups.CONTENT_URI.asAdapter(account))
          .withSelection("${Groups._ID}=?", arrayOf(row.id.toString()))
          .withValue(Groups.TITLE, l.name).build()
      }
    }
    // Lists gone from iCloud: their labels go too, unless changed here and waiting to be sent.
    val listed = lists.map { it.guid }.toSet()
    for ((guid, row) in have) {
      if (guid !in listed && !row.dirty) {
        ops += ContentProviderOperation.newDelete(Groups.CONTENT_URI.asAdapter(account))
          .withSelection("${Groups._ID}=?", arrayOf(row.id.toString())).build()
      }
    }
    if (ops.isNotEmpty()) cr.applyBatch(ContactsContract.AUTHORITY, ops)
  }

  /**
   * The phone's changes to labels sent through `save` (the engine): labels made, renamed and
   * deleted here, then the members added and removed (`members`: list id to added and removed
   * contacts, from ContactsSync's send). A label made here is sent before any member added to
   * it, under the id iCloud gives it.
   */
  fun send(
    cr: ContentResolver, account: Account, members: Map<Long, Pair<List<String>, List<String>>>,
    save: (List<ListChange>) -> List<ListOutcome>,
  ): Int {
    val rows = rows(cr, account)
    val made = rows.filter { it.guid == null && !it.deleted && !it.title.isNullOrBlank() }
    val gone = rows.filter { it.deleted }
    val renamed = rows.filter { it.guid != null && it.dirty && !it.deleted }
    for (r in gone.filter { it.guid == null }) remove(cr, account, r.id)

    val changes = ArrayList<Pair<Row, ListChange>>()
    for (r in made) {
      val (add, _) = members[r.id] ?: (emptyList<String>() to emptyList())
      changes += r to ListChange.Create(r.title!!, add)
    }
    for (r in renamed) changes += r to ListChange.Rename(r.guid!!, r.title ?: "")
    for (r in gone.filter { it.guid != null }) changes += r to ListChange.Delete(r.guid!!)
    for ((rowId, edit) in members) {
      val r = rows.firstOrNull { it.id == rowId } ?: continue
      val guid = r.guid ?: continue // made here: its members went with its creation
      if (r.deleted || (edit.first.isEmpty() && edit.second.isEmpty())) continue
      changes += r to ListChange.Members(guid, edit.first, edit.second)
    }
    if (changes.isEmpty()) return 0
    val outcomes = save(changes.map { it.second })
    var sent = 0
    for ((i, pair) in changes.withIndex()) {
      val (r, change) = pair
      when (val o = outcomes.getOrNull(i)) {
        is ListOutcome.Saved -> {
          sent++
          if (change is ListChange.Delete) remove(cr, account, r.id) else settle(cr, account, r.id, o.guid)
        }
        // Deleted in iCloud meanwhile: the label goes, as the next read would take it away.
        is ListOutcome.Gone -> remove(cr, account, r.id)
        // Refused for good: left as it is here, no longer waiting. Apple's comes back on read.
        is ListOutcome.Failed -> if (!o.later) settle(cr, account, r.id, null)
        null -> {}
      }
    }
    return sent
  }

  private fun settle(cr: ContentResolver, account: Account, id: Long, guid: String?) {
    cr.update(
      Groups.CONTENT_URI.asAdapter(account),
      ContentValues().apply { put(Groups.DIRTY, 0); guid?.let { put(Groups.SOURCE_ID, it) } },
      "${Groups._ID}=?", arrayOf(id.toString()),
    )
  }

  private fun remove(cr: ContentResolver, account: Account, id: Long) {
    cr.delete(Groups.CONTENT_URI.asAdapter(account), "${Groups._ID}=?", arrayOf(id.toString()))
  }
}
