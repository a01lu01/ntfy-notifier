package app.ntfy.notifier

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.util.Log
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException

class BootReceiver : BroadcastReceiver() {
  companion object {
    private const val LOG_TAG = "NtfyBootReceiver"
    private const val ERROR_POLICY_READ = "BOOT_POLICY_READ_FAILED"
    private const val ERROR_SERVICE_START = "BOOT_SERVICE_START_FAILED"
    private const val ERROR_EXECUTOR_REJECTED = "BOOT_EXECUTOR_REJECTED"

    /** Lives for the lifetime of :subscriber and keeps boot policy reads off the receiver thread. */
    private val executor = Executors.newSingleThreadExecutor { task ->
      Thread(task, "ntfy-boot-policy").apply { isDaemon = true }
    }
  }

  override fun onReceive(context: Context, intent: Intent) {
    if (intent.action != Intent.ACTION_BOOT_COMPLETED) return

    val pendingResult = goAsync()
    val applicationContext = context.applicationContext
    try {
      executor.execute {
        try {
          val enabled = try {
            ConfigStore(applicationContext).loadSubscriberAutoStartPolicy()
          } catch (_: Exception) {
            Log.e(LOG_TAG, ERROR_POLICY_READ)
            false
          }
          if (enabled) {
            try {
              NotificationService.sendAction(
                applicationContext,
                NotificationService.ACTION_BOOT
              )
            } catch (_: RuntimeException) {
              Log.e(LOG_TAG, ERROR_SERVICE_START)
            }
          }
        } finally {
          pendingResult?.finish()
        }
      }
    } catch (_: RejectedExecutionException) {
      Log.e(LOG_TAG, ERROR_EXECUTOR_REJECTED)
      pendingResult?.finish()
    }
  }
}
