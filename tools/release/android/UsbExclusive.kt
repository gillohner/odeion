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
        /// USB devices Android detached, not yet taken by the Rust side (takeDetached): it forgets
        /// what the listener chose for exactly those devices.
        private val detachedLog = mutableListOf<JSONObject>()
        /// When a USB audio device was last attached (Android then shows its own "Open Odeion?"
        /// dialog for it, the app takes USB_DEVICE_ATTACHED) and when the app's activity last
        /// resumed: Auto's ask again waits for the resume after an attach, so two dialogs never stack.
        @Volatile private var attachedAt = 0L
        @Volatile private var resumedAt = 0L

        /// The app's activity resumed (UsbAudio's lifecycle callback).
        fun resumed() {
            resumedAt = SystemClock.elapsedRealtime()
        }

        fun init(c: Context) {
            if (context != null) return
            val app = c.applicationContext
            context = app
            val receiver = object : BroadcastReceiver() {
                override fun onReceive(ctx: Context, intent: Intent) {
                    when (intent.action) {
                        // Unplugged: the connection closes; the Rust side works on its own descriptor
                        // (handed over), and its next call finds the device gone. A claim recorded
                        // for it is over (a replug gives Android's driver a fresh device)
                        UsbManager.ACTION_USB_DEVICE_DETACHED -> {
                            val gone = deviceOf(intent)
                            synchronized(lock) {
                                if (gone != null) {
                                    detachedLog.add(
                                        JSONObject().put("vendor_id", gone.vendorId).put("product_id", gone.productId)
                                            .put("device_name", gone.deviceName)
                                    )
                                    while (detachedLog.size > 64) detachedLog.removeAt(0)
                                }
                                if (gone != null && device?.deviceName == gone.deviceName) {
                                    // Unplugged while the app held it: the speaker never comes on loud
                                    MediaControl.safeAfterUnplug()
                                    runCatching { conn?.close() }
                                    conn = null
                                    device = null
                                    claimed.clear()
                                }
                                if (gone != null && returning?.deviceName == gone.deviceName) returning = null
                                if (gone != null && recorded() == gone.deviceName) forget()
                            }
                            UsbAudio.refreshNow()
                        }
                        // Android's answer to the permission dialog: the report gains the DAC's
                        // descriptors, and the Rust side may open it
                        ACTION_PERMISSION -> UsbAudio.refreshNow()
                        // An audio device plugged in: Android's attach dialog may be up now
                        UsbManager.ACTION_USB_DEVICE_ATTACHED -> {
                            val d = deviceOf(intent)
                            if (d != null && isAudio(d)) attachedAt = SystemClock.elapsedRealtime()
                        }
                    }
                }
            }
            val filter = IntentFilter(UsbManager.ACTION_USB_DEVICE_DETACHED)
            filter.addAction(ACTION_PERMISSION)
            filter.addAction(UsbManager.ACTION_USB_DEVICE_ATTACHED)
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

        /// Auto's own ask (Exclusive USB is its first path): Android's dialog for this DAC once, and
        /// only while the app is on screen (Android shows it over the app). Someone who said no is
        /// not asked again; choosing Exclusive USB in Settings asks again (requestPermission).
        @JvmStatic
        fun requestPermissionOnce(vendor: Int, product: Int) {
            val me = android.app.ActivityManager.RunningAppProcessInfo()
            android.app.ActivityManager.getMyMemoryState(me)
            if (me.importance != android.app.ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND) return
            val p = prefs() ?: return
            val key = "auto-asked-$vendor:$product"
            if (p.getBoolean(key, false)) return
            // Counted only once Android was really asked (the DAC listed, the app ready)
            if (ask(vendor, product)) p.edit().putBoolean(key, true).apply()
        }

        /// Auto's ask again on a re-attach of a DAC whose permission was given before (a grant
        /// without "always" ends at the unplug): Android's dialog, only while the app is on screen,
        /// and not while Android's own attach dialog ("Open Odeion?") may be up: only once the app
        /// resumed after the attach (the Rust side asks again then). The Rust side asks once per
        /// attach. Whether Android was asked.
        @JvmStatic
        fun requestPermissionAgain(vendor: Int, product: Int): Boolean {
            val me = android.app.ActivityManager.RunningAppProcessInfo()
            android.app.ActivityManager.getMyMemoryState(me)
            if (me.importance != android.app.ActivityManager.RunningAppProcessInfo.IMPORTANCE_FOREGROUND) return false
            if (attachedAt > resumedAt) return false
            return ask(vendor, product)
        }

        /// Android's dialog for the DAC: when the listener chose Exclusive USB, or Auto's one ask.
        @JvmStatic
        fun requestPermission(vendor: Int, product: Int) {
            ask(vendor, product)
        }

        /// Whether Android's dialog was asked for.
        private fun ask(vendor: Int, product: Int): Boolean {
            val c = context ?: return false
            val d = find(vendor, product) ?: return false
            // Explicit (this package) and mutable: Android adds the answer to it
            val intent = Intent(ACTION_PERMISSION).setPackage(c.packageName)
            val flags = if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0
            val pi = PendingIntent.getBroadcast(c, 0, intent, flags)
            val um = usb() ?: return false
            return runCatching { um.requestPermission(d, pi) }.isSuccess
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
            return openDevice(um, d)
        }

        /// The device the Rust side chose to give back (pm-out-local's recovery_target, from
        /// detached()'s facts), opened exactly by its device name (its path): a lookup by name,
        /// nothing else is ever selected here. Answers as open().
        @JvmStatic
        fun openDetached(name: String): String {
            val um = usb() ?: return refused("no USB here")
            if (recorded() != name) return refused("$name is not the device this app claimed")
            val d = runCatching { um.deviceList[name] }.getOrNull()
                ?: return refused("$name is not attached")
            return openDevice(um, d)
        }

        private fun openDevice(um: UsbManager, d: UsbDevice): String {
            if (!runCatching { um.hasPermission(d) }.getOrDefault(false)) {
                return refused("Android's permission for the DAC was not given")
            }
            synchronized(lock) {
                closing = false
                conn?.let { c ->
                    if (device?.deviceName == d.deviceName) {
                        return JSONObject().put("ok", true).put("fd", handOver(c))
                            .put("descriptors", hex(c.rawDescriptors ?: ByteArray(0))).toString()
                    }
                    runCatching { c.close() }
                }
                val c = runCatching { um.openDevice(d) }.getOrNull() ?: return refused("Android would not open the DAC")
                // Recorded before Android's driver lets go of anything: if this process dies holding
                // it, the next start gives back exactly this device (and nothing else)
                record(d)
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
                            .put("fd", handOver(c)).put("descriptors", hex(c.rawDescriptors ?: ByteArray(0)))
                            .toString()
                    }
                    claimed.add(intf)
                }
                val raw = c.rawDescriptors ?: ByteArray(0)
                return JSONObject().put("ok", true).put("fd", handOver(c)).put("descriptors", hex(raw)).toString()
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
                forget()
            }
            UsbAudio.refreshNow()
            // Android lists it again by then, or it counts as gone
            BitPerfect.handler().postDelayed({ UsbAudio.refreshNow() }, RETURN_MS + 100)
        }

        /// A duplicate of the connection's descriptor, made here under the lock and given up to the
        /// Rust side, which owns it from now on (closes it with its transport): an unplug closing
        /// the connection can no longer take it from under Rust, or let Rust duplicate a number
        /// reused for something else.
        private fun handOver(c: UsbDeviceConnection): Int =
            android.os.ParcelFileDescriptor.fromFd(c.fileDescriptor).detachFd()

        private const val PREFS = "odeion_exclusive_usb"
        private const val CLAIMED = "claimed"

        private fun prefs() = context?.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

        /// The device this app claimed, kept across a crash: "deviceName|vendor|product".
        private fun record(d: UsbDevice) {
            prefs()?.edit()?.putString(CLAIMED, "${d.deviceName}|${d.vendorId}|${d.productId}")?.commit()
        }

        private fun forget() {
            prefs()?.edit()?.remove(CLAIMED)?.commit()
        }

        /// The recorded claim's device name, when there is one.
        private fun recorded(): String? = prefs()?.getString(CLAIMED, null)?.substringBefore('|')

        /// Facts for the Rust side's recovery after the app was killed holding a DAC (Android's
        /// driver never got it back): the claim recorded then, whether a DAC is held now, and every
        /// attached USB device by name. JSON: {"recorded":{device_name, vendor_id, product_id},
        /// "held":bool, "attached":[{device_name, vendor_id, product_id, audio}]}. The Rust side
        /// decides (only the recorded device, under the same name: never an identical DAC), then
        /// opens it with openDetached, gives its interfaces back and releases it (which forgets
        /// the record), or calls forgetClaim. Called once at start.
        @JvmStatic
        fun detached(): String {
            val out = JSONObject()
            val um = usb() ?: return out.toString()
            prefs()?.getString(CLAIMED, null)?.let { rec ->
                val parts = rec.split('|')
                val vendor = parts.getOrNull(1)?.toIntOrNull()
                val product = parts.getOrNull(2)?.toIntOrNull()
                if (parts.size == 3 && vendor != null && product != null) {
                    val claim = JSONObject().put("device_name", parts[0]).put("vendor_id", vendor)
                        .put("product_id", product)
                    out.put("recorded", claim)
                } else {
                    forget()
                }
            }
            out.put("held", synchronized(lock) { conn != null })
            val attached = JSONArray()
            val devices = runCatching { um.deviceList.values.toList() }.getOrDefault(emptyList())
            for (d in devices) {
                attached.put(
                    JSONObject().put("device_name", d.deviceName).put("vendor_id", d.vendorId)
                        .put("product_id", d.productId).put("audio", isAudio(d))
                )
            }
            out.put("attached", attached)
            return out.toString()
        }

        /// The recorded claim is over (its device unplugged since, or another one now).
        @JvmStatic
        fun forgetClaim() {
            forget()
        }

        /// One attached device as the Rust side tells DACs apart (pm-out-local's DacIdentity): its
        /// ids, its serial number when it has one (readable with the permission Exclusive USB has)
        /// and its device name (this attachment: a replug or another device is another path).
        private fun identity(d: UsbDevice): String {
            val o = JSONObject().put("vendor_id", d.vendorId).put("product_id", d.productId)
                .put("device_name", d.deviceName)
            val serial = runCatching { d.serialNumber }.getOrNull()?.trim()?.takeIf { it.isNotEmpty() }
            if (serial != null) o.put("serial", serial)
            return o.toString()
        }

        /// The DAC the app holds now (identity()); empty when none is held.
        @JvmStatic
        fun heldIdentity(): String {
            val d = synchronized(lock) { device } ?: return ""
            return identity(d)
        }

        /// The one attached audio device with these ids (identity()); empty when there is none, or
        /// more than one (they can't be told apart without the app holding one).
        @JvmStatic
        fun identityOf(vendor: Int, product: Int): String {
            val all = runCatching {
                usb()?.deviceList?.values?.filter { it.vendorId == vendor && it.productId == product && isAudio(it) }
            }.getOrNull() ?: return ""
            return all.singleOrNull()?.let { identity(it) } ?: ""
        }

        /// The devices Android detached since the last call, JSON [{vendor_id, product_id,
        /// device_name}], each once.
        @JvmStatic
        fun takeDetached(): String {
            val out = JSONArray()
            synchronized(lock) {
                detachedLog.forEach { out.put(it) }
                detachedLog.clear()
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
