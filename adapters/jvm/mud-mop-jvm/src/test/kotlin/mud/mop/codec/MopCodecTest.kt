package mud.mop.codec

import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class MopCodecTest {
    @Test
    fun `round-trips a simple map`() {
        val msg = mapOf("type" to "ping", "seq" to 1L)
        val buf = ByteArrayOutputStream()
        MopCodec.writeFrame(buf, msg)
        val result = MopCodec.readFrame(ByteArrayInputStream(buf.toByteArray()))
        assertEquals("ping", result["type"])
        assertEquals(1L, (result["seq"] as Number).toLong())
    }

    @Test
    fun `rejects oversized frames`() {
        val buf = ByteArrayOutputStream()
        // Write a length prefix > MAX_MESSAGE_SIZE
        val len = MopCodec.MAX_MESSAGE_SIZE + 1
        buf.write(ByteArray(4) { i -> ((len shr (24 - i * 8)) and 0xFF).toByte() })
        buf.write(ByteArray(10))
        assertFailsWith<MopCodec.MessageTooLarge> {
            MopCodec.readFrame(ByteArrayInputStream(buf.toByteArray()))
        }
    }

    @Test
    fun `handles empty stream as connection closed`() {
        assertFailsWith<MopCodec.ConnectionClosed> {
            MopCodec.readFrame(ByteArrayInputStream(ByteArray(0)))
        }
    }

    @Test
    fun `round-trips nested map`() {
        val msg = mapOf(
            "type" to "driver_request",
            "params" to mapOf("files" to mapOf("a.txt" to "hello")),
            "tags" to listOf("one", "two"),
            "flag" to true,
            "nothing" to null,
            "pi" to 3.14,
        )
        val buf = ByteArrayOutputStream()
        MopCodec.writeFrame(buf, msg)
        val result = MopCodec.readFrame(ByteArrayInputStream(buf.toByteArray()))
        assertEquals("driver_request", result["type"])
        @Suppress("UNCHECKED_CAST")
        val params = result["params"] as Map<String, Any?>
        @Suppress("UNCHECKED_CAST")
        val files = params["files"] as Map<String, Any?>
        assertEquals("hello", files["a.txt"])
        @Suppress("UNCHECKED_CAST")
        val tags = result["tags"] as List<Any?>
        assertEquals(listOf("one", "two"), tags)
        assertEquals(true, result["flag"])
        assertEquals(null, result["nothing"])
    }
}
