package app.ntfy.notifier

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.BitmapFactory
import android.os.Build
import android.os.IBinder
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import com.why.ntfy_notifier.MainActivity
import com.why.ntfy_notifier.R

class NotificationService : Service() {

  companion object {
    const val CHANNEL_LATEST = "ntfy-latest-v2"
    const val CHANNEL_ALERTS = "ntfy-alerts-v2"
    const val NOTIFICATION_ID_LATEST = 1001
    const val NOTIFICATION_ID_ALERT = 1002
    const val ACTION_COPY_OTP = "app.ntfy.notifier.COPY_OTP"
    const val ACTION_STOP = "app.ntfy.notifier.STOP"
    const val EXTRA_OTP = "otp"
    const val PREFS = "ntfy_notifier"
    const val PREF_AUTO_START = "auto_start"

    @Volatile
    var instance: NotificationService? = null

    private var latestTitle = "ntfy-Notifier"
    private var latestMessage = "等待推送…"
    private var latestOtp: String? = null

    fun setAutoStart(context: Context, enabled: Boolean) {
      context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        .edit()
        .putBoolean(PREF_AUTO_START, enabled)
        .apply()
    }

    fun autoStart(context: Context): Boolean {
      return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        .getBoolean(PREF_AUTO_START, false)
    }

    fun updateLatest(title: String, message: String, otp: String?) {
      latestTitle = title
      latestMessage = message
      latestOtp = otp
      instance?.refreshLatest()
    }

    fun showAlert(title: String, message: String, otp: String?) {
      instance?.postAlert(title, message, otp)
    }

    fun copyToClipboard(context: Context, text: String) {
      val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
      cm.setPrimaryClip(ClipData.newPlainText("验证码", text))
      Toast.makeText(context, "验证码已复制", Toast.LENGTH_SHORT).show()
    }
  }

  override fun onCreate() {
    super.onCreate()
    instance = this
    // 清掉旧版本残留通知，避免系统继续展示旧图标
    getSystemService(NotificationManager::class.java).cancelAll()
    createChannels()
    refreshLatest()
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    when (intent?.action) {
      ACTION_COPY_OTP -> {
        val otp = intent.getStringExtra(EXTRA_OTP)
        if (!otp.isNullOrEmpty()) {
          copyToClipboard(this, otp)
        }
      }
      ACTION_STOP -> stopSelf()
      else -> refreshLatest()
    }
    return START_STICKY
  }

  override fun onDestroy() {
    if (instance === this) {
      instance = null
    }
    super.onDestroy()
  }

  override fun onBind(intent: Intent?): IBinder? = null

  private fun createChannels() {
    val nm = getSystemService(NotificationManager::class.java)
    val latest = NotificationChannel(CHANNEL_LATEST, "最新推送", NotificationManager.IMPORTANCE_LOW)
    latest.description = "常驻显示最新一条推送"
    nm.createNotificationChannel(latest)
    val alerts = NotificationChannel(CHANNEL_ALERTS, "推送提醒", NotificationManager.IMPORTANCE_HIGH)
    alerts.description = "每条新推送的提醒"
    nm.createNotificationChannel(alerts)
  }

  private fun refreshLatest() {
    val builder = NotificationCompat.Builder(this, CHANNEL_LATEST)
      .setSmallIcon(R.drawable.ic_stat_ntfy)
      .setLargeIcon(BitmapFactory.decodeResource(resources, R.drawable.ic_notification_large))
      .setContentTitle("ntfy-Notifier 运行中")
      .setContentText(
        if (latestTitle == "ntfy-Notifier" && latestMessage == "等待推送…") {
          "正在接收推送，点击查看最新消息"
        } else {
          "最新：$latestMessage"
        }
      )
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setCategory(NotificationCompat.CATEGORY_SERVICE)
      .setContentIntent(mainActivityIntent())
    latestOtp?.takeIf { it.isNotEmpty() }?.let { otp ->
      builder.addAction(0, "复制验证码", copyOtpIntent(otp))
    }
    val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
    } else {
      0
    }
    ServiceCompat.startForeground(this, NOTIFICATION_ID_LATEST, builder.build(), type)
  }

  private fun postAlert(title: String, message: String, otp: String?) {
    val style = NotificationCompat.MessagingStyle("ntfy")
    style.addMessage(message, System.currentTimeMillis(), title)
    val builder = NotificationCompat.Builder(this, CHANNEL_ALERTS)
      .setSmallIcon(R.drawable.ic_stat_ntfy)
      .setLargeIcon(BitmapFactory.decodeResource(resources, R.drawable.ic_notification_large))
      .setContentTitle(title)
      .setContentText(message)
      .setAutoCancel(true)
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setContentIntent(mainActivityIntent())
      .setStyle(style)
    otp?.takeIf { it.isNotEmpty() }?.let { code ->
      builder.addAction(0, "复制验证码", copyOtpIntent(code))
    }
    getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID_ALERT, builder.build())
  }

  private fun mainActivityIntent(): PendingIntent {
    val intent = Intent(this, MainActivity::class.java).apply {
      flags = Intent.FLAG_ACTIVITY_SINGLE_TOP
    }
    return PendingIntent.getActivity(
      this,
      0,
      intent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    )
  }

  private fun copyOtpIntent(otp: String): PendingIntent {
    val intent = Intent(this, NotificationService::class.java).apply {
      action = ACTION_COPY_OTP
      putExtra(EXTRA_OTP, otp)
    }
    return PendingIntent.getService(
      this,
      otp.hashCode(),
      intent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    )
  }
}
