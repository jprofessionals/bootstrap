package mud.mop.codec

import org.msgpack.core.MessagePack
import org.msgpack.core.MessagePacker
import org.msgpack.value.Value
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.io.OutputStream
import java.nio.ByteBuffer

object MopCodec {
    const val MAX_MESSAGE_SIZE = 16 * 1024 * 1024 // 16 MB

    class ConnectionClosed : Exception("connection closed")
    class MessageTooLarge(size: Int) :
        Exception("message size $size exceeds max $MAX_MESSAGE_SIZE")

    fun writeFrame(out: OutputStream, msg: Map<String, Any?>) {
        val payload = ByteArrayOutputStream()
        val packer = MessagePack.newDefaultPacker(payload)
        packMap(packer, msg)
        packer.flush()
        val bytes = payload.toByteArray()

        if (bytes.size > MAX_MESSAGE_SIZE) {
            throw MessageTooLarge(bytes.size)
        }

        val header = ByteBuffer.allocate(4)
        header.putInt(bytes.size)
        out.write(header.array())
        out.write(bytes)
        out.flush()
    }

    fun readFrame(input: InputStream): Map<String, Any?> {
        val header = readExact(input, 4)
            ?: throw ConnectionClosed()
        val len = ByteBuffer.wrap(header).int

        if (len < 0 || len > MAX_MESSAGE_SIZE) {
            throw MessageTooLarge(len)
        }

        val payload = readExact(input, len)
            ?: throw ConnectionClosed()

        val unpacker = MessagePack.newDefaultUnpacker(payload)
        val value = unpacker.unpackValue()
        return valueToMap(value)
    }

    private fun readExact(input: InputStream, n: Int): ByteArray? {
        val buf = ByteArray(n)
        var offset = 0
        while (offset < n) {
            val read = input.read(buf, offset, n - offset)
            if (read == -1) {
                if (offset == 0) return null
                throw ConnectionClosed()
            }
            offset += read
        }
        return buf
    }

    private fun packMap(packer: MessagePacker, map: Map<String, Any?>) {
        packer.packMapHeader(map.size)
        for ((key, value) in map) {
            packer.packString(key)
            packValue(packer, value)
        }
    }

    private fun packValue(packer: MessagePacker, value: Any?) {
        when (value) {
            null -> packer.packNil()
            is Boolean -> packer.packBoolean(value)
            is Int -> packer.packLong(value.toLong())
            is Long -> packer.packLong(value)
            is Float -> packer.packDouble(value.toDouble())
            is Double -> packer.packDouble(value)
            is String -> packer.packString(value)
            is Map<*, *> -> {
                @Suppress("UNCHECKED_CAST")
                packMap(packer, value as Map<String, Any?>)
            }
            is List<*> -> {
                packer.packArrayHeader(value.size)
                for (item in value) packValue(packer, item)
            }
            else -> packer.packString(value.toString())
        }
    }

    private fun valueToMap(value: Value): Map<String, Any?> {
        val map = value.asMapValue().map()
        val result = mutableMapOf<String, Any?>()
        for ((k, v) in map) {
            result[k.asStringValue().asString()] = valueToAny(v)
        }
        return result
    }

    private fun valueToAny(value: Value): Any? = when {
        value.isNilValue -> null
        value.isBooleanValue -> value.asBooleanValue().boolean
        value.isIntegerValue -> value.asIntegerValue().asLong()
        value.isFloatValue -> value.asFloatValue().toDouble()
        value.isStringValue -> value.asStringValue().asString()
        value.isArrayValue -> value.asArrayValue().list().map { valueToAny(it) }
        value.isMapValue -> {
            val m = value.asMapValue().map()
            m.entries.associate { (k, v) ->
                k.asStringValue().asString() to valueToAny(v)
            }
        }
        else -> value.toString()
    }
}
