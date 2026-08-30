package app.ntfy.notifier

import android.content.Context
import android.system.Os
import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.io.File
import java.security.KeyStore
import java.util.UUID
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class ConfigStoreInstrumentationTest {
  private lateinit var context: Context
  private lateinit var root: File
  private lateinit var keyAlias: String
  private lateinit var cipher: AndroidKeystoreCredentialCipher
  private lateinit var store: ConfigStore

  @Before
  fun setUp() {
    context = InstrumentationRegistry.getInstrumentation().targetContext.applicationContext
    root = File(context.cacheDir, "config-store-${UUID.randomUUID()}")
    assertTrue(root.mkdirs())
    keyAlias = "${AndroidKeystoreCredentialCipher.DEFAULT_KEY_ALIAS}.test.${UUID.randomUUID()}"
    deleteKey()
    cipher = AndroidKeystoreCredentialCipher(keyAlias)
    store = ConfigStore(root, cipher)
  }

  @After
  fun tearDown() {
    deleteKey()
    root.deleteRecursively()
  }

  @Test
  fun realKeystoreRoundtripUsesRandomAuthenticatedCiphertext() {
    val password = "密码-'-测试-🔐"

    val first = cipher.encrypt(password)
    val second = cipher.encrypt(password)

    assertEquals(AndroidKeystoreCredentialCipher.PROVIDER, first.provider)
    assertEquals(AndroidKeystoreCredentialCipher.CREDENTIAL_VERSION, first.version)
    assertNotEquals(first.ciphertext, second.ciphertext)
    assertEquals(password, cipher.decrypt(first))
    assertEquals(password, cipher.decrypt(second))
  }

  @Test
  fun tamperedCiphertextFailsAuthenticationWithoutLeakingPlaintext() {
    val password = "top-secret-value"
    val encrypted = cipher.encrypt(password)
    val packed = Base64.decode(encrypted.ciphertext, Base64.NO_WRAP)
    packed[packed.lastIndex] = (packed.last().toInt() xor 0x01).toByte()
    val tampered = encrypted.copy(
      ciphertext = Base64.encodeToString(packed, Base64.NO_WRAP)
    )

    val error = expectError(ConfigStoreError.CREDENTIAL_DECRYPT) {
      cipher.decrypt(tampered)
    }

    assertFalse(error.message.orEmpty().contains(password))
  }

  @Test
  fun missingAliasPreservesCiphertextUntilExplicitSaveRecovers() {
    val password = "must-not-be-lost"
    val encrypted = cipher.encrypt(password)
    writeV2Config(encrypted)
    writePreferences("dark")
    val original = configFile().readBytes()
    deleteKey()

    val error = expectError(ConfigStoreError.CREDENTIAL_KEY_MISSING) {
      store.loadPublicConfig()
    }

    assertArrayEquals(original, configFile().readBytes())
    assertFalse(keyExists())
    assertFalse(error.message.orEmpty().contains(password))

    val replacement = samplePublicConfig(password = "replacement", theme = "light")
    assertEquals(replacement, store.savePublicConfig(replacement))
    assertTrue(keyExists())
    assertFalse(original.contentEquals(configFile().readBytes()))
    assertEquals(replacement, store.loadPublicConfig())
  }

  @Test
  fun stage11V2LegacyCredentialMigratesAndDtosReturnDecryptedPassword() {
    val password = "v2-旧凭据-🔑"
    writeV2Config(
      CredentialEnvelope(
        provider = LEGACY_PROVIDER,
        version = 1,
        ciphertext = legacyCiphertext(password)
      ),
      server = "https://ntfy.example.com/base",
      username = "alice",
      topic = "alerts",
      autoStart = true,
      autoCopyOtp = true,
      allowInsecureHttp = false
    )
    writePreferences("dark")

    val publicConfig = store.loadPublicConfig()
    val subscriberConfig = store.loadSubscriberConfig()

    assertEquals("https://ntfy.example.com/base", publicConfig.server)
    assertEquals("alice", publicConfig.username)
    assertEquals(password, publicConfig.password)
    assertEquals("alerts", publicConfig.topic)
    assertEquals("dark", publicConfig.themeMode)
    assertTrue(publicConfig.autoStart)
    assertTrue(publicConfig.autoCopyOtp)
    assertFalse(publicConfig.allowInsecureHttp)
    assertEquals(password, subscriberConfig.password)
    assertEquals(publicConfig.server, subscriberConfig.server)
    assertEquals(publicConfig.autoStart, subscriberConfig.autoStart)

    val migrated = JSONObject(configFile().readText())
    val credential = migrated.getJSONObject("credential")
    assertEquals(AndroidKeystoreCredentialCipher.PROVIDER, credential.getString("provider"))
    assertNotEquals(legacyCiphertext(password), credential.getString("ciphertext"))
    assertFalse(configFile().readText().contains(password))
  }

  @Test
  fun versionless119ConfigMigratesThemeHttpChoiceAndPassword() {
    val password = "legacy-1.1.9-凭据"
    val legacy = JSONObject()
      .put("server", "HTTP://EXAMPLE.COM")
      .put("username", "bob")
      .put("topic", "security")
      .put("theme_mode", "light")
      .put("auto_start", true)
      .put("auto_copy_otp", false)
      .put("password", "")
      .put("password_encrypted", legacyCiphertext(password))
    configFile().writeText(legacy.toString(2))

    val loaded = store.loadPublicConfig()

    assertEquals(password, loaded.password)
    assertEquals("light", loaded.themeMode)
    assertTrue(loaded.allowInsecureHttp)
    assertEquals("light", JSONObject(preferencesFile().readText()).getString("theme_mode"))
    val migrated = JSONObject(configFile().readText())
    assertEquals(2L, migrated.getLong("version"))
    assertEquals(
      AndroidKeystoreCredentialCipher.PROVIDER,
      migrated.getJSONObject("credential").getString("provider")
    )
    assertFalse(migrated.has("password"))
    assertFalse(migrated.has("theme_mode"))
  }

  @Test
  fun malformedLegacyUtf16PreservesOriginalBytesAndDoesNotCreateKey() {
    val invalidOddLengthUtf16 = Base64.encodeToString(byteArrayOf(0x61), Base64.NO_WRAP)
    val legacy = JSONObject()
      .put("server", "https://ntfy.example.com")
      .put("username", "alice")
      .put("topic", "alerts")
      .put("theme_mode", "system")
      .put("auto_start", false)
      .put("auto_copy_otp", false)
      .put("allow_insecure_http", false)
      .put("password", "")
      .put("password_encrypted", invalidOddLengthUtf16)
      .toString(2)
      .toByteArray(Charsets.UTF_8)
    configFile().writeBytes(legacy)

    expectError(ConfigStoreError.CREDENTIAL_FORMAT) {
      store.loadPublicConfig()
    }

    assertArrayEquals(legacy, configFile().readBytes())
    assertFalse(preferencesFile().exists())
    assertFalse(keyExists())
  }

  @Test
  fun corruptPreferencesAreSanitizedBeforeBackup() {
    val backupSecret = "must-not-enter-backup"
    val corrupt = "{not-valid-json:$backupSecret".toByteArray(Charsets.UTF_8)
    writeV2Config(
      CredentialEnvelope(LEGACY_PROVIDER, 1, legacyCiphertext("secret"))
    )
    preferencesFile().writeBytes(corrupt)

    val loaded = store.loadPublicConfig()

    assertEquals("system", loaded.themeMode)
    assertEquals("secret", loaded.password)
    assertSanitizedSystemPreferences(backupSecret)
  }

  @Test
  fun preferencesWithUnknownFieldsAreSanitizedBeforeBackup() {
    val backupSecret = "unknown-field-secret"
    val invalid = JSONObject()
      .put("theme_mode", "dark")
      .put("unexpected", backupSecret)
      .toString(2)
      .toByteArray(Charsets.UTF_8)
    writeV2Config(emptyAndroidCredential())
    preferencesFile().writeBytes(invalid)

    val loaded = store.loadPublicConfig()

    assertEquals("system", loaded.themeMode)
    assertSanitizedSystemPreferences(backupSecret)
  }

  @Test
  fun duplicateThemeKeysCannotHideBackupData() {
    val backupSecret = "duplicate-key-secret"
    writeV2Config(emptyAndroidCredential())
    preferencesFile().writeText(
      "{\"theme_mode\":\"$backupSecret\",\"theme_mode\":\"system\"}"
    )

    val loaded = store.loadPublicConfig()

    assertEquals("system", loaded.themeMode)
    assertSanitizedSystemPreferences(backupSecret)
  }

  @Test
  fun duplicateConfigKeysAreRejectedWithoutChangingSensitiveConfig() {
    val original = """
      {
        "version": 2,
        "server": "hidden-duplicate-value",
        "server": "https://ntfy.example.com",
        "username": "alice",
        "topic": "alerts",
        "allow_insecure_http": false,
        "auto_start": false,
        "auto_copy_otp": false,
        "credential": {
          "provider": "${AndroidKeystoreCredentialCipher.PROVIDER}",
          "version": ${AndroidKeystoreCredentialCipher.CREDENTIAL_VERSION},
          "ciphertext": ""
        }
      }
    """.trimIndent().toByteArray(Charsets.UTF_8)
    configFile().writeBytes(original)

    expectError(ConfigStoreError.CONFIG_FORMAT) {
      store.loadPublicConfig()
    }

    assertArrayEquals(original, configFile().readBytes())
    assertFalse(preferencesFile().exists())
    assertFalse(keyExists())
  }

  @Test
  fun missingPreferencesUseSystemThemeWithoutCreatingBackupFile() {
    writeV2Config(emptyAndroidCredential())

    val loaded = store.loadPublicConfig()

    assertEquals("system", loaded.themeMode)
    assertFalse(preferencesFile().exists())
  }

  @Test
  fun illegalThemeIsSanitizedBeforeBackup() {
    val backupSecret = "secret-theme"
    writeV2Config(emptyAndroidCredential())
    preferencesFile().writeText(JSONObject().put("theme_mode", backupSecret).toString(2))

    val loaded = store.loadPublicConfig()

    assertEquals("system", loaded.themeMode)
    assertSanitizedSystemPreferences(backupSecret)
  }

  @Test
  fun oversizedPreferencesAreReplacedWithStrictSystemTheme() {
    writeV2Config(emptyAndroidCredential())
    preferencesFile().writeBytes(ByteArray(1024 * 1024 + 1) { 'S'.code.toByte() })

    val loaded = store.loadPublicConfig()

    assertEquals("system", loaded.themeMode)
    assertSanitizedSystemPreferences("SSSSSSSS")
  }

  @Test
  fun unreadablePreferencesAreAtomicallyReplacedWithStrictSystemTheme() {
    writeV2Config(emptyAndroidCredential())
    writePreferences("dark")
    Os.chmod(preferencesFile().absolutePath, 0)

    val loaded = try {
      store.loadPublicConfig()
    } finally {
      if (preferencesFile().exists()) {
        Os.chmod(preferencesFile().absolutePath, OWNER_READ_WRITE_MODE)
      }
    }

    assertEquals("system", loaded.themeMode)
    assertSanitizedSystemPreferences("dark")
  }

  @Test
  fun failedPreferenceSanitizationDeletesOnlyTheAllowlistedPath() {
    val backupSecret = "delete-this-backup-secret"
    val unrelated = File(root, "unrelated-data").apply { writeText("keep-me") }
    writeV2Config(emptyAndroidCredential())
    preferencesFile().writeText("{invalid:$backupSecret")
    val failingWriter = object : ConfigFileWriter {
      private val delegate = AtomicConfigFileWriter()

      override fun write(target: File, bytes: ByteArray) {
        if (target.name == "preferences.json") {
          throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "测试偏好清洗写入失败")
        }
        delegate.write(target, bytes)
      }

      override fun delete(target: File) {
        delegate.delete(target)
      }
    }
    val failingStore = ConfigStore(root, cipher, failingWriter)

    val loaded = failingStore.loadPublicConfig()

    assertEquals("system", loaded.themeMode)
    assertFalse(preferencesFile().exists())
    assertEquals("keep-me", unrelated.readText())
  }

  @Test
  fun strictConfigDocumentRejectsTrailingDataWithoutChangingOriginalBytes() {
    val invalid = (JSONObject()
      .put("version", 2)
      .put("server", "https://ntfy.example.com")
      .put("username", "alice")
      .put("topic", "alerts")
      .put("allow_insecure_http", false)
      .put("auto_start", false)
      .put("auto_copy_otp", false)
      .put(
        "credential",
        JSONObject()
          .put("provider", LEGACY_PROVIDER)
          .put("version", 1)
          .put("ciphertext", "")
      )
      .toString(2) + "\ntrue")
      .toByteArray(Charsets.UTF_8)
    configFile().writeBytes(invalid)

    expectError(ConfigStoreError.CONFIG_FORMAT) {
      store.loadPublicConfig()
    }

    assertArrayEquals(invalid, configFile().readBytes())
    assertFalse(preferencesFile().exists())
    assertFalse(keyExists())
  }

  @Test
  fun unreadableConfigPathIsNotTreatedAsMissingConfiguration() {
    assertTrue(configFile().mkdir())

    expectError(ConfigStoreError.CONFIG_IO) {
      store.loadPublicConfig()
    }
  }

  @Test
  fun emptyPasswordDoesNotCreateKeystoreAliasAndStillRoundtrips() {
    val input = samplePublicConfig(password = "", theme = "light")

    assertEquals(input, store.savePublicConfig(input))
    assertFalse(keyExists())
    assertEquals(input, store.loadPublicConfig())

    val credential = JSONObject(configFile().readText()).getJSONObject("credential")
    assertEquals(AndroidKeystoreCredentialCipher.PROVIDER, credential.getString("provider"))
    assertEquals("", credential.getString("ciphertext"))
  }

  @Test
  fun oversizedSerializedConfigDoesNotCreateKeyOrChangeExistingFiles() {
    store.savePublicConfig(samplePublicConfig(password = "", theme = "dark"))
    val originalConfig = configFile().readBytes()
    val originalPreferences = preferencesFile().readBytes()
    assertFalse(keyExists())

    val oversized = samplePublicConfig(password = "must-not-create-key", theme = "light")
      .copy(server = "\u0000".repeat(200_000))
    expectError(ConfigStoreError.CONFIG_FORMAT) {
      store.savePublicConfig(oversized)
    }

    assertArrayEquals(originalConfig, configFile().readBytes())
    assertArrayEquals(originalPreferences, preferencesFile().readBytes())
    assertFalse(keyExists())
  }

  @Test
  fun configWriteFailureRollsPreferencesBackToExactOriginalBytes() {
    val originalPreferences = "{\n  \"theme_mode\": \"dark\"\n}".toByteArray(Charsets.UTF_8)
    preferencesFile().writeBytes(originalPreferences)
    val failingWriter = object : ConfigFileWriter {
      private val delegate = AtomicConfigFileWriter()

      override fun write(target: File, bytes: ByteArray) {
        if (target.name == "config.json") {
          throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "测试配置写入失败")
        }
        delegate.write(target, bytes)
      }

      override fun delete(target: File) {
        delegate.delete(target)
      }
    }
    val failingStore = ConfigStore(root, cipher, failingWriter)

    expectError(ConfigStoreError.CONFIG_IO) {
      failingStore.savePublicConfig(samplePublicConfig(password = "new-secret", theme = "light"))
    }

    assertArrayEquals(originalPreferences, preferencesFile().readBytes())
    assertFalse(configFile().exists())
  }

  @Test
  fun configWriteFailureNeverRestoresUnsafePreferenceBytes() {
    val backupSecret = "must-not-be-restored"
    preferencesFile().writeText("{invalid:$backupSecret")
    val failingWriter = object : ConfigFileWriter {
      private val delegate = AtomicConfigFileWriter()

      override fun write(target: File, bytes: ByteArray) {
        if (target.name == "config.json") {
          throw ConfigStoreException(ConfigStoreError.CONFIG_IO, "测试配置写入失败")
        }
        delegate.write(target, bytes)
      }

      override fun delete(target: File) {
        delegate.delete(target)
      }
    }
    val failingStore = ConfigStore(root, cipher, failingWriter)

    expectError(ConfigStoreError.CONFIG_IO) {
      failingStore.savePublicConfig(samplePublicConfig(password = "new-secret", theme = "light"))
    }

    assertFalse(configFile().exists())
    assertSanitizedSystemPreferences(backupSecret)
  }

  @Test
  fun separateInstancesSerializeThreadsBeforeTakingFileLock() {
    val secondStore = ConfigStore(root, cipher)
    val start = CountDownLatch(1)
    val executor = Executors.newFixedThreadPool(4)
    try {
      val futures = (0 until 12).map { index ->
        executor.submit<PublicConfig> {
          assertTrue(start.await(10, TimeUnit.SECONDS))
          val config = samplePublicConfig(
            password = "password-$index",
            theme = if (index % 2 == 0) "light" else "dark"
          )
          (if (index % 2 == 0) store else secondStore).savePublicConfig(config)
        }
      }
      start.countDown()
      futures.forEach { it.get(30, TimeUnit.SECONDS) }

      val loaded = store.loadPublicConfig()
      assertTrue(loaded.password.startsWith("password-"))
      assertTrue(loaded.themeMode == "light" || loaded.themeMode == "dark")
      assertEquals(2L, JSONObject(configFile().readText()).getLong("version"))
    } finally {
      executor.shutdownNow()
    }
  }

  private fun samplePublicConfig(password: String, theme: String) = PublicConfig(
    server = "https://ntfy.example.com",
    username = "alice",
    password = password,
    topic = "alerts",
    themeMode = theme,
    autoStart = true,
    autoCopyOtp = true,
    allowInsecureHttp = false
  )

  private fun writeV2Config(
    credential: CredentialEnvelope,
    server: String = "https://ntfy.example.com",
    username: String = "alice",
    topic: String = "alerts",
    autoStart: Boolean = false,
    autoCopyOtp: Boolean = false,
    allowInsecureHttp: Boolean = false
  ) {
    val root = JSONObject()
      .put("version", 2)
      .put("server", server)
      .put("username", username)
      .put("topic", topic)
      .put("allow_insecure_http", allowInsecureHttp)
      .put("auto_start", autoStart)
      .put("auto_copy_otp", autoCopyOtp)
      .put(
        "credential",
        JSONObject()
          .put("provider", credential.provider)
          .put("version", credential.version)
          .put("ciphertext", credential.ciphertext)
      )
    configFile().writeText(root.toString(2))
  }

  private fun writePreferences(theme: String) {
    preferencesFile().writeText(JSONObject().put("theme_mode", theme).toString(2))
  }

  private fun emptyAndroidCredential() = CredentialEnvelope(
    AndroidKeystoreCredentialCipher.PROVIDER,
    AndroidKeystoreCredentialCipher.CREDENTIAL_VERSION,
    ""
  )

  private fun assertSanitizedSystemPreferences(forbiddenText: String) {
    assertTrue(preferencesFile().isFile)
    val raw = preferencesFile().readText()
    assertFalse(raw.contains(forbiddenText))
    val preferences = JSONObject(raw)
    assertEquals(1, preferences.length())
    assertEquals("system", preferences.getString("theme_mode"))
  }

  private fun legacyCiphertext(password: String): String {
    return Base64.encodeToString(password.toByteArray(Charsets.UTF_16LE), Base64.NO_WRAP)
  }

  private fun configFile() = File(root, "config.json")
  private fun preferencesFile() = File(root, "preferences.json")

  private fun keyStore(): KeyStore = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }

  private fun keyExists(): Boolean = keyStore().containsAlias(keyAlias)

  private fun deleteKey() {
    val keyStore = keyStore()
    if (keyStore.containsAlias(keyAlias)) keyStore.deleteEntry(keyAlias)
  }

  private fun expectError(
    code: ConfigStoreError,
    action: () -> Unit
  ): ConfigStoreException {
    try {
      action()
      fail("Expected ConfigStoreException with code $code")
    } catch (error: ConfigStoreException) {
      assertEquals(code, error.code)
      return error
    }
    throw AssertionError("unreachable")
  }

  companion object {
    private const val LEGACY_PROVIDER = "legacy-utf16le-base64"
    private const val OWNER_READ_WRITE_MODE = 384
  }
}
