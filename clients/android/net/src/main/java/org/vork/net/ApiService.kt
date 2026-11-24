package org.vork.net

import android.app.Service
import android.content.Intent
import android.net.LocalSocket
import android.net.LocalSocketAddress
import android.os.Binder
import android.os.IBinder
import android.os.Parcel
import android.os.ParcelFileDescriptor

/**
 * The node's local API for the user's other vØrk apps (vØrk storage): they
 * can't open this app's sockets, so they bind here and ask for a
 * connection, which comes back as a file descriptor (Binder carries it).
 * Only apps signed as this one may bind (the API permission); what one may
 * then do is the node's own rules for its local API.
 *
 * Transactions: [CONNECT_API] or [CONNECT_CONTROL], no arguments; the
 * reply holds the connection (a ParcelFileDescriptor), or null when the
 * node isn't running.
 */
class ApiService : Service() {
    private val binder = object : Binder() {
        override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
            val node = Node(this@ApiService)
            val socket = when (code) {
                CONNECT_API -> java.io.File(node.state, "api.sock")
                CONNECT_CONTROL -> node.control
                else -> return super.onTransact(code, data, reply, flags)
            }
            data.enforceInterface(DESCRIPTOR)
            val pfd = try {
                LocalSocket().use { s ->
                    s.connect(LocalSocketAddress(socket.absolutePath, LocalSocketAddress.Namespace.FILESYSTEM))
                    ParcelFileDescriptor.dup(s.fileDescriptor)
                }
            } catch (e: Exception) {
                null
            }
            reply?.writeNoException()
            reply?.writeInt(if (pfd != null) 1 else 0)
            pfd?.writeToParcel(reply!!, android.os.Parcelable.PARCELABLE_WRITE_RETURN_VALUE)
            return true
        }
    }

    override fun onBind(intent: Intent?): IBinder {
        // The node runs while an app uses it (Android may refuse to start it
        // from the background; it starts at boot and with this app anyway).
        runCatching { NodeService.start(this) }
        return binder
    }

    companion object {
        const val DESCRIPTOR = "org.vork.net.Api"
        const val CONNECT_API = IBinder.FIRST_CALL_TRANSACTION
        const val CONNECT_CONTROL = IBinder.FIRST_CALL_TRANSACTION + 1
    }
}
