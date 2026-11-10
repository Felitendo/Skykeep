package io.github.yashwanthkamireddi.skykeep.sync

import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * iCloud Drive through the Files app's own calls (DocumentsContract), against the signed-in
 * account's real iCloud, in a folder of its own: "Skykeep test" made at the top, a file written
 * into it and read back, written again (into that same file), renamed, then the folder deleted.
 * Live, so it runs only when asked: `am instrument -e live true …`. Never touches anything else.
 */
@RunWith(AndroidJUnit4::class)
class DriveProviderLiveTest {
  private val context = InstrumentationRegistry.getInstrumentation().targetContext
  private val cr = context.contentResolver
  private val authority = "${context.packageName}.drive"
  private val root = "FOLDER::com.apple.CloudDocs::root"

  private fun doc(id: String) = DocumentsContract.buildDocumentUri(authority, id)

  private fun children(folder: String): Map<String, String> =
    cr.query(DocumentsContract.buildChildDocumentsUri(authority, folder), arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME), null, null, null)!!
      .use { c -> buildMap { while (c.moveToNext()) put(c.getString(1), c.getString(0)) } }

  @Test
  fun a_file_goes_in_comes_back_and_is_cleaned_up() {
    assumeTrue("live only when asked", InstrumentationRegistry.getArguments().getString("live") == "true")
    assertTrue("no leftover test folder", "Skykeep test" !in children(root))
    val folder = DocumentsContract.createDocument(cr, doc(root), Document.MIME_TYPE_DIR, "Skykeep test")!!
    val folderId = DocumentsContract.getDocumentId(folder)
    try {
      val file = DocumentsContract.createDocument(cr, folder, "text/plain", "hello.txt")!!
      cr.openOutputStream(file, "wt")!!.use { it.write("first version".toByteArray()) }
      // Sent when the stream closes, in the provider's own queue: wait for the folder to show it.
      waitFor { children(folderId)["hello.txt"] != null }
      val id = DocumentsContract.getDocumentId(file)
      assertEquals("first version", cr.openInputStream(doc(id))!!.use { String(it.readBytes()) })

      // Written again: into that same file, its id kept.
      cr.openOutputStream(doc(id), "wt")!!.use { it.write("second version".toByteArray()) }
      Thread.sleep(8_000)
      val after = children(folderId)
      assertEquals("written into the same file, not a copy beside it: $after", id, after["hello.txt"])

      DocumentsContract.renameDocument(cr, doc(id), "renamed.txt")
      waitFor { children(folderId).containsKey("renamed.txt") }
    } finally {
      DocumentsContract.deleteDocument(cr, folder)
    }
    waitFor { "Skykeep test" !in children(root) }
  }

  private fun waitFor(seconds: Int = 60, ok: () -> Boolean) {
    repeat(seconds) { if (runCatching(ok).getOrDefault(false)) return; Thread.sleep(1_000) }
    throw AssertionError("not in time")
  }
}
