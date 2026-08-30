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
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.widget.Toast
import androidx.annotation.RequiresApi
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.why.ntfy_notifier.MainActivity
import com.why.ntfy_notifier.R
import java.util.concurrent.RejectedExecutionException
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

class NotificationService : Service(), SubscriberEventSink {

  companion object {
    const val CHANNEL_LATEST = "ntfy-latest-v2"
    const val CHANNEL_ALERTS = "ntfy-alerts-v2"
    const val NOTIFICATION_ID_LATEST = 1001
    const val ACTION_START = "app.ntfy.notifier.START"
    const val ACTION_RECONFIGURE = "app.ntfy.notifier.RECONFIGURE"
    const val ACTION_COPY_OTP = "app.ntfy.notifier.COPY_OTP"
    const val ACTION_STOP = "app.ntfy.notifier.STOP"
    const val EXTRA_OTP = "otp"
    const val PREFS = "ntfy_notifier"
    const val PREF_AUTO_START = "auto_start"

    internal const val STATE_CONNECTING = "connecting"
    internal const val STATE_CONNECTED = "connected"
    internal const val STATE_RETRYING = "retrying"
    internal const val STATE_CONFIGURATION_ERROR = "configuration_error"
    internal const val STATE_STOPPED = "stopped"

    private val ALLOWED_STATES = setOf(
      STATE_CONNECTING,
      STATE_CONNECTED,
      STATE_RETRYING,
      STATE_CONFIGURATION_ERROR,
      STATE_STOPPED
    )

    fun setAutoStart(context: Context, enabled: Boolean): Boolean {
      return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        .edit()
        .putBoolean(PREF_AUTO_START, enabled)
        .commit()
    }

    fun autoStart(context: Context): Boolean {
      return context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        .getBoolean(PREF_AUTO_START, false)
    }

    internal fun actionIntent(context: Context, action: String): Intent {
      return Intent(context, NotificationService::class.java).setAction(action)
    }

    fun sendAction(context: Context, action: String) {
      ContextCompat.startForegroundService(context, actionIntent(context, action))
    }
  }

  private val mainHandler = Handler(Looper.getMainLooper())
  private val subscriptionExecutor = SubscriberProcessControl.executor
  private val requestGeneration = AtomicLong(0)
  private val sessions = SubscriberProcessSession.tracker
  private val currentServiceSession = AtomicLong(0)
  private val callbacksEnabled = AtomicBoolean(false)
  private val nativeActive = AtomicBoolean(false)
  private val nextAlertId = AtomicInteger(2000)

  @Volatile
  private var destroyed = false

  @Volatile
  private var currentState = STATE_CONNECTING

  @Volatile
  private var currentAutoCopyOtp = false

  @Volatile
  private var ownerLease = 0L

  // Accessed only by subscriptionExecutor. It intentionally never crosses an Intent or log call.
  private var currentConnection: NativeSubscriptionConfig? = null
  private val nativeCallback = SubscriberProcessControl.callback
  private lateinit var configStore: ConfigStore

  private var latestMessage: String? = null
  private var latestOtp: String? = null

  override fun onCreate() {
    super.onCreate()
    ownerLease = SubscriberProcessControl.claim(this)
    configStore = ConfigStore(applicationContext)
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      createChannels()
    }
    // Foreground promotion cannot wait for disk or Android Keystore operations.
    refreshForeground(STATE_CONNECTING)
  }

  override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
    when (intent?.action) {
      ACTION_COPY_OTP -> {
        intent.getStringExtra(EXTRA_OTP)
          ?.takeIf { it.isNotBlank() && it.length <= 128 }
          ?.let(::copyOtpToClipboard)
        requestSubscription()
      }
      ACTION_STOP -> {
        requestStop(startId)
        return START_NOT_STICKY
      }
      ACTION_START, ACTION_RECONFIGURE, null -> requestSubscription()
      else -> requestSubscription()
    }
    return START_STICKY
  }

  override fun onDestroy() {
    destroyed = true
    requestGeneration.incrementAndGet()
    callbacksEnabled.set(false)
    nativeActive.set(false)
    if (SubscriberProcessControl.release(ownerLease)) {
      try {
        subscriptionExecutor.execute {
          currentConnection = null
          // A replacement may claim ownership and enqueue its start between release and cleanup.
          // In that ordering the old instance must not stop the replacement's native engine.
          if (SubscriberProcessControl.isUnowned()) {
            stopNativeSafely()
          }
        }
      } catch (_: RejectedExecutionException) {
        // The subscriber process is already shutting down.
      }
    }
    super.onDestroy()
  }

  override fun onBind(intent: Intent?): IBinder? = null

  override fun onSubscriberState(session: Long, state: String) {
    if (state !in ALLOWED_STATES || !acceptsStateCallback(session)) return
    val terminal = state == STATE_CONFIGURATION_ERROR || state == STATE_STOPPED
    if (terminal) {
      nativeActive.set(false)
      callbacksEnabled.set(false)
    }
    mainHandler.post {
      if (
        destroyed ||
        !SubscriberProcessControl.isOwner(ownerLease) ||
        currentServiceSession.get() != session ||
        !sessions.isCurrent(session)
      ) {
        return@post
      }
      if (!terminal && !callbacksEnabled.get()) return@post
      currentState = state
      refreshForeground(state)
    }
  }

  override fun onSubscriberMessage(
    session: Long,
    title: String,
    message: String,
    otp: String?
  ) {
    if (!acceptsMessageCallback(session)) return
    val safeTitle = title.take(128)
    val safeMessage = message.take(2048)
    val safeOtp = otp?.takeIf { it.isNotBlank() && it.length <= 128 }
    mainHandler.post {
      if (!acceptsMessageCallback(session)) return@post
      latestMessage = safeMessage
      latestOtp = safeOtp
      refreshForeground(currentState)
      postAlert(safeTitle, safeMessage, safeOtp)
      if (currentAutoCopyOtp && safeOtp != null) {
        copyOtpToClipboard(safeOtp)
      }
    }
  }

  private fun requestSubscription() {
    val request = requestGeneration.incrementAndGet()
    try {
      subscriptionExecutor.execute {
        val config = try {
          configStore.loadSubscriberConfig()
        } catch (_: Exception) {
          failConfiguration(request)
          return@execute
        }
        if (!requestIsCurrent(request)) return@execute

        currentAutoCopyOtp = config.autoCopyOtp
        val connection = config.toNativeSubscriptionConfig()
        if (connection.server.isBlank() || connection.topic.isBlank()) {
          failConfiguration(request)
          return@execute
        }

        if (nativeActive.get() && currentConnection == connection) {
          return@execute
        }

        val reconfigure = nativeActive.get()
        val session = issueSession()
        callbacksEnabled.set(true)
        postState(session, STATE_CONNECTING)

        val started = if (!NativeSubscriber.isAvailable()) {
          false
        } else {
          try {
            if (reconfigure) {
              NativeSubscriber.nativeReconfigure(
                dataDir.absolutePath,
                session,
                connection.server,
                connection.username,
                connection.password,
                connection.topic,
                connection.allowInsecureHttp,
                nativeCallback
              )
            } else {
              NativeSubscriber.nativeStart(
                dataDir.absolutePath,
                session,
                connection.server,
                connection.username,
                connection.password,
                connection.topic,
                connection.allowInsecureHttp,
                nativeCallback
              )
            }
          } catch (_: LinkageError) {
            false
          } catch (_: RuntimeException) {
            false
          }
        }

        if (!requestIsCurrent(request)) {
          callbacksEnabled.set(false)
          nativeActive.set(false)
          currentConnection = null
          if (started) stopNativeSafely()
          return@execute
        }

        if (started) {
          currentConnection = connection
          nativeActive.set(true)
        } else {
          callbacksEnabled.set(false)
          nativeActive.set(false)
          currentConnection = null
          stopNativeSafely()
          postState(session, STATE_CONFIGURATION_ERROR, requireCallbacks = false)
        }
      }
    } catch (_: RejectedExecutionException) {
      // Service teardown won the race and already invalidated this request.
    }
  }

  private fun failConfiguration(request: Long) {
    if (!requestIsCurrent(request)) return
    val session = issueSession()
    callbacksEnabled.set(false)
    nativeActive.set(false)
    currentConnection = null
    currentAutoCopyOtp = false
    stopNativeSafely()
    postState(session, STATE_CONFIGURATION_ERROR, requireCallbacks = false)
  }

  private fun requestStop(startId: Int) {
    if (destroyed || !SubscriberProcessControl.isOwner(ownerLease)) return
    val request = requestGeneration.incrementAndGet()
    val session = issueSession()
    callbacksEnabled.set(false)
    nativeActive.set(false)
    try {
      subscriptionExecutor.execute {
        currentConnection = null
        stopNativeSafely()
        mainHandler.post {
          if (destroyed || !requestIsCurrent(request) || !sessions.isCurrent(session)) {
            return@post
          }
          currentState = STATE_STOPPED
          refreshForeground(STATE_STOPPED)
          ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
          stopSelfResult(startId)
        }
      }
    } catch (_: RejectedExecutionException) {
      stopSelfResult(startId)
    }
  }

  private fun stopNativeSafely() {
    if (!NativeSubscriber.isAvailable()) return
    try {
      NativeSubscriber.nativeStop()
    } catch (_: LinkageError) {
      // Loader/ABI errors are represented by configuration_error while the service is alive.
    } catch (_: RuntimeException) {
      // A control error must not crash the sticky foreground-service process.
    }
  }

  private fun issueSession(): Long {
    return sessions.advance().also(currentServiceSession::set)
  }

  private fun acceptsStateCallback(session: Long): Boolean {
    return !destroyed &&
      SubscriberProcessControl.isOwner(ownerLease) &&
      callbacksEnabled.get() &&
      currentServiceSession.get() == session &&
      sessions.isCurrent(session)
  }

  private fun acceptsMessageCallback(session: Long): Boolean {
    // The database transaction may have committed immediately before a reconfigure. That old
    // session's notification must still complete, including after callback routing changes owner.
    return !destroyed && sessions.hasIssued(session)
  }

  private fun requestIsCurrent(request: Long): Boolean {
    return !destroyed &&
      SubscriberProcessControl.isOwner(ownerLease) &&
      requestGeneration.get() == request
  }

  private fun postState(
    session: Long,
    state: String,
    requireCallbacks: Boolean = true
  ) {
    mainHandler.post {
      if (
        destroyed ||
        !SubscriberProcessControl.isOwner(ownerLease) ||
        currentServiceSession.get() != session ||
        !sessions.isCurrent(session)
      ) {
        return@post
      }
      if (requireCallbacks && !callbacksEnabled.get()) return@post
      currentState = state
      refreshForeground(state)
    }
  }

  @RequiresApi(Build.VERSION_CODES.O)
  private fun createChannels() {
    val manager = getSystemService(NotificationManager::class.java)
    val latest = NotificationChannel(
      CHANNEL_LATEST,
      "ntfy 订阅状态",
      NotificationManager.IMPORTANCE_LOW
    ).apply {
      description = "常驻显示 ntfy 连接状态和最新推送"
    }
    manager.createNotificationChannel(latest)
    val alerts = NotificationChannel(
      CHANNEL_ALERTS,
      "ntfy 推送提醒",
      NotificationManager.IMPORTANCE_HIGH
    ).apply {
      description = "每条新 ntfy 消息的提醒"
    }
    manager.createNotificationChannel(alerts)
  }

  private fun refreshForeground(state: String) {
    val status = when (state) {
      STATE_CONNECTED -> "正在接收推送"
      STATE_RETRYING -> "连接中断，正在重试…"
      STATE_CONFIGURATION_ERROR -> "配置不可用，请打开应用检查设置"
      STATE_STOPPED -> "订阅已停止"
      else -> "正在连接 ntfy…"
    }
    val content = if (state == STATE_CONNECTED && latestMessage != null) {
      "最新：${latestMessage.orEmpty().take(256)}"
    } else {
      status
    }
    val title = when (state) {
      STATE_CONFIGURATION_ERROR -> "ntfy-Notifier 配置错误"
      STATE_RETRYING -> "ntfy-Notifier 重连中"
      STATE_STOPPED -> "ntfy-Notifier 已停止"
      else -> "ntfy-Notifier 运行中"
    }
    val builder = NotificationCompat.Builder(this, CHANNEL_LATEST)
      .setSmallIcon(R.drawable.ic_stat_ntfy)
      .setContentTitle(title)
      .setContentText(content)
      .setSubText(status)
      .setStyle(NotificationCompat.BigTextStyle().bigText(content))
      .setOngoing(true)
      .setOnlyAlertOnce(true)
      .setSilent(true)
      .setCategory(NotificationCompat.CATEGORY_SERVICE)
      .setContentIntent(mainActivityIntent())
      .addAction(0, "停止", stopIntent())
    latestOtp?.let { otp ->
      builder.addAction(0, "复制验证码", copyOtpIntent(otp, NOTIFICATION_ID_LATEST))
    }
    val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
      ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
    } else {
      0
    }
    ServiceCompat.startForeground(this, NOTIFICATION_ID_LATEST, builder.build(), type)
  }

  private fun postAlert(title: String, message: String, otp: String?) {
    val alertId = nextAlertId.updateAndGet { current ->
      if (current == Int.MAX_VALUE) 2000 else current + 1
    }
    val user = Person.Builder().setName("ntfy").build()
    val sender = Person.Builder().setName(title).build()
    val style = NotificationCompat.MessagingStyle(user)
      .addMessage(
        NotificationCompat.MessagingStyle.Message(
          message,
          System.currentTimeMillis(),
          sender
        )
      )
    val builder = NotificationCompat.Builder(this, CHANNEL_ALERTS)
      .setSmallIcon(R.drawable.ic_stat_ntfy)
      .setContentTitle(title)
      .setContentText(message)
      .setAutoCancel(true)
      .setCategory(NotificationCompat.CATEGORY_MESSAGE)
      .setContentIntent(mainActivityIntent())
      .setStyle(style)
    otp?.let { code ->
      builder.addAction(0, "复制验证码", copyOtpIntent(code, alertId))
    }
    try {
      getSystemService(NotificationManager::class.java).notify(alertId, builder.build())
    } catch (_: SecurityException) {
      // Android 13+ may deny POST_NOTIFICATIONS; subscription persistence remains unaffected.
    }
  }

  private fun mainActivityIntent(): PendingIntent {
    val intent = Intent(this, MainActivity::class.java).apply {
      flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
    }
    return PendingIntent.getActivity(
      this,
      0,
      intent,
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    )
  }

  private fun stopIntent(): PendingIntent {
    return foregroundServicePendingIntent(100, actionIntent(this, ACTION_STOP))
  }

  private fun copyOtpIntent(otp: String, requestCode: Int): PendingIntent {
    val intent = actionIntent(this, ACTION_COPY_OTP).putExtra(EXTRA_OTP, otp)
    return foregroundServicePendingIntent(requestCode, intent)
  }

  private fun foregroundServicePendingIntent(requestCode: Int, intent: Intent): PendingIntent {
    val pendingFlags = PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
    return if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
      PendingIntent.getForegroundService(this, requestCode, intent, pendingFlags)
    } else {
      PendingIntent.getService(this, requestCode, intent, pendingFlags)
    }
  }

  private fun copyOtpToClipboard(otp: String) {
    try {
      val clipboard = getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
      clipboard.setPrimaryClip(ClipData.newPlainText("验证码", otp))
      Toast.makeText(this, "验证码已复制", Toast.LENGTH_SHORT).show()
    } catch (_: RuntimeException) {
      // Do not include the OTP or exception text in logs. Background clipboard policies vary by
      // Android version and device vendor, but a denial must not kill the subscriber process.
      Toast.makeText(this, "无法复制验证码，请打开应用后重试", Toast.LENGTH_LONG).show()
    }
  }
}
