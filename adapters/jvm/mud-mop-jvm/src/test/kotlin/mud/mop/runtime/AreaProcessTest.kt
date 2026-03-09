package mud.mop.runtime

import mud.mop.codec.MopCodec
import java.io.PipedInputStream
import java.io.PipedOutputStream
import kotlin.test.Test
import kotlin.test.assertEquals

class AreaProcessTest {
    @Test
    fun `responds to ping with pong`() {
        val processOut = PipedOutputStream()
        val launcherIn = PipedInputStream(processOut, 65536)
        val launcherOut = PipedOutputStream()
        val processIn = PipedInputStream(launcherOut, 65536)

        val process = AreaProcess(
            input = processIn,
            output = processOut,
            areaId = mapOf("namespace" to "test", "name" to "village"),
            areaPath = "/tmp/nonexistent",
            dbUrl = null,
            scanPackage = "mud.mop.runtime",
        )

        val thread = Thread { process.run() }
        thread.isDaemon = true
        thread.start()

        // Read area_loaded message first
        val loaded = MopCodec.readFrame(launcherIn)
        assertEquals("area_loaded", loaded["type"])

        // Send ping
        MopCodec.writeFrame(launcherOut, mapOf("type" to "ping", "seq" to 42L))

        // Read pong
        val resp = MopCodec.readFrame(launcherIn)
        assertEquals("pong", resp["type"])
        assertEquals(42L, (resp["seq"] as Number).toLong())

        // Close to let process exit
        launcherOut.close()
    }

    @Test
    fun `responds to get_web_data with template data`() {
        val processOut = PipedOutputStream()
        val launcherIn = PipedInputStream(processOut, 65536)
        val launcherOut = PipedOutputStream()
        val processIn = PipedInputStream(launcherOut, 65536)

        val process = AreaProcess(
            input = processIn,
            output = processOut,
            areaId = mapOf("namespace" to "test", "name" to "village"),
            areaPath = "/tmp/nonexistent",
            dbUrl = null,
            scanPackage = "mud.mop.runtime",
        )

        val thread = Thread { process.run() }
        thread.isDaemon = true
        thread.start()

        // Read area_loaded message first
        val loaded = MopCodec.readFrame(launcherIn)
        assertEquals("area_loaded", loaded["type"])

        // Send get_web_data request
        MopCodec.writeFrame(launcherOut, mapOf(
            "type" to "get_web_data",
            "request_id" to 99L,
            "area_key" to "test/village",
        ))

        val resp = MopCodec.readFrame(launcherIn)
        assertEquals("call_result", resp["type"])
        assertEquals(99L, (resp["request_id"] as Number).toLong())

        // Close to let process exit
        launcherOut.close()
    }
}
