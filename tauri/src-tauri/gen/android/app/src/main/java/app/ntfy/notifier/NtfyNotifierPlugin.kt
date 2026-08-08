package app.ntfy.notifier

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.plugin.Invoke
import app.tauri.plugin.Plugin

class NtfyNotifierPlugin(private val activity: Activity) : Plugin(activity) {

  @Command
  fun startService(invoke: Invoke) {
    requestNotificationPermission()
    val context = activity.applicationContext
    val intent = Intent(context, NotificationService::class.java)
    ContextCompat.startForegroundService(context, intent)
    invoke.resolve()
  }

  @Command
  fun setAutoStart(invoke: Invoke) {
    val args = invoke.getArgs()
    NotificationService.setAutoStart(activity.applicationContext, args.getBoolean("enabled", false))
    invoke.resolve()
  }

  @Command
  fun updateNotifications(invoke: Invoke) {
    val args = invoke.getArgs()
    val title = args.getString("title")
    val message = args.getString("message")
    val otp = args.getString("otp", null)
    NotificationService.updateLatest(title, message, otp)
    NotificationService.showAlert(title, message, otp)
    invoke.resolve()
  }

  @Command
  fun copyToClipboard(invoke: Invoke) {
    val args = invoke.getArgs()
    NotificationService.copyToClipboard(activity.applicationContext, args.getString("text"))
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
}
