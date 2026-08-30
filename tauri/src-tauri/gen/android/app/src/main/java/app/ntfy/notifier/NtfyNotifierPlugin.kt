package app.ntfy.notifier

import android.Manifest
import android.app.Activity
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.concurrent.Executors

class NtfyNotifierPlugin(private val activity: Activity) : Plugin(activity) {

  companion object {
    private val CONFIG_FIELDS = setOf(
      "server",
      "username",
      "password",
      "topic",
      "theme_mode",
      "auto_start",
      "auto_copy_otp",
      "allow_insecure_http"
    )
    private val configExecutor = Executors.newSingleThreadExecutor { task ->
      Thread(task, "ntfy-config-store").apply { isDaemon = true }
    }
  }

  private val configStore = ConfigStore(activity.applicationContext)

  @Command
  fun getConfig(invoke: Invoke) {
    configExecutor.execute {
      try {
        invoke.resolve(configStore.loadPublicConfig().toJsObject())
      } catch (error: Exception) {
        invoke.reject(safeConfigError(error))
      }
    }
  }

  @Command
  fun saveConfig(invoke: Invoke) {
    val config = try {
      val value = invoke.getArgs().getJSObject("config")
        ?: throw IllegalArgumentException("CONFIG_INPUT: missing config")
      parsePublicConfig(value)
    } catch (error: Exception) {
      invoke.reject(safeConfigError(error))
      return
    }

    configExecutor.execute {
      try {
        val saved = configStore.savePublicConfig(config)
        invoke.resolve(saved.toJsObject())
      } catch (error: Exception) {
        invoke.reject(safeConfigError(error))
      }
    }
  }

  @Command
  fun startService(invoke: Invoke) {
    requestNotificationPermission()
    NotificationService.sendAction(
      activity.applicationContext,
      NotificationService.ACTION_START
    )
    invoke.resolve()
  }

  @Command
  fun reconfigureService(invoke: Invoke) {
    requestNotificationPermission()
    NotificationService.sendAction(
      activity.applicationContext,
      NotificationService.ACTION_RECONFIGURE
    )
    invoke.resolve()
  }

  private fun requestNotificationPermission() {
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      if (ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) !=
        PackageManager.PERMISSION_GRANTED
      ) {
        ActivityCompat.requestPermissions(
          activity,
          arrayOf(Manifest.permission.POST_NOTIFICATIONS),
          1001
        )
      }
    }
  }

  private fun parsePublicConfig(value: JSObject): PublicConfig {
    val keys = buildSet {
      val iterator = value.keys()
      while (iterator.hasNext()) add(iterator.next())
    }
    if (keys != CONFIG_FIELDS) {
      throw IllegalArgumentException("CONFIG_INPUT: invalid config fields")
    }
    return PublicConfig(
      server = requireString(value, "server"),
      username = requireString(value, "username"),
      password = requireString(value, "password"),
      topic = requireString(value, "topic"),
      themeMode = requireString(value, "theme_mode"),
      autoStart = requireBoolean(value, "auto_start"),
      autoCopyOtp = requireBoolean(value, "auto_copy_otp"),
      allowInsecureHttp = requireBoolean(value, "allow_insecure_http")
    )
  }

  private fun requireString(value: JSObject, key: String): String {
    val field = value.opt(key)
    if (field !is String) throw IllegalArgumentException("CONFIG_INPUT: invalid $key")
    return field
  }

  private fun requireBoolean(value: JSObject, key: String): Boolean {
    val field = value.opt(key)
    if (field !is Boolean) throw IllegalArgumentException("CONFIG_INPUT: invalid $key")
    return field
  }

  private fun PublicConfig.toJsObject(): JSObject = JSObject().apply {
    put("server", server)
    put("username", username)
    put("password", password)
    put("topic", topic)
    put("theme_mode", themeMode)
    put("auto_start", autoStart)
    put("auto_copy_otp", autoCopyOtp)
    put("allow_insecure_http", allowInsecureHttp)
  }

  private fun safeConfigError(error: Exception): String {
    val message = error.message.orEmpty()
    return if (message.startsWith("CONFIG_")) message else "CONFIG_ERROR"
  }
}
