// Copied into the generated Android project by tools/release/android-patch.py, next to
// MainActivity.kt (the package line is rewritten to match). Left out of the .aab
// (`android-patch.py --without-updater`): an app from Google Play updates through Play only.
//
// The phone's part of app updates (crates/pm-desktop/src/update.rs decides what to install and
// checks the download's SHA-256): the installed versionCode, the question, and the install
// through Android's PackageInstaller. Android shows its own confirmation and installs only over
// an app signed with the same key; nothing here installs silently. Every method but `tell` blocks
// its caller, a background thread of the app (never the UI thread).
package to.synonym.pubkymusic

import android.app.Activity
import android.app.AlertDialog
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageInstaller
import android.net.Uri
import android.os.Build
import android.provider.Settings
import android.util.Log
import android.widget.Toast
import androidx.core.content.ContextCompat
import androidx.core.content.IntentCompat
import androidx.core.content.pm.PackageInfoCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import java.io.File
import java.security.MessageDigest
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

object AppUpdate {
    private const val TAG = "pubky-music"
    /** How long the app waits for "Install unknown apps" to be allowed. */
    private const val ALLOW_WAIT_MS = 10 * 60 * 1000L

    @JvmStatic
    fun installedVersionCode(activity: Activity): Long =
        PackageInfoCompat.getLongVersionCode(activity.packageManager.getPackageInfo(activity.packageName, 0))

    /** A question with two buttons; true for `yes`. False when the dialog goes away otherwise. */
    @JvmStatic
    fun ask(activity: Activity, title: String, message: String, yes: String, no: String): Boolean {
        val answer = AtomicBoolean(false)
        val done = CountDownLatch(1)
        activity.runOnUiThread {
            if (activity.isFinishing || activity.isDestroyed) {
                done.countDown()
                return@runOnUiThread
            }
            try {
                AlertDialog.Builder(activity)
                    .setTitle(title)
                    .setMessage(message)
                    .setPositiveButton(yes) { _, _ -> answer.set(true) }
                    .setNegativeButton(no) { _, _ -> }
                    .setOnDismissListener { done.countDown() }
                    .show()
            } catch (e: Exception) {
                Log.w(TAG, "updates: no dialog: $e")
                done.countDown()
            }
        }
        // A dialog nobody answers within a day is a no; the thread does not wait forever.
        done.await(24, TimeUnit.HOURS)
        return answer.get()
    }

    @JvmStatic
    fun tell(activity: Activity, message: String) {
        activity.runOnUiThread {
            Toast.makeText(activity.applicationContext, message, Toast.LENGTH_LONG).show()
        }
    }

    /**
     * Hands the APK at `path` to Android's installer once its SHA-256 is `sha256` (lower-case
     * hex). "" when Android has it (it asks the user to confirm, then installs and ends this
     * app); otherwise why not.
     */
    @JvmStatic
    fun install(activity: Activity, path: String, sha256: String): String {
        val file = File(path)
        if (!file.isFile) return "no downloaded file"
        if (!allowed(activity)) return "installing apps from Odeion is not allowed"
        if (!waitResumed(activity, ALLOW_WAIT_MS)) return "the app did not come back to the screen"
        val pkg = activity.packageName
        val installer = activity.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
        params.setAppPackageName(pkg)
        params.setSize(file.length())
        val id = installer.createSession(params)
        val session = installer.openSession(id)
        try {
            // The file is read once more as it goes to the installer, so what Android installs
            // is exactly what was checked.
            val digest = MessageDigest.getInstance("SHA-256")
            file.inputStream().use { input ->
                session.openWrite("odeion.apk", 0, file.length()).use { out ->
                    val buf = ByteArray(256 * 1024)
                    while (true) {
                        val n = input.read(buf)
                        if (n < 0) break
                        digest.update(buf, 0, n)
                        out.write(buf, 0, n)
                    }
                    session.fsync(out)
                }
            }
            val got = digest.digest().joinToString("") { "%02x".format(it) }
            if (!got.equals(sha256, ignoreCase = true)) {
                session.abandon()
                Log.w(TAG, "updates: not installed: the file's SHA-256 is $got, not $sha256")
                return "the download does not match the update"
            }
            val action = "$pkg.APP_UPDATE_STATUS.$id"
            val receiver = Status(activity)
            ContextCompat.registerReceiver(
                activity.applicationContext, receiver, IntentFilter(action), ContextCompat.RECEIVER_NOT_EXPORTED
            )
            // Mutable: the installer adds the status. Explicit (this package): Android 14 refuses
            // a mutable PendingIntent with an implicit Intent.
            val flags = android.app.PendingIntent.FLAG_UPDATE_CURRENT or
                (if (Build.VERSION.SDK_INT >= 31) android.app.PendingIntent.FLAG_MUTABLE else 0)
            val pending = android.app.PendingIntent.getBroadcast(
                activity.applicationContext, id, Intent(action).setPackage(pkg), flags
            )
            session.commit(pending.intentSender)
            Log.i(TAG, "updates: handed to the installer (session $id)")
            return ""
        } catch (e: Exception) {
            try {
                session.abandon()
            } catch (_: Exception) {
            }
            Log.w(TAG, "updates: the installer refused the file: $e")
            return "the installer refused the file: ${e.message}"
        } finally {
            session.close()
        }
    }

    /** "Install unknown apps" for Odeion: when off, say why it is needed and open its page. */
    private fun allowed(activity: Activity): Boolean {
        val pm = activity.packageManager
        if (pm.canRequestPackageInstalls()) return true
        val open = ask(
            activity,
            "Allow updates",
            "To install updates, Android needs your permission for Odeion to install apps.\n\n" +
                "On the next screen turn on \"Allow from this source\", then come back to Odeion.",
            "Open settings",
            "Not now",
        )
        if (!open) return false
        activity.runOnUiThread {
            try {
                activity.startActivity(
                    Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:${activity.packageName}"))
                )
            } catch (e: Exception) {
                Log.w(TAG, "updates: cannot open the setting: $e")
            }
        }
        val deadline = System.currentTimeMillis() + ALLOW_WAIT_MS
        while (System.currentTimeMillis() < deadline) {
            if (pm.canRequestPackageInstalls()) {
                Log.i(TAG, "updates: installing apps is allowed now")
                return true
            }
            Thread.sleep(1000)
        }
        return false
    }

    /** Android's confirmation can only open while the app is on screen. */
    private fun waitResumed(activity: Activity, timeoutMs: Long): Boolean {
        val owner = activity as? LifecycleOwner ?: return true
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            val resumed = AtomicBoolean(false)
            val read = CountDownLatch(1)
            activity.runOnUiThread {
                resumed.set(owner.lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED))
                read.countDown()
            }
            read.await(5, TimeUnit.SECONDS)
            if (resumed.get()) return true
            if (activity.isDestroyed) return false
            Thread.sleep(500)
        }
        return false
    }

    /** The installer's answers for one session: its confirmation screen, then the outcome. */
    private class Status(private val activity: Activity) : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)
            val message = intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE) ?: ""
            when (status) {
                PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                    val confirm = IntentCompat.getParcelableExtra(intent, Intent.EXTRA_INTENT, Intent::class.java)
                    Log.i(TAG, "updates: Android asks to confirm the install")
                    if (confirm != null) {
                        try {
                            if (activity.isDestroyed) {
                                context.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                            } else {
                                activity.startActivity(confirm)
                            }
                        } catch (e: Exception) {
                            Log.w(TAG, "updates: cannot show Android's confirmation: $e")
                        }
                    }
                    return
                }
                PackageInstaller.STATUS_SUCCESS -> Log.i(TAG, "updates: installed")
                else -> {
                    Log.w(TAG, "updates: not installed (status $status): $message")
                    if (status != PackageInstaller.STATUS_FAILURE_ABORTED) {
                        Toast.makeText(context, "Odeion was not updated: $message", Toast.LENGTH_LONG).show()
                    }
                }
            }
            try {
                context.applicationContext.unregisterReceiver(this)
            } catch (_: Exception) {
            }
        }
    }
}
