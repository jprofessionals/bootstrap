package mud.mop.client

import mud.mop.codec.MopCodec
import org.newsclub.net.unix.AFUNIXSocket
import org.newsclub.net.unix.AFUNIXSocketAddress
import org.slf4j.LoggerFactory
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.withTimeout

class MopClient(
    private val input: InputStream,
    private val output: OutputStream,
    private val adapterName: String,
    private val language: String,
    private val version: String,
) {
    private val logger = LoggerFactory.getLogger(MopClient::class.java)
    private val writeLock = Any()
    private val requestCounter = AtomicLong(0)
    private val pendingRequests = ConcurrentHashMap<Long, CompletableDeferred<Any?>>()

    var onMessage: ((Map<String, Any?>) -> Unit)? = null

    companion object {
        const val REQUEST_TIMEOUT_MS = 10_000L

        fun connect(socketPath: String, adapterName: String, language: String, version: String): MopClient {
            val socket = AFUNIXSocket.newInstance()
            socket.connect(AFUNIXSocketAddress.of(File(socketPath)))
            return MopClient(
                input = socket.getInputStream(),
                output = socket.getOutputStream(),
                adapterName = adapterName,
                language = language,
                version = version,
            )
        }
    }

    fun sendMessage(msg: Map<String, Any?>) {
        synchronized(writeLock) {
            MopCodec.writeFrame(output, msg)
        }
    }

    fun sendHandshake() {
        sendMessage(mapOf(
            "type" to "handshake",
            "adapter_name" to adapterName,
            "language" to language,
            "version" to version,
        ))
    }

    suspend fun sendDriverRequest(action: String, params: Map<String, Any?>): Any? {
        val requestId = requestCounter.incrementAndGet()
        val deferred = CompletableDeferred<Any?>()
        pendingRequests[requestId] = deferred

        sendMessage(mapOf(
            "type" to "driver_request",
            "request_id" to requestId,
            "action" to action,
            "params" to params,
        ))

        return try {
            withTimeout(REQUEST_TIMEOUT_MS) { deferred.await() }
        } catch (e: TimeoutCancellationException) {
            pendingRequests.remove(requestId)
            throw RuntimeException("driver request '$action' timed out after ${REQUEST_TIMEOUT_MS}ms")
        }
    }

    fun readLoop() {
        try {
            while (true) {
                val msg = MopCodec.readFrame(input)
                if (!dispatchResponse(msg)) {
                    onMessage?.invoke(msg)
                }
            }
        } catch (_: MopCodec.ConnectionClosed) {
            logger.info("MOP connection closed")
        } catch (e: Exception) {
            logger.error("MOP read error", e)
        } finally {
            // Complete all pending requests with error
            for ((id, deferred) in pendingRequests) {
                deferred.completeExceptionally(RuntimeException("connection closed"))
                pendingRequests.remove(id)
            }
        }
    }

    private fun dispatchResponse(msg: Map<String, Any?>): Boolean {
        val type = msg["type"] as? String ?: return false
        if (type != "request_response" && type != "request_error") return false

        val requestId = (msg["request_id"] as? Number)?.toLong() ?: return false
        val deferred = pendingRequests.remove(requestId) ?: return false

        when (type) {
            "request_response" -> deferred.complete(msg["result"])
            "request_error" -> deferred.completeExceptionally(
                RuntimeException(msg["error"] as? String ?: "unknown error")
            )
        }
        return true
    }
}
