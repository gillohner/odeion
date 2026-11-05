// Copied into the generated Android project by tools/release/android-patch.py,
// next to MainActivity.kt (the package line is rewritten to match).
//
// Exclusive USB: the app's own USB audio driver (pm-usb-audio, in Rust) plays the DAC through the
// USB device Android hands over. This class asks Android's permission for it, only when the
// listener chose "Exclusive USB" in Settings (never on plain detection: UsbAudio.kt never asks),
// opens it, takes its audio interfaces from Android's driver (claimInterface with force) and hands
// the file descriptor and raw descriptors to the Rust side (src/android.rs, pm-out-local's
// exclusive.rs). The Rust side gives the interfaces back to Android's driver (usbfs), then calls
// release(), which closes the connection: when the mode changes, the DAC goes, or the app closes.
//
// While the app holds the DAC Android lists no audio device for it: UsbAudio's report says the app
// holds it ("held"), so the Rust side keeps it as the phone's DAC; for a few seconds after a
// release too, until Android lists it again.
package to.synonym.pubkymusic

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbConstants
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbDeviceConnection
import android.hardware.usb.UsbInterface
import android.hardware.usb.UsbManager
import android.os.Build
import android.os.SystemClock
import android.media.AudioDeviceInfo
import android.media.AudioManager
import org.json.JSONArray
import org.json.JSONObject

class UsbExclusive private constructor() {
    companion object {
        private const val ACTION_PERMISSION = "to.synonym.pubkymusic.EXCLUSIVE_USB_PERMISSION"
        /// How long a DAC the app gave back may take to reappear in Android's list.
        private const val RETURN_MS = 3000L

        @Volatile private var context: Context? = null
        private val lock = Any()
        private var conn: UsbDeviceConnection? = null
        private var device: UsbDevice? = null
        private val claimed = mutableListOf<UsbInterface>()
        /// The DAC given back, and until when it counts as held.
        private var returning: UsbDevice? = null
        private var returningUntil = 0L
        /// The app is closing: the Rust side gives the DAC back once nothing plays on it.
        @Volatile private var closing = false

        fun init(c: Context) {
            if (context != null) return
            val app = c.applicationContext
            context = app
            val receiver = object : BroadcastReceiver() {
                override fun onReceive(ctx: Context, intent: Intent) {
                    when (intent.action) {
                        // Unplugged while held: nothing to give back, the connection is dead
                        // Unplugged: the connection closes; the Rust side works on its own duplicate
                        // of the descriptor, and its next call finds the device gone
                        UsbManager.ACTION_USB_DEVICE_DETACHED -> {
                            val gone = deviceOf(intent)
                            synchronized(lock) {
                                if (gone != null && device?.deviceName == gone.deviceName) {
                                    runCatching { conn?.close() }
                                    conn = null
                                    device = null
                                    claimed.clear()
                                }
                                if (gone != null && returning?.deviceName == gone.deviceName) returning = null
                            }
                            UsbAudio.refreshNow()
                        }
                        // Android's answer to the permission dialog: the report gains the DAC's
                        // descriptors, and the Rust side may open it
                        ACTION_PERMISSION -> UsbAudio.refreshNow()
                    }
                }
            }
            val filter = IntentFilter(UsbManager.ACTION_USB_DEVICE_DETACHED)
            filter.addAction(ACTION_PERMISSION)
            if (Build.VERSION.SDK_INT >= 33) {
                app.registerReceiver(receiver, filter, null, BitPerfect.handler(), Context.RECEIVER_NOT_EXPORTED)
            } else {
                app.registerReceiver(receiver, filter, null, BitPerfect.handler())
            }
        }

        @Suppress("DEPRECATION")
        private fun deviceOf(intent: Intent): UsbDevice? =
            if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(UsbManager.EXTRA_DEVICE, UsbDevice::class.java)
            else intent.getParcelableExtra(UsbManager.EXTRA_DEVICE)

        private fun usb(): UsbManager? = context?.getSystemService(UsbManager::class.java)

        private fun isAudio(u: UsbDevice) =
            (0 until u.interfaceCount).any { u.getInterface(it).interfaceClass == UsbConstants.USB_CLASS_AUDIO }

        private fun find(vendor: Int, product: Int): UsbDevice? =
            runCatching { usb()?.deviceList?.values?.firstOrNull { it.vendorId == vendor && it.productId == product && isAudio(it) } }
                .getOrNull()

        /// Android's permission for the DAC was given. Never asks.
        @JvmStatic
        fun hasPermission(vendor: Int, product: Int): Boolean {
            val d = find(vendor, product) ?: return false
            return runCatching { usb()?.hasPermission(d) ?: false }.getOrDefault(false)
        }

        /// Android's dialog for the DAC: only when the listener chose Exclusive USB.
        @JvmStatic
        fun requestPermission(vendor: Int, product: Int) {
            val c = context ?: return
            val d = find(vendor, product) ?: return
            // Explicit (this package) and mutable: Android adds the answer to it
            val intent = Intent(ACTION_PERMISSION).setPackage(c.packageName)
            val flags = if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0
            val pi = PendingIntent.getBroadcast(c, 0, intent, flags)
            runCatching { usb()?.requestPermission(d, pi) }
        }

        private fun refused(why: String) = JSONObject().put("ok", false).put("why", why).toString()

        private fun hex(b: ByteArray): String {
            val s = StringBuilder(b.size * 2)
            for (x in b) s.append(String.format("%02x", x.toInt() and 0xff))
            return s.toString()
        }

        /// Open the DAC and take its audio interfaces from Android's driver:
        /// {"ok":true,"fd":N,"descriptors":"<hex>"}, or {"ok":false,"why":"..."}.
        @JvmStatic
        fun open(vendor: Int, product: Int): String {
            val um = usb() ?: return refused("no USB here")
            val d = find(vendor, product) ?: return refused("the DAC is not attached")
            if (!runCatching { um.hasPermission(d) }.getOrDefault(false)) {
                return refused("Android's permission for the DAC was not given")
            }
            synchronized(lock) {
                closing = false
                conn?.let { c ->
                    if (device?.deviceName == d.deviceName) {
                        return JSONObject().put("ok", true).put("fd", c.fileDescriptor)
                            .put("descriptors", hex(c.rawDescriptors ?: ByteArray(0))).toString()
                    }
                    runCatching { c.close() }
                }
                val c = runCatching { um.openDevice(d) }.getOrNull() ?: return refused("Android would not open the DAC")
                // Held before Android's driver lets go, so the report that follows keeps the DAC
                conn = c
                device = d
                claimed.clear()
                returning = null
                for (i in 0 until d.interfaceCount) {
                    val intf = d.getInterface(i)
                    if (intf.interfaceClass != UsbConstants.USB_CLASS_AUDIO) continue
                    if (claimed.any { it.id == intf.id }) continue
                    if (!c.claimInterface(intf, true)) {
                        // Some interfaces are taken from Android's driver already: the connection
                        // stays open, and the Rust side gives them back to Android's driver, then
                        // calls release() (closing alone would leave them without a driver)
                        return JSONObject().put("ok", false).put("why", "another app holds the DAC")
                            .put("fd", c.fileDescriptor).put("descriptors", hex(c.rawDescriptors ?: ByteArray(0)))
                            .toString()
                    }
                    claimed.add(intf)
                }
                val raw = c.rawDescriptors ?: ByteArray(0)
                return JSONObject().put("ok", true).put("fd", c.fileDescriptor).put("descriptors", hex(raw)).toString()
            }
        }

        /// Close the connection (the Rust side gave the interfaces back to Android's driver first).
        @JvmStatic
        fun release() {
            synchronized(lock) {
                val c = conn ?: return
                runCatching { c.close() }
                returning = device
                returningUntil = SystemClock.elapsedRealtime() + RETURN_MS
                conn = null
                device = null
                claimed.clear()
            }
            UsbAudio.refreshNow()
            // Android lists it again by then, or it counts as gone
            BitPerfect.handler().postDelayed({ UsbAudio.refreshNow() }, RETURN_MS + 100)
        }

        /// Attached audio devices the app may open that Android lists no USB audio output for: left
        /// without Android's driver by an app killed while it held them. JSON: [{vendor_id,
        /// product_id}]. Called once at start; the Rust side gives them back (the connection is
        /// opened for that and released).
        @JvmStatic
        fun detached(): String {
            val out = JSONArray()
            val c = context ?: return out.toString()
            val um = usb() ?: return out.toString()
            val am = c.getSystemService(AudioManager::class.java) ?: return out.toString()
            val listed = runCatching {
                am.getDevices(AudioManager.GET_DEVICES_OUTPUTS).filter {
                    it.type == AudioDeviceInfo.TYPE_USB_DEVICE || it.type == AudioDeviceInfo.TYPE_USB_HEADSET
                }.map { it.productName?.toString()?.trim()?.lowercase().orEmpty() }
            }.getOrDefault(emptyList())
            val held = synchronized(lock) { device?.deviceName }
            for (d in runCatching { um.deviceList.values.toList() }.getOrDefault(emptyList())) {
                if (!isAudio(d) || d.deviceName == held) continue
                if (!runCatching { um.hasPermission(d) }.getOrDefault(false)) continue
                val name = runCatching { d.productName }.getOrNull()?.trim()?.lowercase().orEmpty()
                val known = listed.any { it.isNotEmpty() && name.isNotEmpty() && (it.contains(name) || name.contains(it)) }
                if (!known) out.put(JSONObject().put("vendor_id", d.vendorId).put("product_id", d.productId))
            }
            return out.toString()
        }

        /// The app holds a DAC now.
        fun holding(): Boolean = synchronized(lock) { conn != null }

        /// For UsbAudio's report: the DAC the app holds (or gave back moments ago) while it is
        /// still attached.
        fun heldJson(): JSONObject? {
            val d = synchronized(lock) {
                device ?: returning?.takeIf { SystemClock.elapsedRealtime() < returningUntil }
            } ?: return null
            val attached = runCatching { usb()?.deviceList?.values?.any { it.deviceName == d.deviceName } }.getOrNull() ?: false
            if (!attached) return null
            return JSONObject().put("vendor_id", d.vendorId).put("product_id", d.productId)
        }

        /// The app closes: the Rust side gives the DAC back once nothing plays on it.
        fun appClosing() {
            if (!holding()) return
            closing = true
            UsbAudio.refreshNow()
        }

        fun closingNow(): Boolean = closing
    }
}
