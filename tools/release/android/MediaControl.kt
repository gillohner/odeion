// Copied into the generated Android project by tools/release/android-patch.py,
// next to MainActivity.kt (the package line is rewritten to match).
//
// The system's media controls for the zone the app's page controls (src/android.rs and
// src/media_live.rs feed it; src/media.rs decides): one framework MediaSession (android.media.session,
// no androidx) with metadata, cover and PlaybackState, so the media notification (the player
// service's, PlayerService.kt), the lock screen, the quick-settings panel and headset or Bluetooth
// buttons show and drive it. Its volume is the zone's own for a network zone (a remote
// VolumeProvider: the volume keys and Android's panel set the zone's real volume) and Android's media
// volume for the phone's own players (a local session: the keys move STREAM_MUSIC themselves).
// While the app is on screen the activity's media controller is this session, so the keys reach it
// on every Android version; in the background Android sends them to a remote session from
// Android 16 on (12L to 15 only where the phone enables it), to a local one always.
//
// Everything the session's buttons, the page and Android say goes to the Rust side through one queue
// (next()): the page's two calls (window.OdeionAndroid, over androidx.webkit's message listener: the
// main frame only, its origin handed over and checked again in Rust), audio focus, "becoming noisy",
// media volume and route changes. After a USB DAC is unplugged, Android's media volume on the new
// route (the speaker) is lowered to a quarter if it is higher: never louder.
package to.synonym.pubkymusic

import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.drawable.Icon
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaMetadata
import android.media.VolumeProvider
import android.media.session.MediaController
import android.media.session.MediaSession
import android.media.session.PlaybackState
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.webkit.WebView
import androidx.webkit.WebViewCompat
import androidx.webkit.WebViewFeature
import java.lang.ref.WeakReference
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import org.json.JSONObject

class MediaControl private constructor() {
    companion object {
        private const val TAG = "pubky-music"
        /// The page's message port: window.OdeionAndroid (the app's init script) posts through it.
        private const val PORT = "OdeionAndroidPort"
        /// What the page may send at most.
        private const val PAGE_MAX = 16 * 1024
        /// The media volume a speaker gets at most after a USB DAC goes (a quarter).
        private const val SAFE_PERCENT = 25

        private val events = LinkedBlockingQueue<String>(256)
        private val main = Handler(Looper.getMainLooper())
        private val media: AudioAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()

        @Volatile private var context: Context? = null
        @Volatile private var activity: WeakReference<Activity>? = null

        // Main thread only from here
        private var session: MediaSession? = null
        private var provider: VolumeProvider? = null
        private var providerKind = ""
        private var shown: JSONObject? = null
        private var artUrl = ""
        private var art: Bitmap? = null
        private var focusRequest: AudioFocusRequest? = null
        private var focusDucks = true
        /// What the notification shows, and what it showed last.
        private var notifyKey = ""
        private var notified = ""
        private val focusListener = AudioManager.OnAudioFocusChangeListener { change ->
            emit(JSONObject().put("k", "focus").put("change", change))
        }

        private fun emit(o: JSONObject) {
            // A full queue (the Rust side stopped reading): dropped, never blocking Android's thread
            if (!events.offer(o.toString())) android.util.Log.w(TAG, "media controls: an event was dropped")
        }

        private fun command(a: String) = emit(JSONObject().put("k", "cmd").put("a", a))

        private fun audio(): AudioManager? = context?.getSystemService(AudioManager::class.java)

        /// The next event for the Rust side ("" after `timeoutMs`). Blocks: the Rust side's own thread.
        @JvmStatic
        fun next(timeoutMs: Long): String = events.poll(timeoutMs, TimeUnit.MILLISECONDS) ?: ""

        /// Once, on the UI thread, before the node's page loads: the page's port, Android's
        /// broadcasts, and the activity (its media controller is the session while it is on screen).
        @JvmStatic
        fun attach(a: Activity, webview: WebView) {
            activity = WeakReference(a)
            if (WebViewFeature.isFeatureSupported(WebViewFeature.WEB_MESSAGE_LISTENER)) {
                // Every origin may hold the port; only the main frame's messages are passed on, with
                // its origin, and the Rust side takes only the node's (or the offline page's)
                android.util.Log.i(TAG, "media controls: the page's message port is set up")
                WebViewCompat.addWebMessageListener(webview, PORT, setOf("*")) { _, message, sourceOrigin, isMainFrame, _ ->
                    val data = message.data
                    if (isMainFrame && data != null && data.length <= PAGE_MAX) {
                        emit(JSONObject().put("k", "page").put("origin", sourceOrigin.toString()).put("data", data))
                    }
                }
            } else {
                android.util.Log.w(TAG, "media controls: this WebView has no message listener; the page can't name its zone")
            }
            if (context != null) return
            val c = a.applicationContext
            context = c
            a.application.registerActivityLifecycleCallbacks(object : android.app.Application.ActivityLifecycleCallbacks {
                override fun onActivityResumed(x: Activity) {
                    activity = WeakReference(x)
                    session?.let { x.mediaController = MediaController(x, it.sessionToken) }
                }
                override fun onActivityCreated(x: Activity, b: android.os.Bundle?) {}
                override fun onActivityStarted(x: Activity) {}
                override fun onActivityPaused(x: Activity) {}
                override fun onActivityStopped(x: Activity) {}
                override fun onActivitySaveInstanceState(x: Activity, b: android.os.Bundle) {}
                override fun onActivityDestroyed(x: Activity) {}
            })
            val receiver = object : BroadcastReceiver() {
                override fun onReceive(ctx: Context, intent: Intent) {
                    when (intent.action) {
                        // The volume keys, Android's panel, another app: media's only
                        "android.media.VOLUME_CHANGED_ACTION" ->
                            if (intent.getIntExtra("android.media.EXTRA_VOLUME_STREAM_TYPE", -1) == AudioManager.STREAM_MUSIC) {
                                emit(JSONObject().put("k", "media_volume"))
                            }
                        // Headphones out, a Bluetooth sink gone: the phone's players pause. Taking the
                        // DAC for Exclusive USB causes one too: not passed on (as in UsbAudio.kt)
                        AudioManager.ACTION_AUDIO_BECOMING_NOISY ->
                            if (!UsbExclusive.holding()) emit(JSONObject().put("k", "noisy"))
                        // The notification's buttons (below Android 13: Android 13+ draws its own from the session)
                        actionName(ctx) -> intent.getStringExtra("a")?.let { command(it) }
                    }
                }
            }
            val filter = IntentFilter("android.media.VOLUME_CHANGED_ACTION")
            filter.addAction(AudioManager.ACTION_AUDIO_BECOMING_NOISY)
            filter.addAction(actionName(c))
            // Not exported on every version (the notification's buttons drive the zone: no other app
            // may send them; below Android 13 ContextCompat guards it with a permission of the app's)
            androidx.core.content.ContextCompat.registerReceiver(
                c, receiver, filter, null, main, androidx.core.content.ContextCompat.RECEIVER_NOT_EXPORTED
            )
            // Media's route changed (headphones, Bluetooth, a DAC): its volume is another device's
            audio()?.registerAudioDeviceCallback(object : AudioDeviceCallback() {
                override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) {
                    emit(JSONObject().put("k", "media_volume"))
                }
                override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) {
                    // Not when the app took the DAC for Exclusive USB (Android lists it no more then):
                    // an unplug while the app holds it comes from UsbExclusive.kt's own detach
                    val usb = removed.any { it.isSink && (it.type == AudioDeviceInfo.TYPE_USB_DEVICE || it.type == AudioDeviceInfo.TYPE_USB_HEADSET) }
                    if (usb && !UsbExclusive.holding()) safeAfterUnplug()
                    emit(JSONObject().put("k", "media_volume"))
                }
            }, main)
        }

        private fun actionName(c: Context) = c.packageName + ".MEDIA_BUTTON"

        /// A USB DAC went (unplugged, or taken back from Exclusive USB by its unplug): once Android
        /// plays on the speaker, its media volume there is at most a quarter. Never raised.
        @JvmStatic
        fun safeAfterUnplug() {
            main.postDelayed({
                val am = audio() ?: return@postDelayed
                // Only where media plays on the phone itself now: never the DAC (Exclusive USB taking
                // or giving it back lists it again) or another USB device
                val usb = setOf(AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_HEADSET)
                val onUsb = runCatching {
                    if (Build.VERSION.SDK_INT >= 33) am.getAudioDevicesForAttributes(media).any { it.type in usb }
                    else am.getDevices(AudioManager.GET_DEVICES_OUTPUTS).any { it.type in usb }
                }.getOrDefault(true)
                if (onUsb) return@postDelayed
                runCatching {
                    val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
                    val cap = maxOf(1, max * SAFE_PERCENT / 100)
                    if (am.getStreamVolume(AudioManager.STREAM_MUSIC) > cap) {
                        am.setStreamVolume(AudioManager.STREAM_MUSIC, cap, 0)
                        android.util.Log.i(TAG, "media controls: the DAC went; media volume down to $cap of $max")
                    }
                }
                emit(JSONObject().put("k", "media_volume"))
            }, 700)
        }

        /// Android's media volume: "<index> <max> <muted 0|1>", "" when it can't be read.
        @JvmStatic
        fun streamVolume(): String {
            val am = audio() ?: return ""
            return runCatching {
                val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
                if (max <= 0) return@runCatching ""
                val v = am.getStreamVolume(AudioManager.STREAM_MUSIC)
                val muted = Build.VERSION.SDK_INT >= 23 && am.isStreamMute(AudioManager.STREAM_MUSIC)
                "$v $max ${if (muted) 1 else 0}"
            }.getOrDefault("")
        }

        /// Set Android's media volume (no system panel: the app's slider is on screen).
        @JvmStatic
        fun setStreamVolume(index: Int): Boolean {
            val am = audio() ?: return false
            return runCatching { am.setStreamVolume(AudioManager.STREAM_MUSIC, index, 0); true }.getOrDefault(false)
        }

        @JvmStatic
        fun setStreamMute(mute: Boolean): Boolean {
            val am = audio() ?: return false
            if (Build.VERSION.SDK_INT < 23) return false
            return runCatching {
                am.adjustStreamVolume(AudioManager.STREAM_MUSIC, if (mute) AudioManager.ADJUST_MUTE else AudioManager.ADJUST_UNMUTE, 0)
                true
            }.getOrDefault(false)
        }

        /// The session as the Rust side sees it now (src/media.rs `Session`, JSON).
        @JvmStatic
        fun update(json: String) {
            val s = runCatching { JSONObject(json) }.getOrNull() ?: return
            main.post { apply(s) }
        }

        /// The cover for `url` (no bytes: none).
        @JvmStatic
        fun art(url: String, bytes: ByteArray) {
            val bmp = if (bytes.isEmpty()) null else decode(bytes)
            main.post {
                artUrl = url
                art = bmp
                shown?.let { if (it.optString("art") == url) apply(it) }
            }
        }

        /// At most 512 px on its longer side: what the notification and lock screen show.
        private fun decode(bytes: ByteArray): Bitmap? = runCatching {
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
            var sample = 1
            while (maxOf(bounds.outWidth, bounds.outHeight) / sample > 1024) sample *= 2
            val b = BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
                ?: return@runCatching null
            val side = maxOf(b.width, b.height)
            if (side <= 512) b else Bitmap.createScaledBitmap(b, b.width * 512 / side, b.height * 512 / side, true)
        }.getOrNull()

        private val callback = object : MediaSession.Callback() {
            override fun onPlay() = command("play")
            override fun onPause() = command("pause")
            override fun onSkipToNext() = command("next")
            override fun onSkipToPrevious() = command("previous")
            override fun onStop() = command("stop")
            override fun onSeekTo(pos: Long) {
                emit(JSONObject().put("k", "cmd").put("a", "seek").put("ms", maxOf(0L, pos)))
            }
        }

        private fun apply(s: JSONObject) {
            val c = context ?: return
            if (!s.optBoolean("active")) {
                release(c)
                return
            }
            shown = s
            val ms = session ?: MediaSession(c, "Odeion").also {
                it.setCallback(callback, main)
                session = it
            }
            val title = s.optString("title").ifEmpty { s.optString("zone_name").ifEmpty { "Odeion" } }
            val md = MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, title)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, s.optString("artist"))
                .putString(MediaMetadata.METADATA_KEY_ALBUM, s.optString("album"))
                .putString(MediaMetadata.METADATA_KEY_DISPLAY_SUBTITLE, s.optString("zone_name"))
                .putLong(MediaMetadata.METADATA_KEY_DURATION, s.optLong("duration_ms"))
            val bmp = art
            if (bmp != null && artUrl == s.optString("art")) md.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, bmp)
            ms.setMetadata(md.build())
            val state = when (s.optString("state")) {
                "playing" -> PlaybackState.STATE_PLAYING
                "paused" -> PlaybackState.STATE_PAUSED
                else -> PlaybackState.STATE_STOPPED
            }
            var actions = PlaybackState.ACTION_PLAY or PlaybackState.ACTION_PAUSE or
                PlaybackState.ACTION_PLAY_PAUSE or PlaybackState.ACTION_STOP
            if (s.optBoolean("can_next")) actions = actions or PlaybackState.ACTION_SKIP_TO_NEXT
            if (s.optBoolean("can_prev")) actions = actions or PlaybackState.ACTION_SKIP_TO_PREVIOUS
            if (s.optBoolean("can_seek")) actions = actions or PlaybackState.ACTION_SEEK_TO
            notifyKey = listOf(title, s.optString("artist"), s.optString("zone_name"), s.optString("state"),
                s.optBoolean("can_prev"), s.optBoolean("can_next"), if (bmp != null && artUrl == s.optString("art")) artUrl else "").joinToString("\u0000")
            ms.setPlaybackState(
                PlaybackState.Builder()
                    .setActions(actions)
                    .setState(state, s.optLong("position_ms"), s.optDouble("speed", 0.0).toFloat(), SystemClock.elapsedRealtime())
                    .build()
            )
            volume(ms, s.optJSONObject("volume"))
            if (!ms.isActive) {
                ms.isActive = true
                activity?.get()?.let { it.mediaController = MediaController(it, ms.sessionToken) }
            }
            // The notification only when what it shows changed (not for every position update)
            if (notifyKey != notified) {
                notified = notifyKey
                PlayerService.refresh(c)
            }
        }

        /// The session's volume: Android's media volume, or the zone's own in steps (remote).
        private fun volume(ms: MediaSession, v: JSONObject?) {
            val kind = v?.optString("kind") ?: "local"
            when (kind) {
                "remote" -> {
                    val max = maxOf(1, v!!.optInt("max", 100))
                    val cur = v.optInt("current", 0).coerceIn(0, max)
                    val p = provider
                    if (p != null && providerKind == "remote" && p.maxVolume == max) {
                        if (p.currentVolume != cur) p.currentVolume = cur
                    } else {
                        val np = object : VolumeProvider(VOLUME_CONTROL_ABSOLUTE, max, cur) {
                            override fun onSetVolumeTo(volume: Int) {
                                currentVolume = volume.coerceIn(0, maxVolume)
                                emit(JSONObject().put("k", "cmd").put("a", "volume_to").put("v", currentVolume))
                            }
                            override fun onAdjustVolume(direction: Int) {
                                when (direction) {
                                    AudioManager.ADJUST_RAISE, AudioManager.ADJUST_LOWER -> {
                                        currentVolume = (currentVolume + direction).coerceIn(0, maxVolume)
                                        emit(JSONObject().put("k", "cmd").put("a", "volume_by").put("d", direction))
                                    }
                                    // Mute keys: the zone's mute, never its level
                                    AudioManager.ADJUST_MUTE -> emit(JSONObject().put("k", "cmd").put("a", "mute").put("m", true))
                                    AudioManager.ADJUST_UNMUTE -> emit(JSONObject().put("k", "cmd").put("a", "mute").put("m", false))
                                    AudioManager.ADJUST_TOGGLE_MUTE -> emit(JSONObject().put("k", "cmd").put("a", "mute"))
                                }
                            }
                        }
                        provider = np
                        providerKind = "remote"
                        ms.setPlaybackToRemote(np)
                    }
                }
                "fixed" -> if (providerKind != "fixed") {
                    val np = object : VolumeProvider(VOLUME_CONTROL_FIXED, 1, 1) {}
                    provider = np
                    providerKind = "fixed"
                    ms.setPlaybackToRemote(np)
                }
                else -> if (providerKind != "local") {
                    provider = null
                    providerKind = "local"
                    ms.setPlaybackToLocal(media)
                }
            }
        }

        private fun release(c: Context) {
            val ms = session ?: return
            session = null
            shown = null
            provider = null
            providerKind = ""
            notified = ""
            runCatching {
                ms.isActive = false
                ms.release()
            }
            activity?.get()?.let { if (it.mediaController?.sessionToken == ms.sessionToken) it.mediaController = null }
            PlayerService.refresh(c)
        }

        /// The media notification's look for the session, or null when there is none.
        fun styled(c: Context, b: Notification.Builder): Boolean {
            val ms = session ?: return false
            val s = shown ?: return false
            val playing = s.optString("state") == "playing"
            val title = s.optString("title").ifEmpty { s.optString("zone_name").ifEmpty { "Odeion" } }
            val sub = listOf(s.optString("artist"), s.optString("zone_name")).filter { it.isNotEmpty() }.joinToString(" · ")
            b.setContentTitle(title).setContentText(sub).setVisibility(Notification.VISIBILITY_PUBLIC)
            val bmp = art
            if (bmp != null && artUrl == s.optString("art")) b.setLargeIcon(bmp)
            val compact = mutableListOf<Int>()
            fun add(icon: Int, label: String, a: String) {
                compact.add(compact.size)
                b.addAction(Notification.Action.Builder(Icon.createWithResource(c, icon), label, button(c, a)).build())
            }
            if (s.optBoolean("can_prev")) add(android.R.drawable.ic_media_previous, "Previous", "previous")
            if (playing) add(android.R.drawable.ic_media_pause, "Pause", "pause")
            else add(android.R.drawable.ic_media_play, "Play", "play")
            if (s.optBoolean("can_next")) add(android.R.drawable.ic_media_next, "Next", "next")
            b.setStyle(Notification.MediaStyle().setMediaSession(ms.sessionToken).setShowActionsInCompactView(*compact.take(3).toIntArray()))
            return true
        }

        private fun button(c: Context, a: String): PendingIntent {
            val i = Intent(actionName(c)).setPackage(c.packageName).putExtra("a", a)
            return PendingIntent.getBroadcast(c, a.hashCode(), i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }

        /// Hold Android's audio focus while the phone's own zone plays (the WebView takes its own
        /// for the page's player). `ducks`: Android lowers it itself during a short prompt
        /// (shared playback); otherwise (bit-perfect, Exclusive USB) the Rust side pauses instead.
        @JvmStatic
        fun focus(want: Boolean, ducks: Boolean) {
            main.post {
                val am = audio() ?: return@post
                if (!want) {
                    abandon(am)
                    return@post
                }
                if (focusRequest != null && focusDucks == ducks) return@post
                abandon(am)
                focusDucks = ducks
                // (Android 8 and later: the app's minSdk is 26)
                val r = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                    .setAudioAttributes(media)
                    .setWillPauseWhenDucked(!ducks)
                    .setOnAudioFocusChangeListener(focusListener, main)
                    .build()
                if (am.requestAudioFocus(r) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
                    focusRequest = r
                } else {
                    // Refused (a call): the phone's zone pauses, as on a loss; asked again when it plays
                    android.util.Log.i(TAG, "media controls: audio focus refused (a call?)")
                    emit(JSONObject().put("k", "focus").put("change", AudioManager.AUDIOFOCUS_LOSS))
                }
            }
        }

        private fun abandon(am: AudioManager) {
            val r = focusRequest ?: return
            focusRequest = null
            am.abandonAudioFocusRequest(r)
        }

        /// The channel the player's notification uses (PlayerService.kt makes it too).
        fun channel(c: Context) {
            val nm = c.getSystemService(NotificationManager::class.java) ?: return
            if (nm.getNotificationChannel(PlayerService.CHANNEL) == null) {
                nm.createNotificationChannel(
                    NotificationChannel(PlayerService.CHANNEL, "Player", NotificationManager.IMPORTANCE_LOW).apply {
                        description = "Shown while the app plays, or while this phone is a zone of your node"
                    }
                )
            }
        }

        fun active(): Boolean = session != null
    }
}
