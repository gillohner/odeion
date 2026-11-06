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
// On Android 14+ the report also lists the DAC's mixer attributes, and BitPerfect.kt plays
// to it bit-perfect; removing a device stops its bit-perfect tracks at once. While the app holds
// the DAC for Exclusive USB (UsbExclusive.kt) Android lists no audio device for it: the report
// says the app holds it ("held"), and Android's "becoming noisy" warning, which taking it causes,
// is not passed on.
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

    override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>) {
        // A bit-perfect track on a DAC that went stops at once, before anything else
        BitPerfect.removed(removedDevices.map { it.id }.toSet())
        refresh()
    }

    private fun refresh() {
        val text = try {
            report(context)
        } catch (e: Exception) {
            // A device that went away while it was read, a vendor's odd AudioManager: no DAC rather
            // than the last one seen (the next callback reports again)
            android.util.Log.w("pubky-music", "USB audio report failed: $e")
            empty()
        }
        publish(text)
    }

    companion object {
        private val lock = ReentrantLock()
        private val changed = lock.newCondition()
        private var current = ""
        private var version = 0L
        @Volatile private var watcher: UsbAudio? = null

        private fun publish(text: String) {
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

        /// Android's warning that audio is about to move off the device it plays on: the Rust side
        /// treats the DAC as gone at once (its shared stream goes silent, the zone pauses), before
        /// the device's removal arrives. A fresh report follows in two seconds either way.
        fun noisy() {
            if (UsbExclusive.holding()) return
            publish(
                JSONObject().put("v", 1).put("sdk", Build.VERSION.SDK_INT).put("devices", JSONArray())
                    .put("noisy", true).toString()
            )
            BitPerfect.handler().postDelayed({ watcher?.refresh() }, 2000)
        }

        /// A fresh report now (on the worker thread): Exclusive USB took or gave back the DAC, or
        /// Android answered its permission dialog.
        fun refreshNow() {
            BitPerfect.handler().post { watcher?.refresh() }
        }

        /// Starts watching (once); on the UI thread, whose looper delivers the callbacks.
        @JvmStatic
        fun start(activity: Activity) {
            if (watcher != null) return
            val context = activity.applicationContext
            val am = context.getSystemService(AudioManager::class.java) ?: return
            BitPerfect.init(context)
            UsbExclusive.init(context)
            // The app closing: the DAC's mixer back to Android's choice (system sounds come back)
            activity.application.registerActivityLifecycleCallbacks(object : android.app.Application.ActivityLifecycleCallbacks {
                override fun onActivityDestroyed(a: Activity) {
                    if (a.isFinishing) {
                        BitPerfect.clearIfIdle()
                        UsbExclusive.appClosing()
                    }
                }
                override fun onActivityCreated(a: Activity, b: android.os.Bundle?) {}
                override fun onActivityStarted(a: Activity) {}
                override fun onActivityResumed(a: Activity) {}
                override fun onActivityPaused(a: Activity) {}
                override fun onActivityStopped(a: Activity) {}
                override fun onActivitySaveInstanceState(a: Activity, b: android.os.Bundle) {}
            })
            val w = UsbAudio(context)
            watcher = w
            // Registering reports the devices already attached through onAudioDevicesAdded too
            // On a thread of its own, never behind the busy UI thread
            am.registerAudioDeviceCallback(w, BitPerfect.handler())
            // A change of Android's media volume (the volume keys) reports again at once: Android's
            // broadcast comes right away (the settings it persists ~500 ms later, the observer below)
            val volumeChanged = object : android.content.BroadcastReceiver() {
                override fun onReceive(ctx: Context, intent: android.content.Intent) {
                    w.refresh()
                }
            }
            val vf = android.content.IntentFilter("android.media.VOLUME_CHANGED_ACTION")
            if (Build.VERSION.SDK_INT >= 33) {
                context.registerReceiver(volumeChanged, vf, null, BitPerfect.handler(), Context.RECEIVER_NOT_EXPORTED)
            } else {
                context.registerReceiver(volumeChanged, vf, null, BitPerfect.handler())
            }
            context.contentResolver.registerContentObserver(
                android.provider.Settings.System.CONTENT_URI,
                true,
                object : android.database.ContentObserver(BitPerfect.handler()) {
                    override fun onChange(selfChange: Boolean) {
                        w.refresh()
                    }
                },
            )
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
                // Android 14: the formats its mixer can be set to for this DAC, and how (bit-perfect)
                if (Build.VERSION.SDK_INT >= 34) {
                    runCatching { am.getSupportedMixerAttributes(d) }.getOrNull()?.let { list ->
                        val mixer = JSONArray()
                        for (m in list) {
                            mixer.put(
                                JSONObject()
                                    .put("encoding", m.format.encoding)
                                    .put("rate", m.format.sampleRate)
                                    .put("channel_mask", m.format.channelMask)
                                    .put("channels", m.format.channelCount)
                                    .put("behavior", m.mixerBehavior)
                            )
                        }
                        o.put("mixer", mixer)
                    }
                }
                if (um != null) usbFor(d, usb)?.let { o.put("usb", usbJson(um, it)) }
                devices.put(o)
            }
            // Android's media volume: bit-perfect plays at full level, so the Rust side ends Auto's
            // bit-perfect the moment it is turned down (reported again at every change)
            val o = JSONObject()
                .put("v", 1)
                .put("sdk", Build.VERSION.SDK_INT)
                .put("devices", devices)
                .put("media_volume", BitPerfect.mediaVolume())
            UsbExclusive.heldJson()?.let { o.put("held", it) }
            if (UsbExclusive.closingNow()) o.put("release_exclusive", true)
            return o.toString()
        }
    }
}
