// Copied into the generated Android project by tools/release/android-patch.py,
// next to MainActivity.kt (the package line is rewritten to match).
//
// Bit-perfect playback to a USB DAC (Android 14+), for the app's built-in player
// (src/android.rs; the decisions are pm-out-local's bit_perfect.rs): the DAC's mixer is set
// to the track's exact format with MIXER_BEHAVIOR_BIT_PERFECT for USAGE_MEDIA, then an
// AudioTrack plays with the SAME AudioAttributes and the SAME AudioFormat, preferring the DAC.
// open() answers what Android read back and where the track plays, so the Rust side confirms
// bit-perfect only when both are right. When the DAC goes (unplugged, or Android moves the
// track elsewhere) every track on it is paused and emptied at once, so nothing plays on from
// the phone's speaker, and write() answers GONE. The mixer attributes are cleared when the DAC
// goes, when no track has played for a few seconds, and when the app closes, so system sounds
// come back.
package to.synonym.pubkymusic

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioMixerAttributes
import android.media.AudioTrack
import android.os.Build
import android.os.Handler
import android.os.Looper
import java.nio.ByteBuffer
import org.json.JSONObject

class BitPerfect private constructor() {
    private class Playing(val track: AudioTrack, val dacId: Int, val frameBytes: Int) {
        @Volatile var gone = false
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
        private val media: AudioAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()
        @Volatile private var context: Context? = null
        private val tracks = HashMap<Long, Playing>()
        private var next = 1L
        /// The DAC whose mixer attributes are set, if any.
        @Volatile private var preferred: AudioDeviceInfo? = null
        private val main = Handler(Looper.getMainLooper())
        private val clearLater = Runnable { synchronized(tracks) { if (tracks.isEmpty()) clear() } }

        fun init(c: Context) {
            context = c.applicationContext
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

        /// Set the DAC's mixer bit-perfect for this format and start a track in it. JSON:
        /// {ok, handle, buffer_frames, set_ok, readback, routed_id, routed_name} or {ok: false, why}.
        @JvmStatic
        fun open(dacId: Int, encoding: Int, rate: Int, channelMask: Int): String {
            if (Build.VERSION.SDK_INT < 34) return refused("bit-perfect needs Android 14")
            val am = audio() ?: return refused("no audio service")
            val dac = deviceFor(dacId) ?: return refused("the DAC is not attached")
            main.removeCallbacks(clearLater)
            val format = AudioFormat.Builder()
                .setEncoding(encoding)
                .setSampleRate(rate)
                .setChannelMask(channelMask)
                .build()
            val mixer = AudioMixerAttributes.Builder(format)
                .setMixerBehavior(AudioMixerAttributes.MIXER_BEHAVIOR_BIT_PERFECT)
                .build()
            val setOk = try {
                am.setPreferredMixerAttributes(media, dac, mixer)
            } catch (e: Exception) {
                false
            }
            if (setOk) preferred = dac
            val frameBytes = format.channelCount * when (encoding) {
                AudioFormat.ENCODING_PCM_16BIT -> 2
                AudioFormat.ENCODING_PCM_24BIT_PACKED -> 3
                else -> 4
            }
            val track = try {
                val min = AudioTrack.getMinBufferSize(rate, channelMask, encoding)
                AudioTrack.Builder()
                    .setAudioAttributes(media)
                    .setAudioFormat(format)
                    .setTransferMode(AudioTrack.MODE_STREAM)
                    // About half a second, and never under what Android asks for
                    .setBufferSizeInBytes(maxOf(min, rate * frameBytes / 2))
                    .build()
            } catch (e: Exception) {
                return refused("another app holds the DAC, or Android would not play this format (${e.message})")
            }
            track.setPreferredDevice(dac)
            val handle: Long
            val p = Playing(track, dacId, frameBytes)
            synchronized(tracks) {
                handle = next++
                tracks[handle] = p
            }
            // Android moves the track (the DAC went, or another device took over): stop at once
            track.addOnRoutingChangedListener(
                android.media.AudioRouting.OnRoutingChangedListener { r ->
                    val routed = r.routedDevice
                    if (routed != null && routed.id != dacId) lose(p)
                },
                main,
            )
            track.play()
            // Where it plays is known once it runs
            var routed: AudioDeviceInfo? = null
            for (i in 0 until 30) {
                routed = track.routedDevice
                if (routed != null) break
                Thread.sleep(10)
            }
            val readback = try {
                am.getPreferredMixerAttributes(media, dac)
            } catch (e: Exception) {
                null
            }
            return JSONObject()
                .put("ok", true)
                .put("handle", handle)
                .put("buffer_frames", track.bufferSizeInFrames)
                .put("set_ok", setOk)
                .put("readback", mixerJson(readback))
                .put("routed_id", routed?.id ?: JSONObject.NULL)
                .put("routed_name", routed?.productName?.toString() ?: JSONObject.NULL)
                .toString()
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

        /// Write `size` bytes from the start of `buf`, blocking. Bytes written, or GONE, or an
        /// AudioTrack error (negative).
        @JvmStatic
        fun write(handle: Long, buf: ByteBuffer, size: Int): Int {
            val p = get(handle) ?: return AudioTrack.ERROR_INVALID_OPERATION
            if (p.gone) return GONE
            buf.position(0)
            val n = p.track.write(buf, size, AudioTrack.WRITE_BLOCKING)
            return if (p.gone) GONE else n
        }

        @JvmStatic
        fun played(handle: Long): Long = get(handle)?.played() ?: 0L

        @JvmStatic
        fun bufferFrames(handle: Long): Int = get(handle)?.track?.bufferSizeInFrames ?: 0

        @JvmStatic
        fun pause(handle: Long) {
            get(handle)?.track?.pause()
        }

        @JvmStatic
        fun play(handle: Long) {
            val p = get(handle) ?: return
            if (!p.gone) p.track.play()
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
            main.removeCallbacks(clearLater)
            main.postDelayed(clearLater, 3000)
        }

        /// Android's media volume, 0 to 100 (in bit-perfect mode the DAC applies it where it can).
        @JvmStatic
        fun setVolume(percent: Int): Boolean {
            val am = audio() ?: return false
            val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC)
            am.setStreamVolume(AudioManager.STREAM_MUSIC, (percent.coerceIn(0, 100) * max + 50) / 100, 0)
            return true
        }

        /// Devices went away (UsbAudio's callback): tracks on them stop at once.
        fun removed(ids: Set<Int>) {
            val lost = synchronized(tracks) { tracks.values.filter { it.dacId in ids } }
            lost.forEach { lose(it) }
            val pid = preferred?.id
            if (pid != null && pid in ids) clear()
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
