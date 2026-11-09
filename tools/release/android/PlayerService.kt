// Copied into the generated Android project by tools/release/android-patch.py,
// next to MainActivity.kt (the package line is rewritten to match).
//
// Keeps the app's built-in player alive while the app is in the background:
// without a foreground service Android freezes the process within seconds of
// leaving the app, and the node's stream to this phone stops. Started by the
// Rust side (src/android.rs) once the app reaches a node or opens its offline page; it
// must start while the app is on screen (Android 12+ refuses otherwise).
//
// Its notification is the media notification while the app's media session (MediaControl.kt) is
// active: title, artist, cover and the buttons of the zone the app controls, also on the lock
// screen and in the quick-settings panel; otherwise it says what the service is for.
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
            // (debug: what the watch saw, for tools/android-background-probe.py part E2)
            android.util.Log.d("pubky-music", "player service: watch music=${am.isMusicActive} seen=$seen importance=${me.importance} idle=${idleMs / 1000} s")
            if (idleMs >= IDLE_MS) {
                // Only if no start came since: a start still on its way keeps the service
                if (stopSelfResult(lastStart)) {
                    android.util.Log.i("pubky-music", "player service: nothing played for ${IDLE_MS / 60000} min, stopped")
                    return
                }
                idleMs = 0L
            }
            tick.postDelayed(this, WATCH_MS)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        lastStart = startId
        text = intent?.getStringExtra(EXTRA_TEXT) ?: "This phone is a zone"
        zone = intent?.getBooleanExtra(EXTRA_ZONE, true) ?: true
        val n = build(this, true)
        try {
            if (Build.VERSION.SDK_INT >= 29) {
                startForeground(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK)
            } else {
                startForeground(ID, n)
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
        running = true
        idleMs = 0L
        tick.removeCallbacks(watch)
        if (!zone) tick.postDelayed(watch, WATCH_MS)
        android.util.Log.d("pubky-music", "player service: start ${if (zone) "zone" else "app"} mode")
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
        running = false
        tick.removeCallbacks(watch)
        release()
        super.onDestroy()
        // The media session outlives the service (the app on screen): its notification stays
        refresh(applicationContext)
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
        /// The service's foreground notification (the media one while the zone plays).
        const val ID = 1
        /// The media notification while the session doesn't play (paused, stopped) or the service
        /// doesn't run: Android 14+ stops treating a media service as foreground once the session
        /// its notification shows is not playing, and the zone's service must stay (the node may
        /// start music on this phone at any time). So the service's own notification shows the
        /// session only while it plays.
        const val MEDIA_ID = 2
        @Volatile private var running = false
        @Volatile private var text = "This phone is a zone"

        private fun builder(c: Context, ongoing: Boolean): Notification.Builder {
            MediaControl.channel(c)
            val launch = c.packageManager.getLaunchIntentForPackage(c.packageName)
            val open = PendingIntent.getActivity(c, 0, launch, PendingIntent.FLAG_IMMUTABLE)
            return Notification.Builder(c, CHANNEL)
                // A monochrome status-bar icon: the launcher icon is an adaptive icon, which some
                // phones refuse as a small icon ("Bad notification for startForeground", a crash).
                .setSmallIcon(R.drawable.ic_stat_pubky_music)
                .setContentIntent(open)
                .setOnlyAlertOnce(true)
                .setOngoing(ongoing)
        }

        /// The service's notification as it should be now: the media one while the session plays,
        /// else what the service is for.
        fun build(c: Context, service: Boolean): Notification {
            val b = builder(c, service || running)
            if (!(MediaControl.playing() && MediaControl.styled(c, b))) {
                b.setContentTitle("Odeion").setContentText(text)
            }
            return b.build()
        }

        /// The session changed (MediaControl.kt): the notifications follow. The service's shows the
        /// session while it plays; otherwise (paused, stopped, no service) the media notification
        /// is a separate one, gone with the session.
        fun refresh(c: Context) {
            val nm = c.getSystemService(NotificationManager::class.java) ?: return
            try {
                if (running) nm.notify(ID, build(c, false)) else nm.cancel(ID)
                val separate = MediaControl.active() && !(running && MediaControl.playing())
                if (separate) {
                    val b = builder(c, false)
                    if (MediaControl.styled(c, b)) nm.notify(MEDIA_ID, b.build()) else nm.cancel(MEDIA_ID)
                } else {
                    nm.cancel(MEDIA_ID)
                }
            } catch (e: Exception) {
                android.util.Log.w("pubky-music", "player notification: $e")
            }
        }

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
