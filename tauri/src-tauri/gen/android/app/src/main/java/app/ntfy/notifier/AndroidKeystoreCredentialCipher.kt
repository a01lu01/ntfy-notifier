package app.ntfy.notifier

import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.security.KeyStore
import javax.crypto.AEADBadTagException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

data class CredentialEnvelope(
  val provider: String,
  val version: Long,
  val ciphertext: String
)

interface CredentialCipher {
  fun encrypt(plaintext: String): CredentialEnvelope
  fun decrypt(credential: CredentialEnvelope): String
}

/**
 * Stores only the AES key in Android Keystore. The IV, ciphertext and GCM tag are packed into the
 * credential envelope's single ciphertext field as Base64([ivLength][iv][ciphertext+tag]).
 */
class AndroidKeystoreCredentialCipher(
  private val keyAlias: String = DEFAULT_KEY_ALIAS
) : CredentialCipher {

  override fun encrypt(plaintext: String): CredentialEnvelope {
    if (plaintext.isEmpty()) {
      return CredentialEnvelope(PROVIDER, CREDENTIAL_VERSION, "")
    }

    val plaintextBytes = plaintext.toByteArray(Charsets.UTF_8)
    try {
      val cipher = Cipher.getInstance(TRANSFORMATION)
      cipher.init(Cipher.ENCRYPT_MODE, getOrCreateKey())
      cipher.updateAAD(AAD)
      val encrypted = cipher.doFinal(plaintextBytes)
      val iv = cipher.iv ?: throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_ENCRYPT,
        "Android Keystore 未返回加密向量"
      )
      if (iv.size !in MIN_IV_BYTES..MAX_IV_BYTES) {
        throw ConfigStoreException(
          ConfigStoreError.CREDENTIAL_ENCRYPT,
          "Android Keystore 返回的加密向量长度无效"
        )
      }

      val packed = ByteArray(1 + iv.size + encrypted.size)
      packed[0] = iv.size.toByte()
      System.arraycopy(iv, 0, packed, 1, iv.size)
      System.arraycopy(encrypted, 0, packed, 1 + iv.size, encrypted.size)
      return CredentialEnvelope(
        PROVIDER,
        CREDENTIAL_VERSION,
        Base64.encodeToString(packed, Base64.NO_WRAP)
      )
    } catch (error: ConfigStoreException) {
      throw error
    } catch (error: Exception) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_ENCRYPT,
        "Android Keystore 保护凭据失败",
        error
      )
    } finally {
      plaintextBytes.fill(0)
    }
  }

  override fun decrypt(credential: CredentialEnvelope): String {
    if (credential.provider != PROVIDER) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_PROVIDER,
        "不支持的 Android 凭据提供方"
      )
    }
    if (credential.version != CREDENTIAL_VERSION) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_VERSION,
        "不支持的 Android 凭据版本"
      )
    }
    if (credential.ciphertext.isEmpty()) {
      return ""
    }

    val packed = decodeBase64Strict(credential.ciphertext)
    if (packed.size < 1 + MIN_IV_BYTES + GCM_TAG_BYTES) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_FORMAT,
        "Android 凭据密文长度无效"
      )
    }
    val ivLength = packed[0].toInt() and 0xff
    if (ivLength !in MIN_IV_BYTES..MAX_IV_BYTES || packed.size < 1 + ivLength + GCM_TAG_BYTES) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_FORMAT,
        "Android 凭据加密向量长度无效"
      )
    }
    val iv = packed.copyOfRange(1, 1 + ivLength)
    val encrypted = packed.copyOfRange(1 + ivLength, packed.size)

    val key = getExistingKey()
    val plaintext = try {
      val cipher = Cipher.getInstance(TRANSFORMATION)
      cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(GCM_TAG_BITS, iv))
      cipher.updateAAD(AAD)
      cipher.doFinal(encrypted)
    } catch (error: AEADBadTagException) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_DECRYPT,
        "Android 凭据认证失败",
        error
      )
    } catch (error: ConfigStoreException) {
      throw error
    } catch (error: Exception) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_DECRYPT,
        "Android Keystore 解密凭据失败",
        error
      )
    }

    return try {
      decodeUtf8Strict(plaintext)
    } finally {
      plaintext.fill(0)
    }
  }

  private fun getOrCreateKey(): SecretKey {
    val keyStore = loadKeyStore()
    if (keyStore.containsAlias(keyAlias)) {
      return keyFromStore(keyStore)
    }

    return try {
      val generator = KeyGenerator.getInstance(
        KeyProperties.KEY_ALGORITHM_AES,
        ANDROID_KEYSTORE
      )
      generator.init(
        KeyGenParameterSpec.Builder(
          keyAlias,
          KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
        )
          .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
          .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
          .setKeySize(AES_KEY_BITS)
          .setRandomizedEncryptionRequired(true)
          .setUserAuthenticationRequired(false)
          .build()
      )
      generator.generateKey()
    } catch (error: Exception) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_KEY_CREATE,
        "Android Keystore 创建凭据密钥失败",
        error
      )
    }
  }

  private fun getExistingKey(): SecretKey {
    val keyStore = loadKeyStore()
    if (!keyStore.containsAlias(keyAlias)) {
      // Existing ciphertext must never cause a replacement key to be generated. Doing so would
      // make recovery impossible while merely changing the observed error.
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_KEY_MISSING,
        "Android Keystore 凭据密钥不存在"
      )
    }
    return keyFromStore(keyStore)
  }

  private fun loadKeyStore(): KeyStore {
    return try {
      KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
    } catch (error: Exception) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_KEY_ACCESS,
        "Android Keystore 不可用",
        error
      )
    }
  }

  private fun keyFromStore(keyStore: KeyStore): SecretKey {
    return try {
      keyStore.getKey(keyAlias, null) as? SecretKey
        ?: throw ConfigStoreException(
          ConfigStoreError.CREDENTIAL_KEY_ACCESS,
          "Android Keystore 凭据密钥类型无效"
        )
    } catch (error: ConfigStoreException) {
      throw error
    } catch (error: Exception) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_KEY_ACCESS,
        "Android Keystore 读取凭据密钥失败",
        error
      )
    }
  }

  private fun decodeBase64Strict(encoded: String): ByteArray {
    if (encoded.length % 4 != 0 || !BASE64_PATTERN.matches(encoded)) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_FORMAT,
        "Android 凭据密文不是有效的 Base64"
      )
    }
    return try {
      Base64.decode(encoded, Base64.NO_WRAP)
    } catch (error: IllegalArgumentException) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_FORMAT,
        "Android 凭据密文不是有效的 Base64",
        error
      )
    }
  }

  private fun decodeUtf8Strict(bytes: ByteArray): String {
    return try {
      Charsets.UTF_8.newDecoder()
        .onMalformedInput(CodingErrorAction.REPORT)
        .onUnmappableCharacter(CodingErrorAction.REPORT)
        .decode(ByteBuffer.wrap(bytes))
        .toString()
    } catch (error: Exception) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_FORMAT,
        "Android 凭据明文编码无效",
        error
      )
    }
  }

  companion object {
    const val PROVIDER = "android-keystore-aes-256-gcm"
    const val CREDENTIAL_VERSION = 1L
    const val DEFAULT_KEY_ALIAS = "com.why.ntfy_notifier.config.password.v1"

    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"
    private const val AES_KEY_BITS = 256
    private const val GCM_TAG_BITS = 128
    private const val GCM_TAG_BYTES = GCM_TAG_BITS / 8
    private const val MIN_IV_BYTES = 12
    private const val MAX_IV_BYTES = 32
    private val AAD =
      "com.why.ntfy_notifier/config/password/$PROVIDER/v$CREDENTIAL_VERSION"
        .toByteArray(Charsets.UTF_8)
    private val BASE64_PATTERN = Regex("^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$")
  }
}
