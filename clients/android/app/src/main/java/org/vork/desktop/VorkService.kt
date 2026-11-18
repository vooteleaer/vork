package org.vork.desktop

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * Keeps the vØrk node and vorkd running (programs from the app's native
 * libraries), each restarted if it stops; vorkd starts once the node
 * answers, and reconnects to it by itself. Its notification is Android's
 * tray: the same state as the desktop icon, with Open and Stop.
 */
class VorkService : Service() {
    private var vorkd: Process? = null
    private var node: Process? = null
    /** Lets multicast in while the node runs, for mDNS on the local network. */
    private var multicast: android.net.wifi.WifiManager.MulticastLock? = null
    @Volatile private var running = false
    /** Wakes the supervisor before its next round (Restart). */
    private val wake = java.util.concurrent.Semaphore(0)
    private var supervisor: Thread? = null
    private var watcher: Thread? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_RESTART -> {
                restarting = true
                lastExit = null
                // Started again at once, not at the supervisor's next round.
                val old = vorkd
                Thread {
                    old?.destroy()
                    old?.waitFor()
                    wake.release()
                }.start()
                return START_STICKY
            }
        }
        if (running) return START_STICKY
        Core.ready(this)
        channel()
        val notification = notification("Starting…")
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(NOTIFICATION, notification)
        }
        running = true
        multicast = runCatching {
            (applicationContext.getSystemService(WIFI_SERVICE) as android.net.wifi.WifiManager)
                .createMulticastLock("vork-mdns").apply { setReferenceCounted(false); acquire() }
        }.getOrNull()
        supervisor = Thread(::supervise, "vork-supervisor").apply { start() }
        watcher = Thread(::watchLibraries, "vork-library-watch").apply { start() }
        // The address books in the system's Contacts, after a boot or an
        // update too (once the user has allowed it).
        if (contactsAllowed() || calendarAllowed()) Accounts.ensure(this)
        if (contactsAllowed()) Accounts.syncContacts(this)
        if (calendarAllowed()) Accounts.syncCalendars()
        return START_STICKY
    }

    private fun contactsAllowed() = checkSelfPermission(android.Manifest.permission.WRITE_CONTACTS) == android.content.pm.PackageManager.PERMISSION_GRANTED
    private fun calendarAllowed() = checkSelfPermission(android.Manifest.permission.WRITE_CALENDAR) == android.content.pm.PackageManager.PERMISSION_GRANTED

    override fun onDestroy() {
        running = false
        daemonAlive = false
        multicast?.release()
        supervisor?.interrupt()
        watcher?.interrupt()
        vorkd?.destroy()
        // The node after vorkd, which may still be saying goodbye through it.
        Thread {
            vorkd?.waitFor()
            node?.destroy()
        }.start()
        super.onDestroy()
    }

    private fun start(program: File, name: String, vararg args: String): Process {
        val p = Core.Paths(this)
        return ProcessBuilder(listOf(program.absolutePath) + args)
            .apply {
                environment()["HOME"] = p.home.absolutePath
                environment()["VORK_STORE"] = p.store.absolutePath
                environment()["VORK_CONFIG"] = p.nodeConfig.absolutePath
                environment()["VORK_API"] = File(p.node, "state/api.sock").absolutePath
                environment()["VORK_CONTROL"] = File(p.node, "state/control.sock").absolutePath
                redirectErrorStream(true)
                redirectOutput(ProcessBuilder.Redirect.appendTo(File(p.logs, "$name.log")))
            }
            .start()
    }

    /**
     * The libraries a new install joins (vork' packaging/default-libraries,
     * an asset of the app), as a file vorkd reads: it joins each once.
     */
    private fun defaultLibraries(): File {
        val f = File(filesDir, "default-libraries")
        runCatching { assets.open("default-libraries").use { input -> f.outputStream().use { input.copyTo(it) } } }
        return f
    }


    /** Starts what's missing, every few seconds, and says how it is. */
    private fun supervise() {
        val p = Core.Paths(this)
        var firstStart = true
        var booksAt = System.currentTimeMillis()
        while (running) {
            try {
                // A library that came (or went) with contacts: its address book, hourly.
                if (System.currentTimeMillis() - booksAt > 3_600_000 && contactsAllowed()) {
                    booksAt = System.currentTimeMillis()
                    Accounts.syncContacts(this)
                }
                // The node first: its settings and identity made once, then
                // run (restarted if it stops).
                if (node?.isAlive != true) {
                    if (!p.nodeConfig.exists()) {
                        p.node.mkdirs()
                        start(p.vork, "node-init", "init", "--config", p.nodeConfig.absolutePath).waitFor()
                    }
                    // The interface comes from VorkVpn, handed over as a descriptor.
                    val conf = p.nodeConfig.readText()
                    if (!Regex("(?m)^tun\\s*=").containsMatchIn(conf)) p.nodeConfig.appendText("\ntun = \"fd\"\n")
                    // Its name as other nodes see it: the phone's model.
                    if (!Regex("(?m)^name\\s*=").containsMatchIn(conf)) {
                        val model = Build.MODEL.replace("\"", "").trim().ifEmpty { "android" }
                        p.nodeConfig.appendText("name = \"$model\"\n")
                    }
                    node = start(p.vork, "node", "run", "--config", p.nodeConfig.absolutePath)
                    VorkVpn.startIfAllowed(this)
                }
                // vorkd once the node answers (it reconnects by itself later).
                val up = Core.nodeUp()
                // Stopped by itself (not by Restart): why, for the app to show.
                vorkd?.let { old ->
                    if (!old.isAlive && !restarting) lastExit = "exit code ${old.exitValue()}" + logTail(File(p.logs, "vorkd.log")).let { if (it.isEmpty()) "" else ":\n$it" }
                }
                if (vorkd?.isAlive != true && up) {
                    restarting = false
                    vorkd = start(p.vorkd, "vorkd", "--default-libraries", defaultLibraries().absolutePath)
                    if (firstStart) phoneDefaults()
                    firstStart = false
                }
                daemonAlive = vorkd?.isAlive == true
                waitingForNode = !up
                when {
                    up -> update()
                    else -> update("waiting for the vØrk node")
                }
                wake.tryAcquire(if (vorkd?.isAlive == true) 15_000L else 3_000L, java.util.concurrent.TimeUnit.MILLISECONDS)
                wake.drainPermits()
            } catch (e: InterruptedException) {
                return
            } catch (e: Exception) {
                update("vork: ${e.message}")
                Thread.sleep(15_000)
            }
        }
    }

    /**
     * Syncs an address book, or the calendars, when its library changes:
     * vorkd says so the moment one does (`Core.waitChange`), so others'
     * changes are in the system's Contacts and Calendar at once rather than
     * at the next hourly sync.
     */
    private fun watchLibraries() {
        var known = JSONObject()
        while (running) {
            try {
                // The libraries now (one may have come or gone); a new one
                // watched from its count now.
                val books = if (contactsAllowed()) names(Core.contactLibraries()) else emptySet()
                val calendars = if (calendarAllowed()) names(Core.calendarLibraries()) else emptySet()
                val watch = JSONObject()
                for (ns in books + calendars) watch.put(ns, known.opt(ns) ?: JSONObject.NULL)
                // None (or vorkd away): asking which there are is a search, not every few seconds.
                if (watch.length() == 0) {
                    Thread.sleep(60_000)
                    continue
                }
                val answer = JSONObject(Core.waitChange(watch.toString(), 60_000))
                known = answer.optJSONObject("known") ?: JSONObject()
                if (!answer.optBoolean("changed")) continue
                val changed = known.keys().asSequence().filter { !watch.isNull(it) && known.optLong(it) != watch.optLong(it) }.toList()
                for (ns in changed) if (ns in books) Accounts.syncBook(this, ns)
                if (changed.any { it in calendars }) Accounts.syncCalendars()
            } catch (e: InterruptedException) {
                return
            } catch (e: Exception) {
                // vorkd away, or an answer not understood: not at once again.
                try { Thread.sleep(15_000) } catch (stop: InterruptedException) { return }
            }
        }
    }

    /** The `ns` of each library in a `[{"ns", …}]` answer; none for an error. */
    private fun names(answer: String): Set<String> {
        val list = org.json.JSONTokener(answer).nextValue() as? JSONArray ?: return emptySet()
        return (0 until list.length()).map { list.getJSONObject(it).getString("ns") }.toSet()
    }

    /**
     * A phone is a small node (vork ADR-0031): it holds nothing for others
     * unless the user says so, and keeps 1 GB free rather than the desktop's
     * 5 GB (a phone often has less than that free at all). Named after the
     * model. Set once, in its settings, which the user can change.
     */
    private fun phoneDefaults() {
        val prefs = getSharedPreferences("vork", MODE_PRIVATE)
        if (prefs.getInt("phone-defaults-version", 0) >= PHONE_DEFAULTS) return
        Thread.sleep(3_000) // vorkd writes its settings file at first start
        val defaults = JSONObject()
            .put("placement.hold_for_others", "0")
            .put("store.min_free", "1G")
        val current = try { JSONArray(Core.settings()) } catch (e: Exception) { JSONArray() }
        for (i in 0 until current.length()) {
            val s = current.getJSONObject(i)
            if (s.optString("key") == "device" && s.isNull("value")) {
                defaults.put("node.device", Build.MODEL.lowercase().replace(Regex("[^a-z0-9]+"), "-").trim('-'))
            }
        }
        if (Core.saveSettings(defaults.toString()).isEmpty()) {
            prefs.edit().putInt("phone-defaults-version", PHONE_DEFAULTS).apply()
            restarting = true
            vorkd?.destroy() // restarted by the loop, with the setting
        }
    }

    /** The last lines of a log, for saying why vorkd stopped. */
    private fun logTail(f: File, lines: Int = 4): String =
        runCatching { f.readLines().takeLast(lines).joinToString("\n") }.getOrDefault("")

    private fun update(note: String? = null) {
        val text = note ?: try {
            val s = JSONObject(Core.status())
            when {
                s.optBoolean("up") -> "vork running"
                else -> s.optJSONObject("starting")?.optString("text")?.let { "vork starting: $it" } ?: "vork starting"
            }
        } catch (e: Exception) {
            "vork: ${e.message}"
        }
        getSystemService(NotificationManager::class.java).notify(NOTIFICATION, notification(text))
    }

    private fun channel() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(NotificationChannel(CHANNEL, "vork running", NotificationManager.IMPORTANCE_LOW))
    }

    private fun notification(text: String): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val stop = PendingIntent.getService(this, 1, Intent(this, VorkService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_status)
            .setContentTitle("vork")
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .addAction(Notification.Action.Builder(null, "Stop", stop).build())
            .build()
    }

    companion object {
        const val CHANNEL = "vork"
        const val NOTIFICATION = 1
        /** Raised when phoneDefaults() gains a setting, so existing installs get it. */
        const val PHONE_DEFAULTS = 2
        const val ACTION_STOP = "org.vork.desktop.STOP"
        const val ACTION_RESTART = "org.vork.desktop.RESTART"

        /** Whether vorkd's process runs (it may not answer yet: starting). */
        @Volatile var daemonAlive = false
        /** Waiting for the vØrk node before vorkd can start. */
        @Volatile var waitingForNode = false
        /** Why vorkd last stopped by itself: its exit and last log lines. */
        @Volatile var lastExit: String? = null
        @Volatile private var restarting = false

        fun start(context: Context) {
            context.startForegroundService(Intent(context, VorkService::class.java))
        }

        fun restartDaemon(context: Context) {
            context.startService(Intent(context, VorkService::class.java).setAction(ACTION_RESTART))
        }

        fun stop(context: Context) {
            context.startService(Intent(context, VorkService::class.java).setAction(ACTION_STOP))
        }
    }
}
