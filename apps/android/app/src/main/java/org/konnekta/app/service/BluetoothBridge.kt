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
 * service of this phone, and the phone connects to the computer's when a
 * paired computer is not linked (at start, when the app comes to the
 * screen, when Bluetooth turns on, then every few seconds): whoever dials,
 * the link is up at once. Every connection is piped to a loopback port of
 * the core, which multiplexes it (crates/konnekta-core/src/bluetooth.rs).
 * Trust, TLS, pairing and permissions are the core's: this class only moves
 * bytes. The phone and the computer must be paired in the system settings.
 */
class BluetoothBridge(private val context: Context) {
    companion object {
        private const val TAG = "Konnekta"

        /** Same as SERVICE_UUID in the core. */
        val SERVICE_UUID: UUID = UUID.fromString("6b6f6e6e-656b-7461-8000-4b4f4e4e454b")

        fun allowed(context: Context): Boolean =
            Build.VERSION.SDK_INT < Build.VERSION_CODES.S ||
                context.checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) == PackageManager.PERMISSION_GRANTED

        /** Tries between dials while a paired computer is not linked. */
        private const val DIAL_EVERY_MS = 8_000L

        @Volatile
        private var current: BluetoothBridge? = null

        /** Dial the computers now (the app came to the screen). */
        fun kick() {
            current?.wake()
        }
    }

    private val lock = Object()
    private var woken = false

    /** Bluetooth addresses with a live link (either side dialed). */
    private val linked = Collections.synchronizedSet(mutableSetOf<String>())

    private fun wake() = synchronized(lock) {
        woken = true
        lock.notifyAll()
    }

    private fun sleep(ms: Long) = synchronized(lock) {
        if (!woken) lock.wait(ms)
        woken = false
    }

    /** Bluetooth turned on: listen again and dial at once. */
    private val stateReceiver = object : android.content.BroadcastReceiver() {
        override fun onReceive(c: Context, i: android.content.Intent) {
            if (i.getIntExtra(BluetoothAdapter.EXTRA_STATE, -1) == BluetoothAdapter.STATE_ON) wake()
        }
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
        current = this
        runCatching {
            context.registerReceiver(stateReceiver, android.content.IntentFilter(BluetoothAdapter.ACTION_STATE_CHANGED))
        }
        Thread({ acceptLoop(adapter) }, "konnekta-bluetooth").start()
        Thread({ dialLoop(adapter) }, "konnekta-bluetooth-dial").start()
    }

    fun stop() {
        running = false
        if (current === this) current = null
        runCatching { context.unregisterReceiver(stateReceiver) }
        wake()
        runCatching { server?.close() }
        synchronized(sockets) { sockets.toList() }.forEach { runCatching { it.close() } }
        sockets.clear()
    }

    @SuppressLint("MissingPermission")
    private fun acceptLoop(adapter: BluetoothAdapter) {
        while (running) {
            if (!adapter.isEnabled) {
                Thread.sleep(1000)
                continue
            }
            val srv = try {
                adapter.listenUsingRfcommWithServiceRecord("Konnekta", SERVICE_UUID)
            } catch (e: Exception) {
                Log.w(TAG, "bluetooth listen failed", e)
                Thread.sleep(2000)
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

    /**
     * Dial the paired computers' Konnekta service while one of the paired
     * Konnekta computers is not linked: the computer takes the connection
     * and links at once, without waiting for its own next try.
     */
    @SuppressLint("MissingPermission")
    private fun dialLoop(adapter: BluetoothAdapter) {
        // Give the core a moment to start and the computer to dial first.
        sleep(1500)
        while (running) {
            if (adapter.isEnabled && needsLink()) {
                val computers = runCatching { adapter.bondedDevices.toList() }.getOrDefault(emptyList())
                    .filter { it.bluetoothClass?.majorDeviceClass == android.bluetooth.BluetoothClass.Device.Major.COMPUTER }
                    .filter { it.address !in linked }
                for (d in computers) {
                    if (!running || !needsLink()) break
                    val bt = try {
                        d.createRfcommSocketToServiceRecord(SERVICE_UUID).also { it.connect() }
                    } catch (e: Exception) {
                        // Computer away, off, or without Konnekta running.
                        Log.d(TAG, "bluetooth dial ${d.address}: ${e.message}")
                        null
                    }
                    if (bt != null) {
                        Log.i(TAG, "bluetooth link to ${d.address}")
                        bridge(bt)
                    }
                }
            }
            sleep(DIAL_EVERY_MS)
        }
    }

    /** A computer paired with Konnekta is not linked by any means. */
    private fun needsLink(): Boolean {
        val devices = runCatching { KonnektaApp.core?.callArray("devices") }.getOrNull() ?: return false
        return (0 until devices.length()).mapNotNull { devices.optJSONObject(it) }
            .any { it.optBoolean("paired") && !it.optBoolean("connected") }
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
                // Small buffers: Bluetooth is slow, and whatever waits here
                // reaches the computer late (the pointer would trail behind).
                Socket().apply {
                    receiveBufferSize = 32 * 1024
                    sendBufferSize = 32 * 1024
                    connect(java.net.InetSocketAddress(InetAddress.getByName("127.0.0.1"), port))
                }
            } catch (e: Exception) {
                runCatching { bt.close() }
                return@Thread
            }
            tcp.tcpNoDelay = true
            val address = bt.remoteDevice.address
            linked.add(address)
            val a = Closeable { runCatching { bt.close() } }
            val b = Closeable { runCatching { tcp.close() } }
            sockets.add(a); sockets.add(b)
            val up = Thread({ pump(bt.inputStream, tcp.getOutputStream()); a.close(); b.close() }, "konnekta-bt-in")
            up.start()
            pump(tcp.getInputStream(), bt.outputStream)
            a.close(); b.close()
            sockets.remove(a); sockets.remove(b)
            linked.remove(address)
            // Link lost: try again soon.
            wake()
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
