package app.ntfy.notifier

import android.content.Context
import android.system.ErrnoException
import android.system.Os
import android.system.OsConstants
import android.util.Base64
import android.util.JsonReader
import android.util.JsonToken
import android.util.Log
import org.json.JSONException
import org.json.JSONObject
import org.json.JSONTokener
import java.io.File
import java.io.FileNotFoundException
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException
import java.io.RandomAccessFile
import java.io.StringReader
import java.net.InetAddress
import java.net.URI
import java.nio.ByteBuffer
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.locks.ReentrantLock
import kotlin.concurrent.withLock

data class PublicConfig(
  val server: String,
  val username: String,
  val password: String,
  val topic: String,
  val themeMode: String,
  val autoStart: Boolean,
  val autoCopyOtp: Boolean,
  val allowInsecureHttp: Boolean
)

data class SubscriberConfig(
  val server: String,
  val username: String,
  val password: String,
  val topic: String,
  val autoStart: Boolean,
  val autoCopyOtp: Boolean,
  val allowInsecureHttp: Boolean
)

enum class ConfigStoreError {
  CONFIG_FORMAT,
  CONFIG_VERSION,
  CONFIG_IO,
  CONFIG_LOCK,
  CONFIG_ROLLBACK,
  THEME_INVALID,
  CREDENTIAL_PROVIDER,
  CREDENTIAL_VERSION,
  CREDENTIAL_FORMAT,
  CREDENTIAL_KEY_CREATE,
  CREDENTIAL_KEY_ACCESS,
  CREDENTIAL_KEY_MISSING,
  CREDENTIAL_ENCRYPT,
  CREDENTIAL_DECRYPT
}

class ConfigStoreException(
  val code: ConfigStoreError,
  safeDescription: String,
  cause: Throwable? = null
) : Exception("${code.name}: $safeDescription", cause)

/**
 * Cross-process configuration owner for Android. Every read, migration and write takes both an
 * in-process ReentrantLock and an OS-visible FileChannel lock on .config.lock.
 */
class ConfigStore internal constructor(
  private val rootDirectory: File,
  private val credentialCipher: CredentialCipher,
  private val fileWriter: ConfigFileWriter
) {
  private val configFile = File(rootDirectory, CONFIG_FILE_NAME)
  private val preferencesFile = File(rootDirectory, PREFERENCES_FILE_NAME)
  private val lockFile = File(rootDirectory, LOCK_FILE_NAME)

  constructor(context: Context) : this(
    context.applicationContext.dataDir,
    AndroidKeystoreCredentialCipher(),
    AtomicConfigFileWriter()
  )

  internal constructor(
    rootDirectory: File,
    credentialCipher: CredentialCipher
  ) : this(
    rootDirectory,
    credentialCipher,
    AtomicConfigFileWriter()
  )

  fun loadPublicConfig(): PublicConfig = withStorageLock {
    loadPublicConfigLocked()
  }

  fun savePublicConfig(config: PublicConfig): PublicConfig = withStorageLock {
    validateTheme(config.themeMode)
    val passwordUtf8Bytes = validatePublicConfigInput(config)
    val preferenceBytes = serializePreferences(config.themeMode)
    val storedWithoutCredential = StoredConfig(
      version = CONFIG_VERSION,
      server = config.server,
      username = config.username,
      topic = config.topic,
      allowInsecureHttp = config.allowInsecureHttp,
      autoStart = config.autoStart,
      autoCopyOtp = config.autoCopyOtp,
      credential = preflightCredential(passwordUtf8Bytes)
    )
    // Serialize an upper-bound Android credential envelope before touching Keystore. Oversized
    // input therefore cannot create an orphan key or change either on-disk file.
    serializeStoredConfig(storedWithoutCredential)

    val credential = credentialCipher.encrypt(config.password)
    val configBytes = serializeStoredConfig(storedWithoutCredential.copy(credential = credential))
    writeConfigAndPreferencesLocked(configBytes, preferenceBytes)
    config
  }

  fun loadSubscriberConfig(): SubscriberConfig {
    val config = loadPublicConfig()
    return SubscriberConfig(
      server = config.server,
      username = config.username,
      password = config.password,
      topic = config.topic,
      autoStart = config.autoStart,
      autoCopyOtp = config.autoCopyOtp,
      allowInsecureHttp = config.allowInsecureHttp
    )
  }

  /**
   * Reads only the non-secret policy needed by BootReceiver. This deliberately avoids the normal
   * public-config path: boot policy checks must not touch preferences, migrate configuration, or
   * access Android Keystore before the foreground service has been promoted.
   */
  fun loadSubscriberAutoStartPolicy(): Boolean = withStorageLock {
    val original = readOptionalFile(configFile, "读取开机订阅策略失败")
      ?: return@withStorageLock false
    val root = parseJsonObject(original, "开机订阅策略格式无效")
    if (root.has("version")) {
      val version = requireLong(root, "version")
      if (version != CONFIG_VERSION) {
        throw ConfigStoreException(
          ConfigStoreError.CONFIG_VERSION,
          "不支持的配置版本"
        )
      }
    }
    requireBoolean(root, "auto_start")
  }

  private fun loadPublicConfigLocked(): PublicConfig {
    // Preferences are in Android's backup allowlist. Validate or sanitize them before touching
    // the sensitive config so malformed config/credential failures cannot leave arbitrary backup
    // data in preferences.json.
    val theme = loadThemeOrSystemLocked()
    val original = readOptionalFile(configFile, "读取配置文件失败")
    if (original == null) {
      return defaultPublicConfig(theme)
    }
    val root = parseJsonObject(original, "配置文件格式无效")
    return if (!root.has("version")) {
      migrateLegacyV1Locked(root)
    } else {
      val version = requireLong(root, "version")
      if (version != CONFIG_VERSION) {
        throw ConfigStoreException(
          ConfigStoreError.CONFIG_VERSION,
          "不支持的配置版本"
        )
      }
      loadV2Locked(parseStoredV2(root), theme)
    }
  }

  private fun loadV2Locked(stored: StoredConfig, theme: String): PublicConfig {
    val password = when (stored.credential.provider) {
      AndroidKeystoreCredentialCipher.PROVIDER -> credentialCipher.decrypt(stored.credential)
      LEGACY_CREDENTIAL_PROVIDER -> {
        if (stored.credential.version != LEGACY_CREDENTIAL_VERSION) {
          throw ConfigStoreException(
            ConfigStoreError.CREDENTIAL_VERSION,
            "不支持的旧 Android 凭据版本"
          )
        }
        val plaintext = decodeLegacyCredential(stored.credential.ciphertext)
        val migratedCredential = credentialCipher.encrypt(plaintext)
        val migrated = stored.copy(credential = migratedCredential)
        // Preferences were already validated or sanitized before sensitive config migration.
        fileWriter.write(configFile, serializeStoredConfig(migrated))
        plaintext
      }
      else -> throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_PROVIDER,
        "不支持的 Android 凭据提供方"
      )
    }

    return PublicConfig(
      server = stored.server,
      username = stored.username,
      password = password,
      topic = stored.topic,
      themeMode = theme,
      autoStart = stored.autoStart,
      autoCopyOtp = stored.autoCopyOtp,
      allowInsecureHttp = stored.allowInsecureHttp
    )
  }

  private fun migrateLegacyV1Locked(root: JSONObject): PublicConfig {
    requireExactKeys(root, LEGACY_REQUIRED_KEYS, LEGACY_OPTIONAL_KEYS)
    val server = requireString(root, "server")
    val username = requireString(root, "username")
    val topic = requireString(root, "topic")
    val theme = requireString(root, "theme_mode")
    val autoStart = requireBoolean(root, "auto_start")
    val autoCopyOtp = requireBoolean(root, "auto_copy_otp")
    val allowInsecureHttp = optionalBoolean(root, "allow_insecure_http")
      ?: requiresInsecureHttpOptIn(server)
    val legacyPlaintext = optionalString(root, "password") ?: ""
    val legacyCiphertext = optionalString(root, "password_encrypted") ?: ""
    validateTheme(theme)

    val password = if (legacyCiphertext.isNotEmpty()) {
      decodeLegacyCredential(legacyCiphertext)
    } else {
      legacyPlaintext
    }
    val credential = credentialCipher.encrypt(password)
    val migrated = StoredConfig(
      version = CONFIG_VERSION,
      server = server,
      username = username,
      topic = topic,
      allowInsecureHttp = allowInsecureHttp,
      autoStart = autoStart,
      autoCopyOtp = autoCopyOtp,
      credential = credential
    )

    // Complete parsing, strict decoding, Keystore encryption and serialization before changing
    // either file. If config replacement fails, the prior backup-safe preference state is restored.
    val configBytes = serializeStoredConfig(migrated)
    val preferenceBytes = serializePreferences(theme)
    writeConfigAndPreferencesLocked(configBytes, preferenceBytes)

    return PublicConfig(
      server = server,
      username = username,
      password = password,
      topic = topic,
      themeMode = theme,
      autoStart = autoStart,
      autoCopyOtp = autoCopyOtp,
      allowInsecureHttp = allowInsecureHttp
    )
  }

  private fun writeConfigAndPreferencesLocked(
    configBytes: ByteArray,
    preferenceBytes: ByteArray
  ) {
    // Ensure rollback material is itself safe for Android backup. Invalid legacy bytes must never
    // be restored if the following sensitive config write fails.
    val previousTheme = loadThemeOrSystemLocked()
    val oldPreferences = if (
      readOptionalFile(preferencesFile, "读取旧界面偏好失败") == null
    ) {
      null
    } else {
      // Never retain raw bytes as rollback material. Even if a best-effort cleanup could not
      // replace a strange filesystem entry, any later rollback is canonical and backup-safe.
      serializePreferences(previousTheme)
    }

    fileWriter.write(preferencesFile, preferenceBytes)
    try {
      fileWriter.write(configFile, configBytes)
    } catch (primary: ConfigStoreException) {
      try {
        if (oldPreferences == null) {
          fileWriter.delete(preferencesFile)
        } else {
          fileWriter.write(preferencesFile, oldPreferences)
        }
      } catch (rollback: Exception) {
        val error = ConfigStoreException(
          ConfigStoreError.CONFIG_ROLLBACK,
          "配置写入失败且界面偏好回滚失败",
          rollback
        )
        error.addSuppressed(primary)
        throw error
      }
      throw primary
    }
  }

  private fun loadThemeOrSystemLocked(): String {
    val raw = try {
      readOptionalFile(preferencesFile, "读取界面偏好失败")
        ?: return DEFAULT_THEME
    } catch (_: ConfigStoreException) {
      replacePreferencesOrDeleteLocked(DEFAULT_THEME)
      return DEFAULT_THEME
    }
    return try {
      val root = parseJsonObject(raw, "界面偏好格式无效")
      requireExactKeys(root, PREFERENCE_KEYS, emptySet())
      val theme = requireString(root, "theme_mode").also(::validateTheme)
      val canonical = serializePreferences(theme)
      if (!raw.contentEquals(canonical)) {
        // Canonicalization also removes duplicate keys and non-semantic bytes that JSONObject
        // would otherwise hide while the raw allowlisted file remained backup-eligible.
        replacePreferencesOrDeleteLocked(theme)
      }
      theme
    } catch (_: ConfigStoreException) {
      replacePreferencesOrDeleteLocked(DEFAULT_THEME)
      DEFAULT_THEME
    }
  }

  private fun replacePreferencesOrDeleteLocked(theme: String) {
    val sanitized = serializePreferences(theme)
    try {
      fileWriter.write(preferencesFile, sanitized)
    } catch (_: Exception) {
      // Atomic replacement failed before commit. Remove only the exact allowlisted path so stale
      // arbitrary bytes cannot be backed up; never recurse into a non-regular path.
      try {
        fileWriter.delete(preferencesFile)
      } catch (_: Exception) {
        Log.w(LOG_TAG, "PREFERENCES_SANITIZE_DELETE: 删除不安全界面偏好失败")
      }
    }
  }

  private fun parseStoredV2(root: JSONObject): StoredConfig {
    requireExactKeys(root, V2_REQUIRED_KEYS, V2_OPTIONAL_KEYS)
    val credentialObject = requireObject(root, "credential")
    requireExactKeys(credentialObject, CREDENTIAL_KEYS, emptySet())
    return StoredConfig(
      version = requireLong(root, "version"),
      server = requireString(root, "server"),
      username = requireString(root, "username"),
      topic = requireString(root, "topic"),
      allowInsecureHttp = optionalBoolean(root, "allow_insecure_http") ?: false,
      autoStart = requireBoolean(root, "auto_start"),
      autoCopyOtp = requireBoolean(root, "auto_copy_otp"),
      credential = CredentialEnvelope(
        provider = requireString(credentialObject, "provider"),
        version = requireLong(credentialObject, "version"),
        ciphertext = requireString(credentialObject, "ciphertext")
      )
    )
  }

  private fun serializeStoredConfig(config: StoredConfig): ByteArray {
    val credential = JSONObject()
      .put("provider", config.credential.provider)
      .put("version", config.credential.version)
      .put("ciphertext", config.credential.ciphertext)
    val root = JSONObject()
      .put("version", config.version)
      .put("server", config.server)
      .put("username", config.username)
      .put("topic", config.topic)
      .put("allow_insecure_http", config.allowInsecureHttp)
      .put("auto_start", config.autoStart)
      .put("auto_copy_otp", config.autoCopyOtp)
      .put("credential", credential)
    return requireSerializedSize(
      root.toString(2).toByteArray(Charsets.UTF_8),
      "配置数据超过大小限制"
    )
  }

  private fun serializePreferences(theme: String): ByteArray {
    return requireSerializedSize(
      JSONObject()
      .put("theme_mode", theme)
      .toString(2)
      .toByteArray(Charsets.UTF_8),
      "界面偏好超过大小限制"
    )
  }

  private fun validatePublicConfigInput(config: PublicConfig): Int {
    var totalBytes = 0L
    var passwordBytes = 0
    listOf(
      "server" to config.server,
      "username" to config.username,
      "password" to config.password,
      "topic" to config.topic,
      "theme_mode" to config.themeMode
    ).forEach { (name, value) ->
      val fieldBytes = strictUtf8Length(value)
      if (name == "password") passwordBytes = fieldBytes
      totalBytes += fieldBytes
      if (totalBytes > MAX_PUBLIC_CONFIG_UTF8_BYTES) {
        throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "配置文本超过大小限制")
      }
    }
    return passwordBytes
  }

  private fun strictUtf8Length(value: String): Int {
    var bytes = 0L
    var index = 0
    while (index < value.length) {
      val character = value[index]
      bytes += when {
        character.code <= 0x7f -> 1
        character.code <= 0x7ff -> 2
        character.isHighSurrogate() -> {
          if (index + 1 >= value.length || !value[index + 1].isLowSurrogate()) {
            throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "配置文本包含无效 Unicode")
          }
          index += 1
          4
        }
        character.isLowSurrogate() -> throw ConfigStoreException(
          ConfigStoreError.CONFIG_FORMAT,
          "配置文本包含无效 Unicode"
        )
        else -> 3
      }
      if (bytes > MAX_PUBLIC_CONFIG_UTF8_BYTES) {
        throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "配置文本超过大小限制")
      }
      index += 1
    }
    return bytes.toInt()
  }

  private fun preflightCredential(passwordUtf8Bytes: Int): CredentialEnvelope {
    val encodedLength = if (passwordUtf8Bytes == 0) {
      0
    } else {
      val packedLength = 1 + MAX_GCM_IV_BYTES + passwordUtf8Bytes + GCM_TAG_BYTES
      ((packedLength + 2) / 3) * 4
    }
    return CredentialEnvelope(
      AndroidKeystoreCredentialCipher.PROVIDER,
      AndroidKeystoreCredentialCipher.CREDENTIAL_VERSION,
      "A".repeat(encodedLength)
    )
  }

  private fun requireSerializedSize(bytes: ByteArray, safeDescription: String): ByteArray {
    if (bytes.size > MAX_JSON_BYTES) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, safeDescription)
    }
    return bytes
  }

  private fun decodeLegacyCredential(encoded: String): String {
    if (encoded.isEmpty()) return ""
    if (encoded.length % 4 != 0 || !BASE64_PATTERN.matches(encoded)) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_FORMAT,
        "旧 Android 凭据不是有效的 Base64"
      )
    }
    val bytes = try {
      Base64.decode(encoded, Base64.NO_WRAP)
    } catch (error: IllegalArgumentException) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_FORMAT,
        "旧 Android 凭据不是有效的 Base64",
        error
      )
    }
    if (bytes.size % 2 != 0) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_FORMAT,
        "旧 Android 凭据长度无效"
      )
    }
    return try {
      StandardCharsets.UTF_16LE.newDecoder()
        .onMalformedInput(CodingErrorAction.REPORT)
        .onUnmappableCharacter(CodingErrorAction.REPORT)
        .decode(ByteBuffer.wrap(bytes))
        .toString()
    } catch (error: Exception) {
      throw ConfigStoreException(
        ConfigStoreError.CREDENTIAL_FORMAT,
        "旧 Android 凭据文本编码无效",
        error
      )
    } finally {
      bytes.fill(0)
    }
  }

  private fun validateTheme(theme: String) {
    if (theme !in ALLOWED_THEMES) {
      throw ConfigStoreException(ConfigStoreError.THEME_INVALID, "不支持的界面主题")
    }
  }

  private fun requiresInsecureHttpOptIn(server: String): Boolean {
    return try {
      val uri = URI(server.trim())
      uri.scheme.equals("http", ignoreCase = true) && uri.host != null && !isLoopbackHost(uri.host)
    } catch (_: Exception) {
      false
    }
  }

  private fun isLoopbackHost(rawHost: String): Boolean {
    val host = rawHost.removePrefix("[").removeSuffix("]")
    if (host.equals("localhost", ignoreCase = true)) return true
    val ipv4Parts = host.split('.')
    if (ipv4Parts.isNotEmpty() && ipv4Parts.all { it.isNotEmpty() && it.all(Char::isDigit) }) {
      return ipv4Parts.first().toIntOrNull() == 127
    }
    if (host.contains(':')) {
      return try {
        InetAddress.getByName(host).isLoopbackAddress
      } catch (_: Exception) {
        false
      }
    }
    return false
  }

  private fun defaultPublicConfig(theme: String) = PublicConfig(
    server = "",
    username = "",
    password = "",
    topic = "",
    themeMode = theme,
    autoStart = false,
    autoCopyOtp = false,
    allowInsecureHttp = false
  )

  private inline fun <T> withStorageLock(block: () -> T): T {
    try {
      if (!rootDirectory.exists() && !rootDirectory.mkdirs() && !rootDirectory.isDirectory) {
        throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "创建配置目录失败")
      }
      if (!rootDirectory.isDirectory) {
        throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "配置目录无效")
      }
      val processLock = ProcessLocks.forPath(lockFile.canonicalPath)
      return processLock.withLock {
        val lockHandle = RandomAccessFile(lockFile, "rw")
        try {
          val fileLock = lockHandle.channel.lock()
          try {
            block()
          } finally {
            try {
              fileLock.release()
            } catch (_: Exception) {
              // The operation may already have atomically committed. Lock cleanup must not turn
              // that committed state into a normal save failure at the caller.
              Log.w(LOG_TAG, "CONFIG_LOCK_RELEASE: 释放跨进程配置锁失败")
            }
          }
        } finally {
          try {
            lockHandle.close()
          } catch (_: Exception) {
            Log.w(LOG_TAG, "CONFIG_LOCK_CLOSE: 关闭跨进程配置锁失败")
          }
        }
      }
    } catch (error: ConfigStoreException) {
      throw error
    } catch (error: IOException) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_LOCK, "获取配置存储锁失败", error)
    } catch (error: RuntimeException) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_LOCK, "配置存储锁操作失败", error)
    }
  }

  private fun readFile(file: File, safeDescription: String): ByteArray {
    return try {
      FileInputStream(file).use { input ->
        val bytes = ByteArray(MAX_JSON_BYTES + 1)
        var size = 0
        while (size < bytes.size) {
          val count = input.read(bytes, size, bytes.size - size)
          if (count < 0) break
          size += count
        }
        if (size > MAX_JSON_BYTES) {
          throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "配置数据超过大小限制")
        }
        bytes.copyOf(size)
      }
    } catch (error: ConfigStoreException) {
      throw error
    } catch (error: IOException) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_IO, safeDescription, error)
    }
  }

  private fun readOptionalFile(file: File, safeDescription: String): ByteArray? {
    return try {
      readFile(file, safeDescription)
    } catch (error: ConfigStoreException) {
      if (error.cause is FileNotFoundException && pathIsMissing(file)) {
        null
      } else {
        throw error
      }
    }
  }

  private fun pathIsMissing(file: File): Boolean {
    return try {
      Os.lstat(file.absolutePath)
      false
    } catch (error: ErrnoException) {
      error.errno == OsConstants.ENOENT
    } catch (_: Exception) {
      false
    }
  }

  private fun parseJsonObject(bytes: ByteArray, safeDescription: String): JSONObject {
    val text = try {
      Charsets.UTF_8.newDecoder()
        .onMalformedInput(CodingErrorAction.REPORT)
        .onUnmappableCharacter(CodingErrorAction.REPORT)
        .decode(ByteBuffer.wrap(bytes))
        .toString()
    } catch (error: Exception) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, safeDescription, error)
    }
    rejectDuplicateJsonKeys(text, safeDescription)
    return try {
      val tokener = JSONTokener(text)
      val value = tokener.nextValue() as? JSONObject
        ?: throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, safeDescription)
      while (tokener.more()) {
        when (tokener.next()) {
          ' ', '\t', '\r', '\n' -> Unit
          else -> throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, safeDescription)
        }
      }
      value
    } catch (error: ConfigStoreException) {
      throw error
    } catch (error: JSONException) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, safeDescription, error)
    }
  }

  private fun rejectDuplicateJsonKeys(text: String, safeDescription: String) {
    try {
      JsonReader(StringReader(text)).use { reader ->
        reader.isLenient = false
        readStrictJsonValue(reader, 0)
        if (reader.peek() != JsonToken.END_DOCUMENT) {
          throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, safeDescription)
        }
      }
    } catch (error: ConfigStoreException) {
      throw error
    } catch (error: Exception) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, safeDescription, error)
    }
  }

  private fun readStrictJsonValue(reader: JsonReader, depth: Int) {
    if (depth > MAX_JSON_DEPTH) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "JSON 嵌套超过大小限制")
    }
    when (reader.peek()) {
      JsonToken.BEGIN_OBJECT -> {
        reader.beginObject()
        val names = mutableSetOf<String>()
        while (reader.hasNext()) {
          val name = reader.nextName()
          if (!names.add(name)) {
            throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "JSON 对象包含重复字段")
          }
          readStrictJsonValue(reader, depth + 1)
        }
        reader.endObject()
      }
      JsonToken.BEGIN_ARRAY -> {
        reader.beginArray()
        while (reader.hasNext()) readStrictJsonValue(reader, depth + 1)
        reader.endArray()
      }
      JsonToken.STRING, JsonToken.NUMBER -> reader.nextString()
      JsonToken.BOOLEAN -> reader.nextBoolean()
      JsonToken.NULL -> reader.nextNull()
      else -> throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "JSON 数据结构无效")
    }
  }

  private fun requireExactKeys(
    value: JSONObject,
    required: Set<String>,
    optional: Set<String>
  ) {
    val actual = mutableSetOf<String>()
    val keys = value.keys()
    while (keys.hasNext()) actual += keys.next()
    if (!actual.containsAll(required) || actual.any { it !in required && it !in optional }) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "配置字段不完整或包含未知字段")
    }
  }

  private fun requireString(value: JSONObject, key: String): String {
    return value.opt(key) as? String
      ?: throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "配置文本字段类型无效")
  }

  private fun optionalString(value: JSONObject, key: String): String? {
    if (!value.has(key)) return null
    return requireString(value, key)
  }

  private fun requireBoolean(value: JSONObject, key: String): Boolean {
    return value.opt(key) as? Boolean
      ?: throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "配置布尔字段类型无效")
  }

  private fun optionalBoolean(value: JSONObject, key: String): Boolean? {
    if (!value.has(key)) return null
    return requireBoolean(value, key)
  }

  private fun requireLong(value: JSONObject, key: String): Long {
    val number = value.opt(key)
    return when (number) {
      is Int -> number.toLong()
      is Long -> number
      else -> throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "配置整数字段类型无效")
    }
  }

  private fun requireObject(value: JSONObject, key: String): JSONObject {
    return value.opt(key) as? JSONObject
      ?: throw ConfigStoreException(ConfigStoreError.CONFIG_FORMAT, "配置对象字段类型无效")
  }

  private data class StoredConfig(
    val version: Long,
    val server: String,
    val username: String,
    val topic: String,
    val allowInsecureHttp: Boolean,
    val autoStart: Boolean,
    val autoCopyOtp: Boolean,
    val credential: CredentialEnvelope
  )

  companion object {
    private const val LOG_TAG = "ConfigStore"
    private const val CONFIG_VERSION = 2L
    private const val LEGACY_CREDENTIAL_PROVIDER = "legacy-utf16le-base64"
    private const val LEGACY_CREDENTIAL_VERSION = 1L
    private const val DEFAULT_THEME = "system"
    private const val CONFIG_FILE_NAME = "config.json"
    private const val PREFERENCES_FILE_NAME = "preferences.json"
    private const val LOCK_FILE_NAME = ".config.lock"
    private const val MAX_JSON_BYTES = 1024 * 1024
    private const val MAX_JSON_DEPTH = 32
    private const val MAX_PUBLIC_CONFIG_UTF8_BYTES = 512 * 1024L
    private const val MAX_GCM_IV_BYTES = 32
    private const val GCM_TAG_BYTES = 16
    private val ALLOWED_THEMES = setOf("system", "light", "dark")
    private val BASE64_PATTERN = Regex("^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$")
    private val V2_REQUIRED_KEYS = setOf(
      "version",
      "server",
      "username",
      "topic",
      "auto_start",
      "auto_copy_otp",
      "credential"
    )
    private val V2_OPTIONAL_KEYS = setOf("allow_insecure_http")
    private val CREDENTIAL_KEYS = setOf("provider", "version", "ciphertext")
    private val LEGACY_REQUIRED_KEYS = setOf(
      "server",
      "username",
      "topic",
      "theme_mode",
      "auto_start",
      "auto_copy_otp"
    )
    private val LEGACY_OPTIONAL_KEYS = setOf(
      "allow_insecure_http",
      "password",
      "password_encrypted"
    )
    private val PREFERENCE_KEYS = setOf("theme_mode")
  }
}

internal interface ConfigFileWriter {
  fun write(target: File, bytes: ByteArray)
  fun delete(target: File)
}

internal class AtomicConfigFileWriter : ConfigFileWriter {
  override fun write(target: File, bytes: ByteArray) {
    val directory = target.parentFile
      ?: throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "数据文件缺少父目录")
    if (!directory.isDirectory) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "数据文件父目录无效")
    }

    val temporary = try {
      File.createTempFile(".${target.name}.", ".tmp", directory)
    } catch (error: IOException) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "创建唯一临时文件失败", error)
    }
    var renamed = false
    try {
      FileOutputStream(temporary).use { output ->
        output.write(bytes)
        output.flush()
        output.fd.sync()
      }
      Os.rename(temporary.absolutePath, target.absolutePath)
      renamed = true
      syncDirectoryBestEffort(directory)
    } catch (error: ConfigStoreException) {
      throw error
    } catch (error: Exception) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "原子替换数据文件失败", error)
    } finally {
      if (!renamed) temporary.delete()
    }
  }

  override fun delete(target: File) {
    val directory = target.parentFile
      ?: throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "数据文件缺少父目录")
    try {
      Os.remove(target.absolutePath)
    } catch (error: ErrnoException) {
      if (error.errno == OsConstants.ENOENT) return
      throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "删除数据文件失败", error)
    } catch (error: Exception) {
      throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "删除数据文件失败", error)
    }
    syncDirectoryBestEffort(directory)
  }

  private fun syncDirectoryBestEffort(directory: File) {
    val descriptor = try {
      Os.open(directory.absolutePath, OsConstants.O_RDONLY, 0)
    } catch (_: Exception) {
      Log.w(LOG_TAG, "CONFIG_DIRECTORY_SYNC: 无法打开数据目录进行刷新")
      return
    }
    try {
      Os.fsync(descriptor)
    } catch (_: Exception) {
      // The atomic rename/delete has already committed. Report a durability warning without
      // converting a committed operation into a normal failure that would trigger false rollback.
      Log.w(LOG_TAG, "CONFIG_DIRECTORY_SYNC: 数据目录刷新失败")
    } finally {
      try {
        Os.close(descriptor)
      } catch (_: Exception) {
        // Directory data has already been synced. Closing failure has no safe recovery action.
      }
    }
  }

  companion object {
    private const val LOG_TAG = "ConfigStore"
  }
}

private object ProcessLocks {
  private val locks = ConcurrentHashMap<String, ReentrantLock>()

  fun forPath(path: String): ReentrantLock = locks.getOrPut(path) { ReentrantLock() }
}
