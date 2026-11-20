package org.vork.net

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
import java.io.File

/**
 * Keeps the vØrk node running (a program from the app's native libraries),
 * restarted if it stops, with a notification saying how it is. Its
 * settings are its own `vork.toml`; this only makes sure it has a name and
 * waits for the interface from VorkVpn.
 */
class NodeService : Service() {
    private var node: Process? = null
    @Volatile private var running = false
    private var supervisor: Thread? = null
    private var multicast: android.net.wifi.WifiManager.MulticastLock? = null
    private val wake = java.util.concurrent.Semaphore(0)
    /** Link requests already shown as notifications. */
    private val told = mutableSetOf<String>()

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopSelf()
                return START_NOT_STICKY
            }
            ACTION_RESTART -> {
                val old = node
                Thread {
                    old?.destroy()
                    old?.waitFor()
                    wake.release()
                }.start()
                return START_STICKY
            }
        }
        if (running) return START_STICKY
        channel()
        val n = notification("Starting…")
        if (Build.VERSION.SDK_INT >= 34) startForeground(NOTIFICATION, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE) else startForeground(NOTIFICATION, n)
        running = true
        multicast = runCatching {
            (applicationContext.getSystemService(WIFI_SERVICE) as android.net.wifi.WifiManager)
                .createMulticastLock("vork-mdns").apply { setReferenceCounted(false); acquire() }
        }.getOrNull()
        supervisor = Thread(::supervise, "vork-node").apply { start() }
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        supervisor?.interrupt()
        node?.destroy()
        multicast?.release()
        super.onDestroy()
    }

    private fun start(vararg args: String): Process {
        val p = Node(this)
        return ProcessBuilder(listOf(p.program.absolutePath) + args)
            .apply {
                environment()["HOME"] = filesDir.absolutePath
                environment()["VORK_CONFIG"] = p.config.absolutePath
                redirectErrorStream(true)
                redirectOutput(ProcessBuilder.Redirect.appendTo(p.log))
            }
            .start()
    }

    /** Its configuration made once; a name (the phone's model) and the interface from VorkVpn. */
    private fun prepare(p: Node) {
        if (!p.config.exists()) {
            p.dir.mkdirs()
            start("init", "--config", p.config.absolutePath).waitFor()
        }
        val conf = p.config.readText()
        if (!Regex("(?m)^tun\\s*=").containsMatchIn(conf)) p.config.appendText("\ntun = \"fd\"\n")
        if (!Regex("(?m)^name\\s*=").containsMatchIn(p.config.readText())) {
            val model = Build.MODEL.replace("\"", "").trim().ifEmpty { "android" }
            p.config.appendText("name = \"$model\"\n")
        }
        // A port of its own, chosen once: at home a phone shares the router's
        // NAT with a server on 51820, and a router that keeps source ports
        // gives the phone's flows that same outside port, so a node talking
        // to the phone could never reach the server's forwarded port.
        val fixed = Regex("(?m)^listen\\s*=\\s*\"0\\.0\\.0\\.0:51820\"\\s*$")
        val now = p.config.readText()
        if (fixed.containsMatchIn(now)) {
            val port = 40000 + java.security.SecureRandom().nextInt(20000)
            p.config.writeText(now.replace(fixed, "listen = \"0.0.0.0:$port\""))
        }
        // The log is the node's own: kept short.
        if (p.log.length() > 1_000_000) p.log.writeText("")
    }

    private fun supervise() {
        val p = Node(this)
        while (running) {
            try {
                if (node?.isAlive != true) {
                    prepare(p)
                    node = start("run", "--config", p.config.absolutePath)
                    VorkVpn.startIfAllowed(this)
                }
                val s = p.status()
                val text = when {
                    s.has("error") -> "starting the node"
                    else -> {
                        val nodes = s.optJSONArray("nodes")?.length() ?: 0
                        val up = (0 until (s.optJSONArray("neighbors")?.length() ?: 0)).count { s.getJSONArray("neighbors").getJSONObject(it).optBoolean("up") }
                        "${s.optString("name")}: $up connected, $nodes node(s) known"
                    }
                }
                getSystemService(NotificationManager::class.java).notify(NOTIFICATION, notification(text))
                // Someone asks to be one of this owner's devices: say so, even with the app closed.
                val asking = s.optJSONObject("links")?.optJSONArray("requests")
                for (i in 0 until (asking?.length() ?: 0)) {
                    val r = asking!!.getJSONObject(i)
                    val address = r.optString("address")
                    if (address in told) continue
                    told += address
                    val open = PendingIntent.getActivity(this, 2, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
                    val n = Notification.Builder(this, CHANNEL)
                        .setSmallIcon(R.drawable.ic_status)
                        .setContentTitle("${r.optString("name").ifEmpty { address.take(8) }} wants to be one of your devices")
                        .setContentText("Open vØrk to allow or dismiss it")
                        .setContentIntent(open)
                        .setAutoCancel(true)
                        .build()
                    getSystemService(NotificationManager::class.java).notify(address.hashCode(), n)
                }
                wake.tryAcquire(15_000L, java.util.concurrent.TimeUnit.MILLISECONDS)
                wake.drainPermits()
            } catch (e: InterruptedException) {
                return
            } catch (e: Exception) {
                try { Thread.sleep(10_000) } catch (stop: InterruptedException) { return }
            }
        }
    }

    private fun channel() {
        getSystemService(NotificationManager::class.java).createNotificationChannel(NotificationChannel(CHANNEL, "vØrk node", NotificationManager.IMPORTANCE_LOW))
    }

    private fun notification(text: String): Notification {
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        val stop = PendingIntent.getService(this, 1, Intent(this, NodeService::class.java).setAction(ACTION_STOP), PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, CHANNEL)
            .setSmallIcon(R.drawable.ic_status)
            .setContentTitle("vØrk")
            .setContentText(text)
            .setContentIntent(open)
            .setOngoing(true)
            .addAction(Notification.Action.Builder(null, "Stop", stop).build())
            .build()
    }

    companion object {
        const val CHANNEL = "node"
        const val NOTIFICATION = 1
        const val ACTION_STOP = "org.vork.net.STOP"
        const val ACTION_RESTART = "org.vork.net.RESTART"

        fun start(context: Context) {
            context.startForegroundService(Intent(context, NodeService::class.java))
        }

        fun restart(context: Context) {
            context.startService(Intent(context, NodeService::class.java).setAction(ACTION_RESTART))
        }

        fun stop(context: Context) {
            context.startService(Intent(context, NodeService::class.java).setAction(ACTION_STOP))
        }
    }
}
