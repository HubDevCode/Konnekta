package org.konnekta.app.service

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothServerSocket
import android.bluetooth.BluetoothSocket
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.util.Log
import org.konnekta.app.KonnektaApp
import java.io.InputStream
import java.io.OutputStream
import java.net.InetAddress
import java.net.Socket
import java.util.Collections
import java.util.UUID

/**
 * Bluetooth link with a computer. The computer connects to the "Konnekta"
 * service of this phone; every connection is piped to a loopback port of the
 * core, which multiplexes it (crates/konnekta-core/src/bluetooth.rs). Trust,
 * TLS, pairing and permissions are the core's: this class only moves bytes.
 * The phone and the computer must be paired in the system Bluetooth settings.
 */
class BluetoothBridge(private val context: Context) {
    companion object {
        private const val TAG = "Konnekta"

        /** Same as SERVICE_UUID in the core. */
        val SERVICE_UUID: UUID = UUID.fromString("6b6f6e6e-656b-7461-8000-4b4f4e4e454b")

        fun allowed(context: Context): Boolean =
            Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
                context.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED
    }

    @Volatile
    private var running = false
    private var server: BluetoothServerSocket? = null
    private val sockets = Collections.synchronizedSet(mutableSetOf<Closeable>())

    private fun interface Closeable {
        fun close()
    }

    /** Start listening, if Bluetooth is on and the permission is granted. */
    @SuppressLint("MissingPermission")
    fun start() {
        if (running || !allowed(context)) return
        val adapter: BluetoothAdapter = context.getSystemService(BluetoothManager::class.java)?.adapter ?: return
        running = true
        Thread({ acceptLoop(adapter) }, "konnekta-bluetooth").start()
    }

    fun stop() {
        running = false
        runCatching { server?.close() }
        synchronized(sockets) { sockets.toList() }.forEach { runCatching { it.close() } }
        sockets.clear()
    }

    @SuppressLint("MissingPermission")
    private fun acceptLoop(adapter: BluetoothAdapter) {
        while (running) {
            if (!adapter.isEnabled) {
                Thread.sleep(5000)
                continue
            }
            val srv = try {
                adapter.listenUsingRfcommWithServiceRecord("Konnekta", SERVICE_UUID)
            } catch (e: Exception) {
                Log.w(TAG, "bluetooth listen failed", e)
                Thread.sleep(5000)
                continue
            }
            server = srv
            try {
                while (running) {
                    val bt = srv.accept()
                    // Only a computer paired with this phone in the system settings.
                    if (bt.remoteDevice.bondState != BluetoothDevice.BOND_BONDED) {
                        runCatching { bt.close() }
                        continue
                    }
                    bridge(bt)
                }
            } catch (e: Exception) {
                if (running) Log.d(TAG, "bluetooth accept ended: ${e.message}")
            } finally {
                runCatching { srv.close() }
            }
            if (running) Thread.sleep(1000)
        }
    }

    /** Pipe [bt] to the core's loopback bridge port, both ways, until either ends. */
    private fun bridge(bt: BluetoothSocket) {
        val port = runCatching { KonnektaApp.core?.callObject("info")?.optInt("bluetoothPort", 0) ?: 0 }.getOrDefault(0)
        if (port <= 0) {
            runCatching { bt.close() }
            return
        }
        Thread({
            val tcp = try {
                Socket(InetAddress.getByName("127.0.0.1"), port)
            } catch (e: Exception) {
                runCatching { bt.close() }
                return@Thread
            }
            tcp.tcpNoDelay = true
            val a = Closeable { runCatching { bt.close() } }
            val b = Closeable { runCatching { tcp.close() } }
            sockets.add(a); sockets.add(b)
            val up = Thread({ pump(bt.inputStream, tcp.getOutputStream()); a.close(); b.close() }, "konnekta-bt-in")
            up.start()
            pump(tcp.getInputStream(), bt.outputStream)
            a.close(); b.close()
            sockets.remove(a); sockets.remove(b)
        }, "konnekta-bt-out").start()
    }

    private fun pump(from: InputStream, to: OutputStream) {
        val buf = ByteArray(8192)
        try {
            while (true) {
                val n = from.read(buf)
                if (n < 0) break
                to.write(buf, 0, n)
                to.flush()
            }
        } catch (_: Exception) {
        }
    }
}
