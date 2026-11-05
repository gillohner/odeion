// Copied into the generated Android project by tools/release/android-patch.py,
// next to MainActivity.kt (the package line is rewritten to match).
//
// The USB DACs and headsets attached to the phone, for the app's built-in player
// (src/android.rs; parsed by pm-out-local's usb_dac.rs): an AudioDeviceCallback keeps a
// report (JSON) of every USB audio output with what Android lists for it (name, sample
// rates, encodings, channel counts, whether media is routed there), and the Rust side
// waits for the next change with next(). The matching UsbDevice adds its ids and, only
// when the app already has permission for it, its raw descriptors: this never asks.
//
// Playback is unchanged: Android's mixer still plays the DAC (shared mode).
package to.synonym.pubkymusic

import android.app.Activity
import android.content.Context
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.media.AudioAttributes
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import java.util.concurrent.TimeUnit
import java.util.concurrent.locks.ReentrantLock
import org.json.JSONArray
import org.json.JSONObject

class UsbAudio private constructor(private val context: Context) : AudioDeviceCallback() {
    override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>) = refresh()

    override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) = refresh()

    private fun refresh() {
        val text = try {
            report(context)
        } catch (e: Exception) {
            // A device that went away while it was read, a vendor's odd AudioManager: no DAC rather
            // than the last one seen (the next callback reports again)
            android.util.Log.w("pubky-music", "USB audio report failed: $e")
            empty()
        }
        lock.lock()
        try {
            if (text != current) {
                current = text
                version += 1
                changed.signalAll()
            }
        } finally {
            lock.unlock()
        }
    }

    companion object {
        private val lock = ReentrantLock()
        private val changed = lock.newCondition()
        private var current = ""
        private var version = 0L
        @Volatile private var watcher: UsbAudio? = null

        /// Starts watching (once); on the UI thread, whose looper delivers the callbacks.
        @JvmStatic
        fun start(activity: Activity) {
            if (watcher != null) return
            val context = activity.applicationContext
            val am = context.getSystemService(AudioManager::class.java) ?: return
            val w = UsbAudio(context)
            watcher = w
            // Registering reports the devices already attached through onAudioDevicesAdded too
            am.registerAudioDeviceCallback(w, null)
            w.refresh()
        }

        /// "<version>\n<report>" once the report is newer than `seen`, or after `timeoutMs`.
        /// Blocks: called from the Rust side's own thread, never the UI thread.
        @JvmStatic
        fun next(seen: Long, timeoutMs: Long): String {
            var left = TimeUnit.MILLISECONDS.toNanos(timeoutMs)
            lock.lock()
            try {
                while (version == seen && left > 0) {
                    left = changed.awaitNanos(left)
                }
                return "$version\n$current"
            } finally {
                lock.unlock()
            }
        }

        private fun isUsbSink(d: AudioDeviceInfo) =
            d.isSink && (d.type == AudioDeviceInfo.TYPE_USB_DEVICE || d.type == AudioDeviceInfo.TYPE_USB_HEADSET)

        private fun isAudio(u: UsbDevice) =
            (0 until u.interfaceCount).any { u.getInterface(it).interfaceClass == UsbConstants.USB_CLASS_AUDIO }

        /// The UsbDevice behind an audio device: AudioDeviceInfo carries no USB ids, so by product
        /// name, else the only USB audio device attached.
        private fun usbFor(d: AudioDeviceInfo, usb: List<UsbDevice>): UsbDevice? {
            val name = d.productName?.toString()?.trim()?.lowercase().orEmpty()
            val byName = usb.filter {
                val p = runCatching { it.productName }.getOrNull()?.trim()?.lowercase().orEmpty()
                p.isNotEmpty() && name.isNotEmpty() && (name.contains(p) || p.contains(name))
            }
            return byName.singleOrNull() ?: usb.singleOrNull()
        }

        private fun hex(b: ByteArray): String {
            val s = StringBuilder(b.size * 2)
            for (x in b) s.append(String.format("%02x", x.toInt() and 0xff))
            return s.toString()
        }

        private fun usbJson(um: UsbManager, u: UsbDevice): JSONObject {
            val o = JSONObject()
            o.put("vendor_id", u.vendorId)
            o.put("product_id", u.productId)
            runCatching { u.manufacturerName }.getOrNull()?.let { o.put("manufacturer", it) }
            runCatching { u.productName }.getOrNull()?.let { o.put("product", it) }
            // Only with a permission granted before (step 1 never asks)
            if (runCatching { um.hasPermission(u) }.getOrDefault(false)) {
                val conn = runCatching { um.openDevice(u) }.getOrNull()
                if (conn != null) {
                    try {
                        // A DAC's are a few hundred bytes; the cap keeps a report of several devices small
                        conn.rawDescriptors?.let { if (it.size <= 16384) o.put("descriptors", hex(it)) }
                    } finally {
                        conn.close()
                    }
                }
            }
            return o
        }

        private fun empty(): String =
            JSONObject().put("v", 1).put("sdk", Build.VERSION.SDK_INT).put("devices", JSONArray()).toString()

        private fun report(context: Context): String {
            val am = context.getSystemService(AudioManager::class.java)
            val um = context.getSystemService(UsbManager::class.java)
            val routed: Set<Int>? = if (Build.VERSION.SDK_INT >= 33) {
                runCatching {
                    val media = AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build()
                    am.getAudioDevicesForAttributes(media).map { it.id }.toSet()
                }.getOrNull()
            } else null
            val usb = runCatching { um?.deviceList?.values?.filter { isAudio(it) } }.getOrNull().orEmpty()
            val devices = JSONArray()
            for (d in am.getDevices(AudioManager.GET_DEVICES_OUTPUTS)) {
                if (!isUsbSink(d)) continue
                val o = JSONObject()
                o.put("id", d.id)
                o.put("type", d.type)
                o.put("name", d.productName?.toString().orEmpty())
                o.put("rates", JSONArray(d.sampleRates.toList()))
                o.put("encodings", JSONArray(d.encodings.toList()))
                o.put("channel_counts", JSONArray(d.channelCounts.toList()))
                if (routed != null) o.put("routed", d.id in routed)
                if (um != null) usbFor(d, usb)?.let { o.put("usb", usbJson(um, it)) }
                devices.put(o)
            }
            return JSONObject()
                .put("v", 1)
                .put("sdk", Build.VERSION.SDK_INT)
                .put("devices", devices)
                .toString()
        }
    }
}
