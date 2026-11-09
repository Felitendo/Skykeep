package io.github.yashwanthkamireddi.skykeep

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec
import uniffi.quince_mobile.SecretStore

/**
 * The engine's secrets, as the keyring keeps them on desktop: each value sealed with AES-GCM
 * under a key that never leaves Android's Keystore, written to the app's no-backup folder so
 * it is never copied into a cloud or device backup. The entry's name is bound into the seal,
 * so a value moved to another entry fails to open rather than being read as that entry.
 */
class KeystoreSecrets(context: Context) : SecretStore {
  private val dir = File(context.noBackupFilesDir, "secrets").apply { mkdirs() }
  private val lock = Any()

  private fun key(): SecretKey {
    val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
    (ks.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
    val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
    gen.init(
      KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        .build(),
    )
    return gen.generateKey()
  }

  /** One file per entry, named by a hash so no service or account name appears on disk. */
  private fun file(service: String, name: String): File {
    val digest = java.security.MessageDigest.getInstance("SHA-256").digest("$service\u0000$name".toByteArray())
    return File(dir, Base64.encodeToString(digest, Base64.URL_SAFE or Base64.NO_PADDING or Base64.NO_WRAP))
  }

  override fun get(service: String, name: String): String? = synchronized(lock) {
    val f = file(service, name)
    if (!f.exists()) return null
    val bytes = f.readBytes()
    val cipher = Cipher.getInstance(TRANSFORM)
    cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, bytes, 0, IV))
    cipher.updateAAD("$service\u0000$name".toByteArray())
    String(cipher.doFinal(bytes, IV, bytes.size - IV), Charsets.UTF_8)
  }

  override fun set(service: String, name: String, value: String) = synchronized(lock) {
    val cipher = Cipher.getInstance(TRANSFORM)
    cipher.init(Cipher.ENCRYPT_MODE, key())
    cipher.updateAAD("$service\u0000$name".toByteArray())
    val sealed = cipher.iv + cipher.doFinal(value.toByteArray(Charsets.UTF_8))
    // Written whole then renamed over, so a crash mid-write never leaves half a secret.
    val f = file(service, name)
    val tmp = File(dir, f.name + ".part")
    tmp.writeBytes(sealed)
    if (!tmp.renameTo(f)) throw java.io.IOException("Could not save the secret.")
  }

  override fun delete(service: String, name: String) = synchronized(lock) {
    file(service, name).delete()
    Unit
  }

  private companion object {
    const val ALIAS = "skykeep-secrets"
    const val TRANSFORM = "AES/GCM/NoPadding"
    const val IV = 12
  }
}
