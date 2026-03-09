package mud.launcher

import mud.mop.codec.MopCodec
import org.newsclub.net.unix.AFUNIXServerSocket
import org.newsclub.net.unix.AFUNIXSocketAddress
import kotlin.test.Test
import kotlin.test.assertEquals
import java.io.File

class LauncherIntegrationTest {
    @Test
    fun `launcher connects and sends handshake`() {
        val socketPath = "/tmp/mud-launcher-test-${System.nanoTime()}.sock"
        val socketFile = File(socketPath)
        socketFile.delete()

        val server = AFUNIXServerSocket.newInstance()
        server.bind(AFUNIXSocketAddress.of(socketFile))

        // Start launcher in background thread
        val launcherThread = Thread {
            try {
                val client = mud.mop.client.MopClient.connect(
                    socketPath = socketPath,
                    adapterName = "mud-adapter-jvm",
                    language = "kotlin",
                    version = "0.1.0",
                )
                client.sendHandshake()

                // Wait for ping and respond with pong
                client.onMessage = { msg ->
                    if (msg["type"] == "ping") {
                        client.sendMessage(mapOf("type" to "pong", "seq" to msg["seq"]))
                    }
                }
                client.readLoop()
            } catch (_: Exception) {}
        }
        launcherThread.isDaemon = true
        launcherThread.start()

        try {
            // Accept connection
            val conn = server.accept()
            val input = conn.getInputStream()
            val output = conn.getOutputStream()

            // Read handshake
            val handshake = MopCodec.readFrame(input)
            assertEquals("handshake", handshake["type"])
            assertEquals("mud-adapter-jvm", handshake["adapter_name"])
            assertEquals("kotlin", handshake["language"])

            // Send ping
            MopCodec.writeFrame(output, mapOf("type" to "ping", "seq" to 1L))

            // Read pong
            val pong = MopCodec.readFrame(input)
            assertEquals("pong", pong["type"])
            assertEquals(1L, (pong["seq"] as Number).toLong())

            conn.close()
        } finally {
            server.close()
            socketFile.delete()
        }
    }
}
