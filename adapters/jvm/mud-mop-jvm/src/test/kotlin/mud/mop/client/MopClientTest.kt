package mud.mop.client

import mud.mop.codec.MopCodec
import java.io.PipedInputStream
import java.io.PipedOutputStream
import kotlin.test.Test
import kotlin.test.assertEquals
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

class MopClientTest {
    @Test
    fun `sends handshake on connect`() {
        val clientOut = PipedOutputStream()
        val serverIn = PipedInputStream(clientOut, 65536)
        val dummyOut = PipedOutputStream()

        val client = MopClient(
            input = PipedInputStream(dummyOut, 65536),
            output = clientOut,
            adapterName = "test-adapter",
            language = "kotlin",
            version = "0.1.0"
        )

        client.sendHandshake()

        val msg = MopCodec.readFrame(serverIn)
        assertEquals("handshake", msg["type"])
        assertEquals("test-adapter", msg["adapter_name"])
        assertEquals("kotlin", msg["language"])
    }

    @Test
    fun `send and receive driver request correlates by request_id`() {
        val clientOut = PipedOutputStream()
        val serverIn = PipedInputStream(clientOut, 65536)
        val serverOut = PipedOutputStream()
        val clientIn = PipedInputStream(serverOut, 65536)

        val client = MopClient(
            input = clientIn,
            output = clientOut,
            adapterName = "test",
            language = "kotlin",
            version = "0.1.0"
        )

        // Start read loop in a real background thread
        val readThread = Thread { client.readLoop() }
        readThread.isDaemon = true
        readThread.start()

        val resultRef = AtomicReference<Any?>()
        val latch = CountDownLatch(1)

        // Send driver request in a background thread
        Thread {
            try {
                val result = kotlinx.coroutines.runBlocking {
                    client.sendDriverRequest("set_area_template", mapOf("files" to mapOf<String, String>()))
                }
                resultRef.set(result)
            } catch (e: Exception) {
                resultRef.set(e)
            } finally {
                latch.countDown()
            }
        }.start()

        // Read the request from the server side (on main test thread)
        val req = MopCodec.readFrame(serverIn)
        assertEquals("driver_request", req["type"])
        assertEquals("set_area_template", req["action"])
        val reqId = (req["request_id"] as Number).toLong()

        // Send response back
        MopCodec.writeFrame(serverOut, mapOf(
            "type" to "request_response",
            "request_id" to reqId,
            "result" to true
        ))

        // Wait for the request thread to complete
        assert(latch.await(5, TimeUnit.SECONDS)) { "Driver request timed out" }
        assertEquals(true, resultRef.get())

        // Clean up
        serverOut.close()
    }
}
