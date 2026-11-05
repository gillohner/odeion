// Copied into the generated Android project by tools/release/android-patch.py,
// next to MainActivity.kt (the package line is rewritten to match).
//
// Keeps the app's built-in player alive while the app is in the background:
// without a foreground service Android freezes the process within seconds of
// leaving the app, and the node's stream to this phone stops. Started by the
// Rust side (src/android.rs) once the app reaches a node or opens its offline page; it
// must start while the app is on screen (Android 12+ refuses otherwise).
package to.synonym.pubkymusic

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager

class PlayerService : Service() {
    private var wake: PowerManager.WakeLock? = null
    private var wifi: WifiManager.WifiLock? = null
    // Both modes keep the CPU and Wi-Fi awake: with the screen off, the next song's first bytes
    // (through the tunnel too) must arrive after the last one ends. The phone as a zone of the
    // node runs while connected: the node may start music on it at any time. The app playing the
    // music itself (through the tunnel, the offline app) ends the service, and its locks, after a
    // while with nothing playing while the app is out of sight; it starts again when the app
    // comes back. (isMusicActive counts any app's music: another app playing keeps it running.)
    private var zone = true
    private var idleMs = 0L
    private var lastStart = 0
    private val tick = Handler(Looper.getMainLooper())
    private val watch = object : Runnable {
        override fun run() {
            if (zone) return
            val am = getSystemService(android.media.AudioManager::class.java)
            val me = android.app.ActivityManager.RunningAppProcessInfo()
            android.app.ActivityManager.getMyMemoryState(me)
            val seen = me.importance == android.app.ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND
            // Counted only while out of sight: time on screen with nothing playing (browsing,
            // paused) must not end the service just after the music starts and the app is left
            idleMs = if (am.isMusicActive || seen) 0L else idleMs + WATCH_MS
            if (idleMs >= IDLE_MS) {
                android.util.Log.i("pubky-music", "player service: nothing played for ${IDLE_MS / 60000} min, stopping")
                // Only if no start came since: a start still on its way keeps the service
                stopSelfResult(lastStart)
                return
            }
            tick.postDelayed(this, WATCH_MS)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        lastStart = startId
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(CHANNEL, "Player", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shown while the app plays, or while this phone is a zone of your node"
            }
        )
        val launch = packageManager.getLaunchIntentForPackage(packageName)
        val open = PendingIntent.getActivity(this, 0, launch, PendingIntent.FLAG_IMMUTABLE)
        val text = intent?.getStringExtra(EXTRA_TEXT) ?: "This phone is a zone"
        zone = intent?.getBooleanExtra(EXTRA_ZONE, true) ?: true
        val n = Notification.Builder(this, CHANNEL)
            // A monochrome status-bar icon: the launcher icon is an adaptive icon, which some
            // phones refuse as a small icon ("Bad notification for startForeground", a crash).
            .setSmallIcon(R.drawable.ic_stat_pubky_music)
            .setContentTitle("Odeion")
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .build()
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(1, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
            } else {
                startForeground(1, n)
            }
        } catch (e: Exception) {
            // Refused (a missing permission, a background start, a system that wants more):
            // no background playback, but the app carries on. A start refused because the app was
            // in the background is not a crash: it is tried again when the app comes back.
            if (backgroundRefusal(e)) {
                android.util.Log.w("pubky-music", "player service: a start from the background was refused; again when the app is back")
            } else {
                CrashReport.write(this, "The player service could not start", android.util.Log.getStackTraceString(e))
            }
            stopSelf()
            return START_NOT_STICKY
        }
        hold()
        idleMs = 0L
        tick.removeCallbacks(watch)
        if (!zone) tick.postDelayed(watch, WATCH_MS)
        return START_NOT_STICKY
    }

    /// The stream arrives over Wi-Fi with the screen off: keep the CPU and the radio awake.
    private fun hold() {
        if (wake == null) {
            val pm = getSystemService(PowerManager::class.java)
            wake = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "pubky-music:player").apply {
                setReferenceCounted(false)
                acquire()
            }
        }
        if (wifi == null) {
            val wm = applicationContext.getSystemService(WifiManager::class.java)
            @Suppress("DEPRECATION")
            val mode = if (Build.VERSION.SDK_INT >= 29) WifiManager.WIFI_MODE_FULL_LOW_LATENCY
                       else WifiManager.WIFI_MODE_FULL_HIGH_PERF
            wifi = wm.createWifiLock(mode, "pubky-music:player").apply {
                setReferenceCounted(false)
                acquire()
            }
        }
    }

    /// Swiping the app away ends the zone too.
    override fun onTaskRemoved(rootIntent: Intent?) {
        stopSelf()
    }

    private fun release() {
        wake?.release()
        wifi?.release()
        wake = null
        wifi = null
    }

    override fun onDestroy() {
        tick.removeCallbacks(watch)
        release()
        super.onDestroy()
    }

    companion object {
        const val CHANNEL = "player"
        const val EXTRA_TEXT = "text"
        const val EXTRA_ZONE = "zone"
        const val WATCH_MS = 60_000L
        const val IDLE_MS = 5 * 60_000L

        /// Android 12+ refuses a foreground service started from the background
        /// (ForegroundServiceStartNotAllowedException, by name: the class is new in API 31).
        fun backgroundRefusal(e: Exception): Boolean =
            Build.VERSION.SDK_INT >= 31 && e.javaClass.name == "android.app.ForegroundServiceStartNotAllowedException"
        private const val NOTIFY = "android.permission.POST_NOTIFICATIONS"

        /// Whether Android took the start (false: refused, for example from the background).
        @JvmStatic
        fun start(activity: Activity, text: String, zone: Boolean): Boolean {
            // Android 13+: without this the service still runs, but its notification is hidden.
            // Asked once: someone who said no is not asked again on every launch.
            val prefs = activity.getSharedPreferences("pubky-music", Context.MODE_PRIVATE)
            if (Build.VERSION.SDK_INT >= 33 &&
                activity.checkSelfPermission(NOTIFY) != PackageManager.PERMISSION_GRANTED &&
                !prefs.getBoolean("asked-notifications", false)
            ) {
                prefs.edit().putBoolean("asked-notifications", true).apply()
                activity.requestPermissions(arrayOf(NOTIFY), 7)
            }
            val i = Intent(activity, PlayerService::class.java).putExtra(EXTRA_TEXT, text).putExtra(EXTRA_ZONE, zone)
            return try {
                activity.startForegroundService(i)
                true
            } catch (e: Exception) {
                // From the background (Android 12+): not a crash; the app starts it again when it is back
                if (backgroundRefusal(e)) {
                    android.util.Log.w("pubky-music", "player service: a start from the background was refused; again when the app is back")
                } else {
                    CrashReport.write(activity, "The player service was refused", android.util.Log.getStackTraceString(e))
                }
                false
            }
        }

        @JvmStatic
        fun stop(context: Context) {
            context.stopService(Intent(context, PlayerService::class.java))
        }
    }
}
