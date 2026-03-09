package mud.launcher

import kotlinx.coroutines.*
import mud.mop.codec.MopCodec
import org.newsclub.net.unix.AFUNIXServerSocket
import org.newsclub.net.unix.AFUNIXSocketAddress
import org.slf4j.LoggerFactory
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicReference

enum class AreaState {
    BUILDING,
    STARTING,
    RUNNING,
    PENDING_UNLOAD,
}

class AreaEntry(
    val areaId: Map<String, String>,
    private val _state: AtomicReference<AreaState>,
    var process: Process? = null,
    var socketPath: String? = null,
    var serverSocket: AFUNIXServerSocket? = null,
    var outputStream: OutputStream? = null,
    var inputStream: InputStream? = null,
    var readJob: Job? = null,
    var stderrJob: Job? = null,
    var stdoutJob: Job? = null,
    var acceptJob: Job? = null,
) {
    constructor(areaId: Map<String, String>, state: AreaState) :
        this(areaId, AtomicReference(state))

    var state: AreaState
        get() = _state.get()
        set(value) { _state.set(value) }

    fun compareAndSetState(expected: AreaState, new: AreaState): Boolean =
        _state.compareAndSet(expected, new)
}

class AreaProcessManager(
    private val onSendToDriver: (Map<String, Any?>) -> Unit,
    private val onLog: (level: String, message: String) -> Unit,
) {
    private val logger = LoggerFactory.getLogger(AreaProcessManager::class.java)
    private val areas = ConcurrentHashMap<String, AreaEntry>()
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val gradleBuilder = GradleBuilder { level, msg -> onLog(level, msg) }

    fun hasArea(key: String): Boolean = areas.containsKey(key)
    fun getState(key: String): AreaState? = areas[key]?.state
    fun firstRunningAreaKey(): String? = areas.entries.firstOrNull { it.value.state == AreaState.RUNNING }?.key

    fun markBuilding(key: String, areaId: Map<String, String>) {
        areas[key] = AreaEntry(areaId = areaId, state = AreaState.BUILDING)
    }

    fun markRunning(key: String) {
        areas[key]?.state = AreaState.RUNNING
    }

    fun remove(key: String) {
        val entry = areas.remove(key) ?: return
        entry.readJob?.cancel()
        entry.stderrJob?.cancel()
        entry.stdoutJob?.cancel()
        entry.acceptJob?.cancel()
        entry.process?.destroyForcibly()
        try { entry.serverSocket?.close() } catch (_: Exception) {}
        entry.socketPath?.let { File(it).delete() }
    }

    fun requestUnload(key: String) {
        val entry = areas[key] ?: return
        if (entry.compareAndSetState(AreaState.BUILDING, AreaState.PENDING_UNLOAD)) {
            // Will be cleaned up when build completes
            return
        }
        // For RUNNING or STARTING, remove immediately
        if (entry.state == AreaState.RUNNING || entry.state == AreaState.STARTING) {
            remove(key)
        }
    }

    /**
     * Handle LoadArea: trigger async Gradle build, then spawn child JVM.
     * Does NOT block the caller -- build runs on a coroutine.
     */
    fun loadArea(areaId: Map<String, String>, areaPath: String, dbUrl: String?) {
        val key = "${areaId["namespace"]}/${areaId["name"]}"

        // If already loaded, unload first
        if (hasArea(key)) {
            remove(key)
        }

        markBuilding(key, areaId)
        onLog("info", "Building area $key...")

        scope.launch {
            // Async Gradle build
            val buildResult = if (GradleBuilder.isGradleProject(areaPath)) {
                gradleBuilder.build(areaPath)
            } else {
                onLog("warn", "No Gradle project at $areaPath, skipping build")
                BuildResult(success = true, jarPath = null)
            }

            // Check if unload was requested during build
            if (getState(key) == AreaState.PENDING_UNLOAD) {
                remove(key)
                onLog("info", "Area $key unloaded (was pending during build)")
                return@launch
            }

            if (!buildResult.success) {
                onLog("error", "Build failed for $key: ${buildResult.error}")
                onSendToDriver(mapOf(
                    "type" to "area_error",
                    "area_id" to areaId,
                    "error" to "build failed: ${buildResult.error}",
                ))
                remove(key)
                return@launch
            }

            // Spawn child JVM process
            spawnChild(key, areaId, areaPath, dbUrl, buildResult.jarPath)
        }
    }

    private fun spawnChild(
        key: String,
        areaId: Map<String, String>,
        areaPath: String,
        dbUrl: String?,
        jarPath: String?,
    ) {
        val entry = areas[key] ?: return

        // Create per-area Unix socket and bind server socket
        val socketPath = "/tmp/mud-area-${areaId["namespace"]}-${areaId["name"]}.sock"
        val socketFile = File(socketPath)
        socketFile.delete()

        entry.state = AreaState.STARTING
        entry.socketPath = socketPath

        val serverSocket = try {
            val ss = AFUNIXServerSocket.newInstance()
            ss.bind(AFUNIXSocketAddress.of(socketFile))
            ss
        } catch (e: Exception) {
            logger.error("Failed to create server socket for {}", key, e)
            onSendToDriver(mapOf(
                "type" to "area_error",
                "area_id" to areaId,
                "error" to "server socket failed: ${e.message}",
            ))
            remove(key)
            return
        }
        entry.serverSocket = serverSocket

        // Build the java command
        val javaCmd = System.getenv("JAVA_HOME")?.let { "$it/bin/java" } ?: "java"
        val cmd = mutableListOf(javaCmd)

        if (jarPath != null) {
            cmd.addAll(listOf("-jar", jarPath))
        } else {
            // Fallback: run AreaProcess directly from classpath
            cmd.addAll(listOf(
                "-cp", System.getProperty("java.class.path"),
                "mud.mop.runtime.AreaProcess"
            ))
        }

        val processBuilder = ProcessBuilder(cmd)
            .apply {
                environment()["MUD_SOCKET_PATH"] = socketPath
                environment()["MUD_AREA_NS"] = areaId["namespace"]
                environment()["MUD_AREA_NAME"] = areaId["name"]
                environment()["MUD_AREA_PATH"] = areaPath
                dbUrl?.let { environment()["MUD_DB_URL"] = it }
            }
            .redirectErrorStream(false)

        try {
            val process = processBuilder.start()
            entry.process = process

            // Capture stderr in background
            entry.stderrJob = scope.launch {
                process.errorStream.bufferedReader().forEachLine { line ->
                    onLog("warn", "[$key stderr] $line")
                }
            }

            // Capture stdout in background
            entry.stdoutJob = scope.launch {
                process.inputStream.bufferedReader().forEachLine { line ->
                    onLog("info", "[$key stdout] $line")
                }
            }

            // Accept child connection in background
            entry.acceptJob = scope.launch {
                try {
                    val conn = withContext(Dispatchers.IO) {
                        serverSocket.accept()
                    }
                    entry.inputStream = conn.getInputStream()
                    entry.outputStream = conn.getOutputStream()
                    entry.state = AreaState.RUNNING
                    logger.info("Area {} child connected", key)

                    // Forward messages from child to driver
                    entry.readJob = scope.launch {
                        try {
                            val input = entry.inputStream!!
                            while (true) {
                                val msg = MopCodec.readFrame(input)
                                onSendToDriver(msg)
                            }
                        } catch (_: MopCodec.ConnectionClosed) {
                            logger.info("Area {} child disconnected", key)
                        } catch (e: Exception) {
                            logger.error("Error reading from area {}", key, e)
                        }
                    }
                } catch (e: Exception) {
                    if (getState(key) != AreaState.PENDING_UNLOAD) {
                        logger.error("Failed to accept child connection for {}", key, e)
                        onSendToDriver(mapOf(
                            "type" to "area_error",
                            "area_id" to areaId,
                            "error" to "child connect failed: ${e.message}",
                        ))
                    }
                }
            }

            logger.info("Spawned child process for area {}", key)

        } catch (e: Exception) {
            logger.error("Failed to spawn child for {}", key, e)
            onSendToDriver(mapOf(
                "type" to "area_error",
                "area_id" to areaId,
                "error" to "spawn failed: ${e.message}",
            ))
            remove(key)
        }
    }

    /**
     * Route a message to the correct area's child process.
     */
    fun routeToArea(key: String, msg: Map<String, Any?>) {
        val entry = areas[key]
        if (entry == null) {
            logger.warn("No area process for {}", key)
            return
        }
        val out = entry.outputStream
        if (out == null) {
            logger.warn("Area {} not yet connected", key)
            return
        }
        synchronized(out) {
            MopCodec.writeFrame(out, msg)
        }
    }

    fun shutdown() {
        for (key in areas.keys.toList()) {
            remove(key)
        }
        scope.cancel()
    }
}
