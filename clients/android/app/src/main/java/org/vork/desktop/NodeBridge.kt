package org.vork.desktop

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.net.LocalServerSocket
import android.net.LocalSocket
import android.net.LocalSocketAddress
import android.os.IBinder
import android.os.Parcel
import android.os.ParcelFileDescriptor
import java.io.File

/**
 * vØrk storage runs on the vØrk app's node (org.vork.net), whose sockets
 * this app can't open. The vØrk app hands out connections over Binder
 * (its ApiService); this hands them on to vorkd, vork-web and the core
 * through a socket of this app's own ([socket]): a program connects,
 * writes which endpoint it wants ("api" or "control", a line), and gets
 * the connection as a file descriptor with one byte (vork-api's `broker:`
 * endpoints). Nothing is copied through here: each connection is the
 * node's own.
 */
object NodeBridge {
    private const val NET = "org.vork.net"
    private const val DESCRIPTOR = "org.vork.net.Api"
    private const val CONNECT_API = IBinder.FIRST_CALL_TRANSACTION
    private const val CONNECT_CONTROL = IBinder.FIRST_CALL_TRANSACTION + 1

    @Volatile private var binder: IBinder? = null
    @Volatile private var started = false
    private val lock = Object()

    fun socket(context: Context) = File(context.filesDir, "node-broker.sock")

    /** The endpoints programs and the core use (vork-api's `broker:`). */
    fun api(context: Context) = "broker:${socket(context).absolutePath}#api"
    fun control(context: Context) = "broker:${socket(context).absolutePath}#control"

    /** Whether the vØrk app is installed. */
    fun netInstalled(context: Context): Boolean =
        runCatching { context.packageManager.getPackageInfo(NET, 0) }.isSuccess

    /** Bound to the vØrk app, and handing connections on; once per process. */
    fun start(context: Context) {
        synchronized(lock) {
            if (started) return
            started = true
        }
        val app = context.applicationContext
        bind(app)
        Thread({ serve(app) }, "vork-node-bridge").apply { isDaemon = true }.start()
    }

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            synchronized(lock) {
                binder = service
                lock.notifyAll()
            }
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            binder = null // Android binds again when the vØrk app is back
        }

        override fun onBindingDied(name: ComponentName?) {
            binder = null
        }
    }

    private fun bind(context: Context): Boolean = runCatching {
        context.bindService(Intent("org.vork.net.action.API").setClassName(NET, "$NET.ApiService"), connection, Context.BIND_AUTO_CREATE)
    }.getOrDefault(false)

    /** A connection from the vØrk app's node, waiting a little for the binding; null if there's none. */
    private fun connect(context: Context, which: String): ParcelFileDescriptor? {
        val b = synchronized(lock) {
            if (binder == null) {
                bind(context)
                lock.wait(5_000)
            }
            binder
        } ?: return null
        val data = Parcel.obtain()
        val reply = Parcel.obtain()
        return try {
            data.writeInterfaceToken(DESCRIPTOR)
            b.transact(if (which == "control") CONNECT_CONTROL else CONNECT_API, data, reply, 0)
            reply.readException()
            if (reply.readInt() != 0) ParcelFileDescriptor.CREATOR.createFromParcel(reply) else null
        } catch (e: Exception) {
            null
        } finally {
            data.recycle()
            reply.recycle()
        }
    }

    private fun serve(context: Context) {
        val path = socket(context)
        path.delete()
        val server = LocalSocket(LocalSocket.SOCKET_STREAM).run {
            bind(LocalSocketAddress(path.absolutePath, LocalSocketAddress.Namespace.FILESYSTEM))
            LocalServerSocket(fileDescriptor)
        }
        while (true) {
            val client = try { server.accept() } catch (e: Exception) { Thread.sleep(1_000); continue }
            Thread {
                client.use { c ->
                    runCatching {
                        c.soTimeout = 10_000
                        val which = c.inputStream.bufferedReader().readLine()?.trim() ?: return@runCatching
                        val pfd = connect(context, which)
                        if (pfd != null) {
                            pfd.use {
                                c.setFileDescriptorsForSend(arrayOf(it.fileDescriptor))
                                c.outputStream.write(1)
                                c.outputStream.flush()
                            }
                        } else {
                            c.outputStream.write(0) // no node: the program says so and tries again later
                        }
                    }
                }
            }.start()
        }
    }
}
