// Copied into the generated Android project by tools/release/android-patch.py,
// next to MainActivity.kt (the package line is rewritten to match).
//
// Bit-perfect playback to a USB DAC (Android 14+), for the app's built-in player
// (src/android.rs; the decisions are pm-out-local's bit_perfect.rs): the DAC's mixer is set
// to the track's exact format with MIXER_BEHAVIOR_BIT_PERFECT for USAGE_MEDIA, then an
// AudioTrack plays with the SAME AudioAttributes and the SAME AudioFormat, preferring the DAC.
// open() answers what Android read back and where the track plays, so the Rust side confirms
// bit-perfect only when both are right (and uses no track Android plays elsewhere).
//
// Nothing reaches the phone's speaker when the DAC goes: Android's "becoming noisy" warning
// (sent before it reroutes), the device's removal and the track's routing listener all stop
// every track on the DAC at once, on a thread of their own (never behind the busy UI thread),
// and write() checks before every write that the track still plays through the DAC.
//
// A change to the DAC's mixer by anyone else withdraws the confirmation (status()). The mixer
// attributes are cleared when the DAC goes, a few seconds after the last track stops, after a
// minute with every track paused (and set again when one plays on), when an open fails, and
// when the app closes with nothing playing, so system sounds come back.
package to.synonym.pubkymusic

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioMixerAttributes
import android.media.AudioTrack
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import java.nio.ByteBuffer
import org.json.JSONObject

class BitPerfect private constructor() {
    private class Playing(val track: AudioTrack, val dacId: Int, val format: AudioFormat) {
        @Volatile var gone = false
        /// Why it is no longer bit-perfect as confirmed at open ("" while it is).
        @Volatile var unconfirmed = ""
        /// Paused since (0: playing).
        @Volatile var pausedAt = 0L
        /// The mixer attributes were cleared while it was paused: set them again before it plays.
        @Volatile var mixerCleared = false
        private var last = 0L
        private var wraps = 0L

        /// getPlaybackHeadPosition is an unsigned 32-bit count that wraps.
        @Synchronized fun played(): Long {
            val now = track.playbackHeadPosition.toLong() and 0xffffffffL
            if (now < last) wraps += 1L shl 32
            last = now
            return wraps + now
        }
    }

    companion object {
        const val GONE = -1000
        /// Every track paused this long: the mixer attributes are cleared (system sounds back).
        private const val IDLE_MS = 60_000L
        private val media: AudioAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()
        @Volatile private var context: Context? = null
        private val tracks = HashMap<Long, Playing>()
        private var next = 1L
        /// The DAC whose mixer attributes are set, if any.
        @Volatile private var preferred: AudioDeviceInfo? = null
        /// Listeners and timers run here, never behind the UI thread.
        /// The thread listeners and timers run on (UsbAudio's device callback too).
        fun handler(): Handler = worker

        private val worker: Handler by lazy {
            val t = HandlerThread("bit-perfect")
            t.start()
            Handler(t.looper)
        }
        private val clearLater = Runnable { synchronized(tracks) { if (tracks.isEmpty()) clear() } }
        private val idleCheck = object : Runnable {
            override fun run() {
                val now = System.currentTimeMillis()
                // Checked, marked and cleared under the lock play() and pause() take: a track
                // resumed meanwhile is never left playing on a cleared mixer as "confirmed"
                synchronized(tracks) {
                    val idle = tracks.values.isNotEmpty() &&
                        tracks.values.all { it.pausedAt != 0L && now - it.pausedAt >= IDLE_MS }
                    if (idle && preferred != null) {
                        tracks.values.forEach { it.mixerCleared = true }
                        clear()
                    }
                }
                worker.postDelayed(this, 10_000)
            }
        }

        fun init(c: Context) {
            if (context != null) return
            context = c.applicationContext
            // Android's warning before it moves audio off a device that goes (a USB DAC unplugged)
            val noisy = object : BroadcastReceiver() {
                override fun onReceive(ctx: Context, intent: Intent) {
                    if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) {
                        loseAll()
                        UsbAudio.noisy()
                    }
                }
            }
            val filter = IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY)
            if (Build.VERSION.SDK_INT >= 33) {
                c.applicationContext.registerReceiver(noisy, filter, null, worker, Context.RECEIVER_NOT_EXPORTED)
            } else {
                c.applicationContext.registerReceiver(noisy, filter, null, worker)
            }
            // Someone else changed the DAC's mixer (or cleared it): the tracks on it are no longer
            // what was confirmed
            if (Build.VERSION.SDK_INT >= 34) {
                runCatching {
                    audio()?.addOnPreferredMixerAttributesChangedListener(
                        java.util.concurrent.Executor { r -> worker.post(r) },
                        AudioManager.OnPreferredMixerAttributesChangedListener { attrs, device, mixer ->
                            mixerChanged(attrs, device, mixer)
                        },
                    )
                }
            }
            worker.postDelayed(idleCheck, 10_000)
        }

        private fun mixerChanged(attrs: AudioAttributes, device: AudioDeviceInfo, mixer: AudioMixerAttributes?) {
            if (attrs.usage != AudioAttributes.USAGE_MEDIA) return
            val mine = synchronized(tracks) { tracks.values.filter { it.dacId == device.id } }
            for (p in mine) {
                val same = mixer != null && mixer.mixerBehavior == AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT &&
                    mixer.format.encoding == p.format.encoding && mixer.format.sampleRate == p.format.sampleRate &&
                    mixer.format.channelMask == p.format.channelMask
                // Our own clear while the track is paused is no change to it; anything else is
                if (!same && !(p.mixerCleared && p.pausedAt != 0L)) {
                    p.unconfirmed = if (mixer == null) "Android's mixer for the DAC was cleared: not bit-perfect any more"
                    else "Android's mixer for the DAC was changed: not bit-perfect any more"
                }
            }
        }

        private fun audio(): AudioManager? = context?.getSystemService(AudioManager::class.java)

        private fun deviceFor(id: Int): AudioDeviceInfo? =
            audio()?.getDevices(AudioManager.GET_DEVICES_OUTPUTS)?.firstOrNull { it.id == id }

        private fun mixerJson(m: AudioMixerAttributes?): Any =
            if (m == null) JSONObject.NULL else JSONObject()
                .put("encoding", m.format.encoding)
                .put("rate", m.format.sampleRate)
                .put("channel_mask", m.format.channelMask)
                .put("channels", m.format.channelCount)
                .put("behavior", m.mixerBehavior)

        private fun refused(why: String) = JSONObject().put("ok", false).put("why", why).toString()

        /// The DAC is not there (any more): typed, so the Rust side pauses the zone instead of
        /// opening Android's default route.
        private fun gone(why: String) = JSONObject().put("ok", false).put("why", why).put("gone", true).toString()

        /// The one gate for "nothing reaches another device": a track may play only while it is
        /// not lost, the DAC it was opened for is still attached, and Android routes it there.
        /// write() and play() ask here; a track that fails it is lost (paused, flushed) for good.
        private fun reachable(p: Playing): Boolean {
            if (p.gone) return false
            if (deviceFor(p.dacId) == null) return false
            val routed = p.track.routedDevice
            return routed == null || routed.id == p.dacId
        }

        private fun mixerFor(format: AudioFormat) = AudioMixerAttributes.Builder(format)
            .setMixerBehavior(AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT)
            .build()

        /// Set the DAC's mixer bit-perfect for this format and start a track in it. JSON:
        /// {ok, handle, buffer_frames, set_ok, readback, routed_id, routed_name} or {ok: false, why}.
        /// A failure leaves nothing behind: no track, and the mixer as it was.
        @JvmStatic
        fun open(dacId: Int, encoding: Int, rate: Int, channelMask: Int): String {
            if (Build.VERSION.SDK_INT < 34) return refused("bit-perfect needs Android 14")
            val am = audio() ?: return refused("no audio service")
            val dac = deviceFor(dacId) ?: return gone("the DAC is not attached")
            worker.removeCallbacks(clearLater)
            val format = AudioFormat.Builder()
                .setEncoding(encoding)
                .setSampleRate(rate)
                .setChannelMask(channelMask)
                .build()
            val hadPreferred = preferred != null
            val setOk = try {
                am.setPreferredMixerAttributes(media, dac, mixerFor(format))
            } catch (e: Exception) {
                false
            }
            if (setOk) preferred = dac
            val frameBytes = format.channelCount * when (encoding) {
                AudioFormat.ENCODING_PCM_16BIT -> 2
                AudioFormat.ENCODING_PCM_24BIT_PACKED -> 3
                else -> 4
            }
            var track: AudioTrack? = null
            var handle = 0L
            try {
                val min = AudioTrack.getMinBufferSize(rate, channelMask, encoding)
                val t = AudioTrack.Builder()
                    .setAudioAttributes(media)
                    .setAudioFormat(format)
                    .setTransferMode(AudioTrack.MODE_STREAM)
                    // About half a second, and never under what Android asks for
                    .setBufferSizeInBytes(maxOf(min, rate * frameBytes / 2))
                    .build()
                track = t
                t.setPreferredDevice(dac)
                val p = Playing(t, dacId, format)
                // Android moves the track (the DAC went, or another device took over): stop at once
                t.addOnRoutingChangedListener(
                    android.media.AudioRouting.OnRoutingChangedListener { r ->
                        val routed = r.routedDevice
                        if (routed != null && routed.id != dacId) lose(p)
                    },
                    worker,
                )
                t.play()
                // Where it plays is known once it runs
                var routed: AudioDeviceInfo? = null
                for (i in 0 until 30) {
                    routed = t.routedDevice
                    if (routed != null) break
                    Thread.sleep(10)
                }
                val readback = try {
                    am.getPreferredMixerAttributes(media, dac)
                } catch (e: Exception) {
                    null
                }
                synchronized(tracks) {
                    handle = next++
                    tracks[handle] = p
                }
                return JSONObject()
                    .put("ok", true)
                    .put("handle", handle)
                    .put("buffer_frames", t.bufferSizeInFrames)
                    .put("set_ok", setOk)
                    .put("readback", mixerJson(readback))
                    .put("routed_id", routed?.id ?: JSONObject.NULL)
                    .put("routed_name", routed?.productName?.toString() ?: JSONObject.NULL)
                    .toString()
            } catch (e: Exception) {
                // Undo: no track left, and the mixer as it was when nothing else plays
                track?.let { runCatching { it.release() } }
                synchronized(tracks) { if (handle != 0L) tracks.remove(handle) }
                // Nothing else plays: the mixer back to Android's choice, now or soon (the last
                // track's own clean-up was cancelled by this open)
                if (synchronized(tracks) { tracks.isEmpty() }) {
                    if (setOk && !hadPreferred) clear() else worker.postDelayed(clearLater, 3000)
                }
                return refused("another app holds the DAC, or Android would not play this format (${e.message})")
            }
        }

        private fun get(handle: Long): Playing? = synchronized(tracks) { tracks[handle] }

        private fun lose(p: Playing) {
            if (p.gone) return
            p.gone = true
            try {
                p.track.pause()
                p.track.flush()
            } catch (e: Exception) {
            }
        }

        /// Every track stops at once (Android is about to move audio off the DAC).
        fun loseAll() {
            val all = synchronized(tracks) { tracks.values.toList() }
            all.forEach { lose(it) }
        }

        /// Write `size` bytes from the start of `buf`, blocking. Bytes written, or GONE, or an
        /// AudioTrack error (negative). Nothing is written once the track plays anywhere but the DAC.
        @JvmStatic
        fun write(handle: Long, buf: ByteBuffer, size: Int): Int {
            val p = get(handle) ?: return AudioTrack.ERROR_INVALID_OPERATION
            if (!reachable(p)) {
                lose(p)
                return GONE
            }
            buf.position(0)
            val n = p.track.write(buf, size, AudioTrack.WRITE_BLOCKING)
            return if (p.gone) GONE else n
        }

        /// "" while the track is bit-perfect as confirmed at open, else why not any more.
        @JvmStatic
        fun status(handle: Long): String {
            val p = get(handle) ?: return "the bit-perfect player stopped"
            val routed = p.track.routedDevice
            if (routed != null && routed.id != p.dacId) return "Android moved the track off the DAC"
            return p.unconfirmed
        }

        @JvmStatic
        fun played(handle: Long): Long = get(handle)?.played() ?: 0L

        @JvmStatic
        fun bufferFrames(handle: Long): Int = get(handle)?.track?.bufferSizeInFrames ?: 0

        @JvmStatic
        fun pause(handle: Long) {
            synchronized(tracks) {
                val p = tracks[handle] ?: return
                p.track.pause()
                if (p.pausedAt == 0L) p.pausedAt = System.currentTimeMillis()
            }
        }

        @JvmStatic
        fun play(handle: Long) {
            synchronized(tracks) {
                val p = tracks[handle] ?: return
                // Never started for a DAC that is not there or not routed (the next write says GONE)
                if (!reachable(p)) {
                    lose(p)
                    return
                }
                // Cleared during a long pause: set again first, then confirmed only as at open: the
                // mixer read back in exactly this format, and the track on the DAC
                if (p.mixerCleared && Build.VERSION.SDK_INT >= 34) {
                    val am = audio()
                    val dac = deviceFor(p.dacId)
                    val set = am != null && dac != null &&
                        runCatching { am.setPreferredMixerAttributes(media, dac, mixerFor(p.format)) }.getOrDefault(false)
                    if (set) preferred = dac
                    val back = if (set) runCatching { am!!.getPreferredMixerAttributes(media, dac!!) }.getOrNull() else null
                    val same = back != null && back.mixerBehavior == AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT &&
                        back.format.encoding == p.format.encoding && back.format.sampleRate == p.format.sampleRate &&
                        back.format.channelMask == p.format.channelMask
                    if (!same) p.unconfirmed = "Android refused bit-perfect again after the pause"
                    p.mixerCleared = false
                }
                p.pausedAt = 0L
                p.track.play()
                val routed = p.track.routedDevice
                if (routed != null && routed.id != p.dacId) p.unconfirmed = "Android moved the track off the DAC"
            }
        }

        @JvmStatic
        fun flush(handle: Long) {
            get(handle)?.track?.flush()
        }

        /// The track is done: released; the mixer attributes go a few seconds later unless
        /// another track starts (a format change between songs).
        @JvmStatic
        fun stop(handle: Long) {
            val p = synchronized(tracks) { tracks.remove(handle) } ?: return
            try {
                p.track.stop()
            } catch (e: Exception) {
            }
            p.track.release()
            worker.removeCallbacks(clearLater)
            worker.postDelayed(clearLater, 3000)
        }

        /// Android's media volume, 0 to 100 (-1: unknown), for UsbAudio's report. On a DAC Android
        /// plays bit-perfect it is the DAC's own volume (Android scales nothing and hands it to the
        /// DAC's hardware volume): the Rust side keeps it as that DAC's level.
        @JvmStatic
        fun mediaVolume(): Int {
            val am = audio() ?: return -1
            val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
            if (max <= 0) return -1
            val v = am.getStreamVolume(AudioManager.STREAM_MUSIC)
            // 100 only at the top step (the Rust side's mapping: media_volume.rs)
            if (v >= max) return 100
            return minOf(99, (v * 100 + max / 2) / max)
        }

        /// Before the first bit-perfect track on a DAC: Android's media volume down to `percent`
        /// (the DAC's last level, else a low one), never up, and only while Android routes media
        /// to that very DAC and nothing else (getAudioDevicesForAttributes, by its id: with
        /// several devices the index may be another's, the speaker's). Answers the media volume on
        /// that DAC as read back, 0 to 100; -1 when media isn't on that DAC alone (nothing set);
        /// -2 when Android keeps the volume fixed or didn't take the set (no start level). In bit-perfect mode Android passes
        /// the media volume to the DAC's own volume (BitPerfectThread, then the USB HAL's
        /// setVolume sets its feature unit); the samples stay untouched. Never forced to 100 %.
        @JvmStatic
        fun setMediaVolumeOn(dacId: Int, percent: Int): Int {
            if (Build.VERSION.SDK_INT < 33) return -1
            val am = audio() ?: return -1
            return runCatching {
                if (am.getAudioDevicesForAttributes(media).singleOrNull()?.id != dacId) return@runCatching -1
                // A device whose volume Android keeps fixed: no level can be set
                if (am.isVolumeFixed) return@runCatching -2
                val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
                if (max <= 0) return@runCatching -1
                // The nearest index, the top one only for 100 % (media_volume.rs index_for)
                val p = percent.coerceIn(0, 100)
                val target = if (p == 100) max else minOf(max - 1, (p * max + 50) / 100)
                val now = am.getStreamVolume(AudioManager.STREAM_MUSIC)
                // Lowered only: a level below it already stays (an unplug meanwhile lowers, never raises)
                if (now > target) am.setStreamVolume(AudioManager.STREAM_MUSIC, target, 0)
                // Read back: Android ignoring the set (a vendor quirk) is no start level
                val v = am.getStreamVolume(AudioManager.STREAM_MUSIC)
                if (v > target) return@runCatching -2
                if (v >= max) 100 else minOf(99, (v * 100 + max / 2) / max)
            }.getOrDefault(-1)
        }

        /// Devices went away (UsbAudio's callback): tracks on them stop at once.
        fun removed(ids: Set<Int>) {
            val lost = synchronized(tracks) { tracks.values.filter { it.dacId in ids } }
            lost.forEach { lose(it) }
            val pid = preferred?.id
            if (pid != null && pid in ids) clear()
        }

        /// The app closes: system sounds back, unless something still plays (the player service).
        fun clearIfIdle() {
            val playing = synchronized(tracks) { tracks.values.any { it.pausedAt == 0L && !it.gone } }
            if (!playing) {
                synchronized(tracks) { tracks.values.forEach { it.mixerCleared = true } }
                clear()
            }
        }

        /// System sounds back: the DAC's mixer as Android chooses.
        fun clear() {
            val dac = preferred ?: return
            preferred = null
            if (Build.VERSION.SDK_INT >= 34) {
                try {
                    audio()?.clearPreferredMixerAttributes(media, dac)
                } catch (e: Exception) {
                }
            }
        }
    }
}
