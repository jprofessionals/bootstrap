# JVM Adapter Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Java/Kotlin adapter for the MUD driver with a launcher + per-area child process architecture, supporting Spring Boot, Ktor, and Quarkus frameworks.

**Architecture:** A thin Kotlin launcher process connects to the driver via MOP (MessagePack over Unix socket), routes messages by area ID, and manages per-area child JVM processes. Each area is an independent Gradle project that builds to a fat JAR. The launcher triggers async Gradle builds, captures build logs, and forwards them via MOP.

**Tech Stack:** Kotlin, Gradle (Kotlin DSL), msgpack-java, junixsocket, kotlinx-coroutines, ClassGraph, Flyway, SLF4J

**Reference files:**
- MOP protocol: `crates/mud-mop/src/message.rs` (message types), `crates/mud-mop/src/codec.rs` (wire format)
- Ruby adapter: `adapters/ruby/bin/mud-adapter` (entry point), `adapters/ruby/lib/mud_adapter/client.rb` (MOP client)
- Driver adapter mgmt: `crates/mud-driver/src/runtime/adapter_manager.rs`, `crates/mud-driver/src/config.rs`
- Driver template handling: `crates/mud-driver/src/server.rs:1219` (`handle_set_area_template`)
- Driver repo creation: `crates/mud-driver/src/server.rs:1261` (`handle_repo_create`), `crates/mud-driver/src/git/repo_manager.rs`
- Ruby area template: `adapters/ruby/lib/mud_adapter/stdlib/templates/area/`
- Build manager: `crates/mud-driver/src/web/build_manager.rs`

---

## Task 1: Gradle Multi-Project Scaffold

Set up the root Gradle project structure for the JVM adapter with three subprojects.

**Files:**
- Create: `adapters/jvm/build.gradle.kts`
- Create: `adapters/jvm/settings.gradle.kts`
- Create: `adapters/jvm/gradle.properties`
- Create: `adapters/jvm/mud-mop-jvm/build.gradle.kts`
- Create: `adapters/jvm/launcher/build.gradle.kts`
- Create: `adapters/jvm/stdlib/build.gradle.kts`

**Step 1: Create root build files**

`adapters/jvm/settings.gradle.kts`:
```kotlin
rootProject.name = "mud-adapter-jvm"

include("mud-mop-jvm")
include("launcher")
include("stdlib")
```

`adapters/jvm/build.gradle.kts`:
```kotlin
plugins {
    kotlin("jvm") version "2.1.0" apply false
}

allprojects {
    group = "mud"
    version = "0.1.0"

    repositories {
        mavenCentral()
    }
}
```

`adapters/jvm/gradle.properties`:
```properties
kotlin.code.style=official
org.gradle.parallel=true
```

**Step 2: Create mud-mop-jvm build file**

`adapters/jvm/mud-mop-jvm/build.gradle.kts`:
```kotlin
plugins {
    kotlin("jvm")
}

dependencies {
    implementation("org.msgpack:msgpack-core:0.9.8")
    implementation("com.kohlschutter.junixsocket:junixsocket-core:2.10.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    implementation("org.slf4j:slf4j-api:2.0.16")

    testImplementation(kotlin("test"))
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.9.0")
}
```

**Step 3: Create launcher build file**

`adapters/jvm/launcher/build.gradle.kts`:
```kotlin
plugins {
    kotlin("jvm")
    application
}

application {
    mainClass.set("mud.launcher.MainKt")
}

dependencies {
    implementation(project(":mud-mop-jvm"))
    implementation(project(":stdlib"))
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    implementation("ch.qos.logback:logback-classic:1.5.12")

    testImplementation(kotlin("test"))
}

tasks.jar {
    manifest { attributes("Main-Class" to "mud.launcher.MainKt") }
    duplicatesStrategy = DuplicatesStrategy.EXCLUDE
    from(configurations.runtimeClasspath.get().map { if (it.isDirectory) it else zipTree(it) })
}
```

**Step 4: Create stdlib build file**

`adapters/jvm/stdlib/build.gradle.kts`:
```kotlin
plugins {
    kotlin("jvm")
}

dependencies {
    implementation(project(":mud-mop-jvm"))

    testImplementation(kotlin("test"))
}
```

**Step 5: Run Gradle wrapper setup**

Run: `cd adapters/jvm && gradle wrapper --gradle-version 8.12`
Expected: `gradlew`, `gradlew.bat`, `gradle/wrapper/` created

**Step 6: Verify build compiles**

Run: `cd adapters/jvm && ./gradlew build`
Expected: BUILD SUCCESSFUL (no source files yet, but project structure resolves)

**Step 7: Commit**

```bash
git add adapters/jvm/
git commit -m "feat(jvm): scaffold Gradle multi-project for JVM adapter"
```

---

## Task 2: MOP Wire Protocol — Codec

Implement the length-prefixed MessagePack codec matching the Rust driver's wire format:
4-byte big-endian u32 length prefix + MessagePack payload, max 16MB.

**Files:**
- Create: `adapters/jvm/mud-mop-jvm/src/main/kotlin/mud/mop/codec/MopCodec.kt`
- Create: `adapters/jvm/mud-mop-jvm/src/test/kotlin/mud/mop/codec/MopCodecTest.kt`

**Step 1: Write the failing test**

`MopCodecTest.kt`:
```kotlin
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
}
```

**Step 2: Run test to verify it fails**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.codec.MopCodecTest"`
Expected: FAIL — class not found

**Step 3: Write implementation**

`MopCodec.kt`:
```kotlin
package mud.mop.codec

import org.msgpack.core.MessagePack
import org.msgpack.core.MessageUnpacker
import org.msgpack.core.MessagePacker
import org.msgpack.value.Value
import org.msgpack.value.ValueFactory
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
```

**Step 4: Run test to verify it passes**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.codec.MopCodecTest"`
Expected: PASS

**Step 5: Commit**

```bash
git add adapters/jvm/mud-mop-jvm/src/
git commit -m "feat(jvm): implement MOP wire protocol codec"
```

---

## Task 3: MOP Client — Socket Connection & Message Loop

Implement the MOP client that connects via Unix socket, sends/receives messages,
and handles the request/response correlation pattern.

**Files:**
- Create: `adapters/jvm/mud-mop-jvm/src/main/kotlin/mud/mop/client/MopClient.kt`
- Create: `adapters/jvm/mud-mop-jvm/src/test/kotlin/mud/mop/client/MopClientTest.kt`

**Step 1: Write the failing test**

`MopClientTest.kt`:
```kotlin
package mud.mop.client

import mud.mop.codec.MopCodec
import java.io.PipedInputStream
import java.io.PipedOutputStream
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout

class MopClientTest {
    @Test
    fun `sends handshake on connect`() = runTest {
        val clientOut = PipedOutputStream()
        val serverIn = PipedInputStream(clientOut)
        val serverOut = PipedOutputStream()
        val clientIn = PipedInputStream(serverOut)

        val client = MopClient(
            input = clientIn,
            output = clientOut,
            adapterName = "test-adapter",
            language = "kotlin",
            version = "0.1.0"
        )

        launch { client.sendHandshake() }

        withTimeout(1000) {
            val msg = MopCodec.readFrame(serverIn)
            assertEquals("handshake", msg["type"])
            assertEquals("test-adapter", msg["adapter_name"])
            assertEquals("kotlin", msg["language"])
        }
    }

    @Test
    fun `send and receive driver request correlates by request_id`() = runTest {
        val clientOut = PipedOutputStream()
        val serverIn = PipedInputStream(clientOut)
        val serverOut = PipedOutputStream()
        val clientIn = PipedInputStream(serverOut)

        val client = MopClient(
            input = clientIn,
            output = clientOut,
            adapterName = "test",
            language = "kotlin",
            version = "0.1.0"
        )

        // Start read loop in background
        val readJob = launch { client.readLoop() }

        // Send a driver request in background, capture result
        val resultDeferred = launch {
            val result = client.sendDriverRequest("set_area_template", mapOf("files" to mapOf<String, String>()))
            assertEquals(true, result)
        }

        // Read the request from server side
        withTimeout(1000) {
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
        }

        withTimeout(1000) { resultDeferred.join() }
        readJob.cancel()
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.client.MopClientTest"`
Expected: FAIL — class not found

**Step 3: Write implementation**

`MopClient.kt`:
```kotlin
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
```

**Step 4: Run tests to verify they pass**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.client.MopClientTest"`
Expected: PASS

**Step 5: Commit**

```bash
git add adapters/jvm/mud-mop-jvm/src/
git commit -m "feat(jvm): implement MOP client with request/response correlation"
```

---

## Task 4: SLF4J MOP Log Appender

Implement a custom SLF4J appender that forwards log messages via MOP `Log` messages.

**Files:**
- Create: `adapters/jvm/mud-mop-jvm/src/main/kotlin/mud/mop/logging/MopLogAppender.kt`
- Create: `adapters/jvm/mud-mop-jvm/src/test/kotlin/mud/mop/logging/MopLogAppenderTest.kt`

**Step 1: Write the failing test**

`MopLogAppenderTest.kt`:
```kotlin
package mud.mop.logging

import kotlin.test.Test
import kotlin.test.assertEquals

class MopLogAppenderTest {
    @Test
    fun `captures log events and produces MOP messages`() {
        val captured = mutableListOf<Map<String, Any?>>()
        val appender = MopLogAppender(areaKey = "test/village") { msg -> captured.add(msg) }

        appender.log("info", "area loaded successfully")
        appender.log("error", "something went wrong")

        assertEquals(2, captured.size)
        assertEquals("log", captured[0]["type"])
        assertEquals("info", captured[0]["level"])
        assertEquals("area loaded successfully", captured[0]["message"])
        assertEquals("test/village", captured[0]["area"])
        assertEquals("error", captured[1]["level"])
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.logging.MopLogAppenderTest"`
Expected: FAIL — class not found

**Step 3: Write implementation**

`MopLogAppender.kt`:
```kotlin
package mud.mop.logging

import ch.qos.logback.classic.spi.ILoggingEvent
import ch.qos.logback.core.AppenderBase
import ch.qos.logback.classic.Level

class MopLogAppender(
    private val areaKey: String? = null,
    private val sender: (Map<String, Any?>) -> Unit,
) : AppenderBase<ILoggingEvent>() {

    override fun append(event: ILoggingEvent) {
        val level = when (event.level) {
            Level.ERROR -> "error"
            Level.WARN -> "warn"
            Level.INFO -> "info"
            Level.DEBUG -> "debug"
            Level.TRACE -> "trace"
            else -> "info"
        }
        log(level, event.formattedMessage)
    }

    fun log(level: String, message: String) {
        sender(mapOf(
            "type" to "log",
            "level" to level,
            "message" to message,
            "area" to areaKey,
        ))
    }
}
```

Note: The Logback dependency is already included transitively via `logback-classic` in the launcher.
The `mud-mop-jvm` module only depends on `slf4j-api`. Add the Logback dependency to `mud-mop-jvm/build.gradle.kts`:

```kotlin
implementation("ch.qos.logback:logback-classic:1.5.12")
```

**Step 4: Run test to verify it passes**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.logging.MopLogAppenderTest"`
Expected: PASS

**Step 5: Commit**

```bash
git add adapters/jvm/mud-mop-jvm/src/
git commit -m "feat(jvm): add SLF4J MOP log appender"
```

---

## Task 5: Stdlib Base Classes & Annotations

Implement the game object base classes (Room, Item, NPC, Daemon, Area) and
discovery annotations.

**Files:**
- Create: `adapters/jvm/stdlib/src/main/kotlin/mud/stdlib/annotations/MudAnnotations.kt`
- Create: `adapters/jvm/stdlib/src/main/kotlin/mud/stdlib/world/GameObject.kt`
- Create: `adapters/jvm/stdlib/src/main/kotlin/mud/stdlib/world/Room.kt`
- Create: `adapters/jvm/stdlib/src/main/kotlin/mud/stdlib/world/Item.kt`
- Create: `adapters/jvm/stdlib/src/main/kotlin/mud/stdlib/world/NPC.kt`
- Create: `adapters/jvm/stdlib/src/main/kotlin/mud/stdlib/world/Daemon.kt`
- Create: `adapters/jvm/stdlib/src/main/kotlin/mud/stdlib/world/Area.kt`
- Create: `adapters/jvm/stdlib/src/test/kotlin/mud/stdlib/world/AreaTest.kt`

**Step 1: Write the failing test**

`AreaTest.kt`:
```kotlin
package mud.stdlib.world

import mud.stdlib.annotations.*
import kotlin.test.Test
import kotlin.test.assertEquals

@MudRoom
class TestRoom : Room() {
    override val name = "Test Room"
    override val description = "A test room."
}

@MudArea(webMode = WebMode.TEMPLATE)
class TestArea : Area() {
    @WebData
    fun data(): Map<String, Any> = mapOf(
        "room_count" to rooms.size,
        "area_name" to name
    )
}

class AreaTest {
    @Test
    fun `area registers rooms`() {
        val area = TestArea()
        area.name = "village"
        area.registerRoom("entrance", TestRoom())
        assertEquals(1, area.rooms.size)
        assertEquals("Test Room", area.rooms["entrance"]?.name)
    }

    @Test
    fun `WebData annotation is discoverable`() {
        val area = TestArea()
        area.name = "village"
        val method = area::class.java.methods.find {
            it.isAnnotationPresent(WebData::class.java)
        }
        assertEquals("data", method?.name)
        @Suppress("UNCHECKED_CAST")
        val result = method?.invoke(area) as Map<String, Any>
        assertEquals("village", result["area_name"])
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd adapters/jvm && ./gradlew :stdlib:test --tests "mud.stdlib.world.AreaTest"`
Expected: FAIL — classes not found

**Step 3: Write annotations**

`MudAnnotations.kt`:
```kotlin
package mud.stdlib.annotations

@Target(AnnotationTarget.CLASS)
@Retention(AnnotationRetention.RUNTIME)
annotation class MudRoom

@Target(AnnotationTarget.CLASS)
@Retention(AnnotationRetention.RUNTIME)
annotation class MudNPC

@Target(AnnotationTarget.CLASS)
@Retention(AnnotationRetention.RUNTIME)
annotation class MudItem

@Target(AnnotationTarget.CLASS)
@Retention(AnnotationRetention.RUNTIME)
annotation class MudDaemon

@Target(AnnotationTarget.CLASS)
@Retention(AnnotationRetention.RUNTIME)
annotation class MudArea(val webMode: WebMode = WebMode.TEMPLATE)

@Target(AnnotationTarget.FUNCTION)
@Retention(AnnotationRetention.RUNTIME)
annotation class WebData

enum class WebMode {
    TEMPLATE, SPA, STATIC
}
```

**Step 4: Write base classes**

`GameObject.kt`:
```kotlin
package mud.stdlib.world

open class GameObject {
    open val name: String = ""
    open val description: String = ""
}
```

`Room.kt`:
```kotlin
package mud.stdlib.world

open class Room : GameObject() {
    private val exits = mutableMapOf<String, String>()

    fun exit(direction: String, to: String) {
        exits[direction] = to
    }

    fun exits(): Map<String, String> = exits
    fun hasExit(direction: String): Boolean = direction in exits

    open fun onEnter(player: String) {}
}
```

`Item.kt`:
```kotlin
package mud.stdlib.world

open class Item : GameObject() {
    open val portable: Boolean = false

    open fun onUse(player: String, target: String?) {}
}
```

`NPC.kt`:
```kotlin
package mud.stdlib.world

open class NPC : GameObject() {
    open val location: String? = null

    open fun onTalk(player: String) {}
}
```

`Daemon.kt`:
```kotlin
package mud.stdlib.world

open class Daemon : GameObject() {
    open fun tick() {}
}
```

`Area.kt`:
```kotlin
package mud.stdlib.world

open class Area {
    var name: String = ""
    var namespace: String = ""
    var path: String = ""

    val rooms = mutableMapOf<String, Room>()
    val items = mutableMapOf<String, Item>()
    val npcs = mutableMapOf<String, NPC>()
    val daemons = mutableMapOf<String, Daemon>()

    fun registerRoom(key: String, room: Room) { rooms[key] = room }
    fun registerItem(key: String, item: Item) { items[key] = item }
    fun registerNPC(key: String, npc: NPC) { npcs[key] = npc }
    fun registerDaemon(key: String, daemon: Daemon) { daemons[key] = daemon }
}
```

**Step 5: Run tests to verify they pass**

Run: `cd adapters/jvm && ./gradlew :stdlib:test --tests "mud.stdlib.world.AreaTest"`
Expected: PASS

**Step 6: Commit**

```bash
git add adapters/jvm/stdlib/src/
git commit -m "feat(jvm): add stdlib base classes and discovery annotations"
```

---

## Task 6: Classpath Scanner — Area Runtime

Implement the classpath scanner that discovers `@MudRoom`, `@MudNPC`, etc.
annotated classes and instantiates them into an `Area`.

**Files:**
- Create: `adapters/jvm/mud-mop-jvm/src/main/kotlin/mud/mop/runtime/AreaRuntime.kt`
- Create: `adapters/jvm/mud-mop-jvm/src/test/kotlin/mud/mop/runtime/AreaRuntimeTest.kt`

Add ClassGraph dependency to `mud-mop-jvm/build.gradle.kts`:
```kotlin
implementation("io.github.classgraph:classgraph:4.8.179")
```

**Step 1: Write the failing test**

`AreaRuntimeTest.kt`:
```kotlin
package mud.mop.runtime

import mud.stdlib.annotations.*
import mud.stdlib.world.*
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull

// Test classes — these live in the test classpath so ClassGraph can find them.
@MudRoom
class ScanTestRoom : Room() {
    override val name = "Scanned Room"
    override val description = "Found by scanner."
}

@MudNPC
class ScanTestNPC : NPC() {
    override val name = "Scanned NPC"
}

@MudArea(webMode = WebMode.TEMPLATE)
class ScanTestArea : Area() {
    @WebData
    fun data(): Map<String, Any> = mapOf("room_count" to rooms.size)
}

class AreaRuntimeTest {
    @Test
    fun `scans and populates area from annotated classes`() {
        val runtime = AreaRuntime("mud.mop.runtime")
        val area = runtime.area
        assertNotNull(area)
        assertEquals(WebMode.TEMPLATE, runtime.webMode)
        // Room should be registered
        assertEquals(1, area.rooms.size)
        assertEquals("Scanned Room", area.rooms.values.first().name)
        // NPC should be registered
        assertEquals(1, area.npcs.size)
    }

    @Test
    fun `invokes WebData method`() {
        val runtime = AreaRuntime("mud.mop.runtime")
        runtime.area.registerRoom("extra", ScanTestRoom())
        val data = runtime.getWebData()
        assertNotNull(data)
        assertEquals(2, data["room_count"]) // original + extra
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.runtime.AreaRuntimeTest"`
Expected: FAIL — class not found

**Step 3: Write implementation**

`AreaRuntime.kt`:
```kotlin
package mud.mop.runtime

import io.github.classgraph.ClassGraph
import mud.stdlib.annotations.*
import mud.stdlib.world.*
import org.slf4j.LoggerFactory
import java.lang.reflect.Method

class AreaRuntime(scanPackage: String) {
    private val logger = LoggerFactory.getLogger(AreaRuntime::class.java)

    val area: Area
    val webMode: WebMode
    private val webDataMethod: Method?
    private val webDataTarget: Any?

    init {
        var foundArea: Area? = null
        var mode = WebMode.TEMPLATE
        var dataMethod: Method? = null
        var dataTarget: Any? = null

        val scanResult = ClassGraph()
            .enableAnnotationInfo()
            .acceptPackages(scanPackage)
            .scan()

        // Find @MudArea class
        for (classInfo in scanResult.getClassesWithAnnotation(MudArea::class.java)) {
            val clazz = classInfo.loadClass()
            val annotation = clazz.getAnnotation(MudArea::class.java)
            val instance = clazz.getDeclaredConstructor().newInstance()
            if (instance is Area) {
                foundArea = instance
                mode = annotation.webMode
                // Find @WebData method
                for (m in clazz.methods) {
                    if (m.isAnnotationPresent(WebData::class.java)) {
                        dataMethod = m
                        dataTarget = instance
                        break
                    }
                }
            }
            break // only one @MudArea per area
        }

        area = foundArea ?: Area()
        webMode = mode
        webDataMethod = dataMethod
        webDataTarget = dataTarget

        // Scan and register rooms
        for (classInfo in scanResult.getClassesWithAnnotation(MudRoom::class.java)) {
            val clazz = classInfo.loadClass()
            val instance = clazz.getDeclaredConstructor().newInstance()
            if (instance is Room) {
                val key = classInfo.simpleName.replaceFirstChar { it.lowercase() }
                area.registerRoom(key, instance)
                logger.info("Registered room: {}", key)
            }
        }

        // Scan and register items
        for (classInfo in scanResult.getClassesWithAnnotation(MudItem::class.java)) {
            val clazz = classInfo.loadClass()
            val instance = clazz.getDeclaredConstructor().newInstance()
            if (instance is Item) {
                val key = classInfo.simpleName.replaceFirstChar { it.lowercase() }
                area.registerItem(key, instance)
                logger.info("Registered item: {}", key)
            }
        }

        // Scan and register NPCs
        for (classInfo in scanResult.getClassesWithAnnotation(MudNPC::class.java)) {
            val clazz = classInfo.loadClass()
            val instance = clazz.getDeclaredConstructor().newInstance()
            if (instance is NPC) {
                val key = classInfo.simpleName.replaceFirstChar { it.lowercase() }
                area.registerNPC(key, instance)
                logger.info("Registered NPC: {}", key)
            }
        }

        // Scan and register daemons
        for (classInfo in scanResult.getClassesWithAnnotation(MudDaemon::class.java)) {
            val clazz = classInfo.loadClass()
            val instance = clazz.getDeclaredConstructor().newInstance()
            if (instance is Daemon) {
                val key = classInfo.simpleName.replaceFirstChar { it.lowercase() }
                area.registerDaemon(key, instance)
                logger.info("Registered daemon: {}", key)
            }
        }

        scanResult.close()
    }

    @Suppress("UNCHECKED_CAST")
    fun getWebData(): Map<String, Any>? {
        val method = webDataMethod ?: return null
        val target = webDataTarget ?: return null
        return try {
            method.invoke(target) as? Map<String, Any>
        } catch (e: Exception) {
            logger.error("Failed to invoke @WebData method", e)
            null
        }
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.runtime.AreaRuntimeTest"`
Expected: PASS

**Step 5: Commit**

```bash
git add adapters/jvm/mud-mop-jvm/src/
git commit -m "feat(jvm): add classpath scanner for annotated game objects"
```

---

## Task 7: Flyway Migration Runner

Integrate Flyway to run database migrations from `db/migrations/` on area load.

**Files:**
- Create: `adapters/jvm/mud-mop-jvm/src/main/kotlin/mud/mop/migrations/FlywayRunner.kt`
- Create: `adapters/jvm/mud-mop-jvm/src/test/kotlin/mud/mop/migrations/FlywayRunnerTest.kt`

Add Flyway dependency to `mud-mop-jvm/build.gradle.kts`:
```kotlin
implementation("org.flywaydb:flyway-core:10.22.0")
implementation("org.flywaydb:flyway-database-postgresql:10.22.0")
implementation("org.postgresql:postgresql:42.7.4")
```

**Step 1: Write the failing test**

`FlywayRunnerTest.kt` — tests the runner initializes correctly without a live DB:
```kotlin
package mud.mop.migrations

import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue
import java.io.File

class FlywayRunnerTest {
    @Test
    fun `detects migration directory exists`() {
        val tempDir = File(System.getProperty("java.io.tmpdir"), "mud-flyway-test-${System.nanoTime()}")
        val migDir = File(tempDir, "db/migrations")
        migDir.mkdirs()
        File(migDir, "V1__init.sql").writeText("CREATE TABLE test (id INT);")

        try {
            assertTrue(FlywayRunner.hasMigrations(tempDir.absolutePath))
        } finally {
            tempDir.deleteRecursively()
        }
    }

    @Test
    fun `returns false when no migration directory`() {
        val tempDir = File(System.getProperty("java.io.tmpdir"), "mud-flyway-test-empty-${System.nanoTime()}")
        tempDir.mkdirs()
        try {
            assertFalse(FlywayRunner.hasMigrations(tempDir.absolutePath))
        } finally {
            tempDir.deleteRecursively()
        }
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.migrations.FlywayRunnerTest"`
Expected: FAIL — class not found

**Step 3: Write implementation**

`FlywayRunner.kt`:
```kotlin
package mud.mop.migrations

import org.flywaydb.core.Flyway
import org.slf4j.LoggerFactory
import java.io.File

object FlywayRunner {
    private val logger = LoggerFactory.getLogger(FlywayRunner::class.java)

    fun hasMigrations(areaPath: String): Boolean {
        val migDir = File(areaPath, "db/migrations")
        return migDir.isDirectory && migDir.listFiles()?.any { it.name.endsWith(".sql") } == true
    }

    fun run(areaPath: String, dbUrl: String): Int {
        val migDir = File(areaPath, "db/migrations")
        if (!migDir.isDirectory) {
            logger.debug("No migration directory at {}", migDir)
            return 0
        }

        val flyway = Flyway.configure()
            .dataSource(dbUrl, null, null)
            .locations("filesystem:${migDir.absolutePath}")
            .load()

        val result = flyway.migrate()
        logger.info("Ran {} migration(s) for area at {}", result.migrationsExecuted, areaPath)
        return result.migrationsExecuted
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.migrations.FlywayRunnerTest"`
Expected: PASS

**Step 5: Commit**

```bash
git add adapters/jvm/mud-mop-jvm/src/
git commit -m "feat(jvm): add Flyway migration runner for area databases"
```

---

## Task 8: Area Child Process — Entry Point

Implement the child JVM process that runs a single area: connects to the launcher
via MOP, scans classes, runs migrations, and handles messages.

**Files:**
- Create: `adapters/jvm/mud-mop-jvm/src/main/kotlin/mud/mop/runtime/AreaProcess.kt`
- Create: `adapters/jvm/mud-mop-jvm/src/test/kotlin/mud/mop/runtime/AreaProcessTest.kt`

**Step 1: Write the failing test**

`AreaProcessTest.kt`:
```kotlin
package mud.mop.runtime

import mud.mop.codec.MopCodec
import java.io.PipedInputStream
import java.io.PipedOutputStream
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeout

class AreaProcessTest {
    @Test
    fun `responds to ping with pong`() = runTest {
        val processOut = PipedOutputStream()
        val launcherIn = PipedInputStream(processOut)
        val launcherOut = PipedOutputStream()
        val processIn = PipedInputStream(launcherOut)

        val process = AreaProcess(
            input = processIn,
            output = processOut,
            areaId = mapOf("namespace" to "test", "name" to "village"),
            areaPath = "/tmp/nonexistent",
            dbUrl = null,
            scanPackage = "mud.mop.runtime", // reuse test classes from Task 6
        )

        val job = launch { process.run() }

        // Send ping
        MopCodec.writeFrame(launcherOut, mapOf("type" to "ping", "seq" to 42L))

        // Read pong
        withTimeout(1000) {
            val resp = MopCodec.readFrame(launcherIn)
            assertEquals("pong", resp["type"])
            assertEquals(42L, (resp["seq"] as Number).toLong())
        }

        job.cancel()
    }

    @Test
    fun `responds to get_web_data with template data`() = runTest {
        val processOut = PipedOutputStream()
        val launcherIn = PipedInputStream(processOut)
        val launcherOut = PipedOutputStream()
        val processIn = PipedInputStream(launcherOut)

        val process = AreaProcess(
            input = processIn,
            output = processOut,
            areaId = mapOf("namespace" to "test", "name" to "village"),
            areaPath = "/tmp/nonexistent",
            dbUrl = null,
            scanPackage = "mud.mop.runtime",
        )

        val job = launch { process.run() }

        // Send get_web_data request
        MopCodec.writeFrame(launcherOut, mapOf(
            "type" to "get_web_data",
            "request_id" to 99L,
            "area_key" to "test/village",
        ))

        withTimeout(1000) {
            val resp = MopCodec.readFrame(launcherIn)
            assertEquals("call_result", resp["type"])
            assertEquals(99L, (resp["request_id"] as Number).toLong())
        }

        job.cancel()
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.runtime.AreaProcessTest"`
Expected: FAIL — class not found

**Step 3: Write implementation**

`AreaProcess.kt`:
```kotlin
package mud.mop.runtime

import mud.mop.codec.MopCodec
import mud.mop.logging.MopLogAppender
import mud.mop.migrations.FlywayRunner
import org.slf4j.LoggerFactory
import java.io.InputStream
import java.io.OutputStream

class AreaProcess(
    private val input: InputStream,
    private val output: OutputStream,
    private val areaId: Map<String, String>,
    private val areaPath: String,
    private val dbUrl: String?,
    private val scanPackage: String,
) {
    private val logger = LoggerFactory.getLogger(AreaProcess::class.java)
    private val writeLock = Any()
    private lateinit var runtime: AreaRuntime
    private val areaKey = "${areaId["namespace"]}/${areaId["name"]}"

    fun run() {
        logger.info("Starting area process for {}", areaKey)

        // Run migrations if db_url provided
        if (dbUrl != null && FlywayRunner.hasMigrations(areaPath)) {
            try {
                FlywayRunner.run(areaPath, dbUrl)
            } catch (e: Exception) {
                logger.error("Migration failed for {}", areaKey, e)
                sendMessage(mapOf(
                    "type" to "area_error",
                    "area_id" to areaId,
                    "error" to "migration failed: ${e.message}",
                ))
                return
            }
        }

        // Scan classpath for game objects
        try {
            runtime = AreaRuntime(scanPackage)
            runtime.area.name = areaId["name"] ?: ""
            runtime.area.namespace = areaId["namespace"] ?: ""
            runtime.area.path = areaPath
        } catch (e: Exception) {
            logger.error("Failed to initialize area {}", areaKey, e)
            sendMessage(mapOf(
                "type" to "area_error",
                "area_id" to areaId,
                "error" to "init failed: ${e.message}",
            ))
            return
        }

        // Notify area loaded
        sendMessage(mapOf(
            "type" to "area_loaded",
            "area_id" to areaId,
        ))

        // Message loop
        try {
            while (true) {
                val msg = MopCodec.readFrame(input)
                dispatch(msg)
            }
        } catch (_: MopCodec.ConnectionClosed) {
            logger.info("Connection closed for area {}", areaKey)
        } catch (e: Exception) {
            logger.error("Error in area {} message loop", areaKey, e)
        }
    }

    private fun dispatch(msg: Map<String, Any?>) {
        when (msg["type"]) {
            "ping" -> sendMessage(mapOf("type" to "pong", "seq" to msg["seq"]))

            "session_start" -> {
                logger.info("Session started: {}", msg["session_id"])
            }

            "session_input" -> {
                val sessionId = msg["session_id"]
                val line = msg["line"] as? String ?: ""
                handleInput(sessionId, line)
            }

            "session_end" -> {
                logger.info("Session ended: {}", msg["session_id"])
            }

            "get_web_data" -> {
                val requestId = msg["request_id"]
                val data = runtime.getWebData() ?: emptyMap()
                sendMessage(mapOf(
                    "type" to "call_result",
                    "request_id" to requestId,
                    "result" to data,
                ))
            }

            "check_builder_access" -> {
                val requestId = msg["request_id"]
                sendMessage(mapOf(
                    "type" to "call_result",
                    "request_id" to requestId,
                    "result" to mapOf("allowed" to true),
                ))
            }

            else -> logger.warn("Unhandled message type: {}", msg["type"])
        }
    }

    private fun handleInput(sessionId: Any?, line: String) {
        // Basic command handling — areas can override
        sendMessage(mapOf(
            "type" to "session_output",
            "session_id" to sessionId,
            "text" to "You said: $line\n",
        ))
    }

    private fun sendMessage(msg: Map<String, Any?>) {
        synchronized(writeLock) {
            MopCodec.writeFrame(output, msg)
        }
    }

    companion object {
        /**
         * Entry point for child JVM processes spawned by the launcher.
         * Expected env vars: MUD_SOCKET_PATH, MUD_AREA_NS, MUD_AREA_NAME,
         * MUD_AREA_PATH, MUD_DB_URL (optional), MUD_SCAN_PACKAGE
         */
        @JvmStatic
        fun main(args: Array<String>) {
            val socketPath = System.getenv("MUD_SOCKET_PATH")
                ?: throw IllegalArgumentException("MUD_SOCKET_PATH not set")
            val ns = System.getenv("MUD_AREA_NS")
                ?: throw IllegalArgumentException("MUD_AREA_NS not set")
            val name = System.getenv("MUD_AREA_NAME")
                ?: throw IllegalArgumentException("MUD_AREA_NAME not set")
            val path = System.getenv("MUD_AREA_PATH")
                ?: throw IllegalArgumentException("MUD_AREA_PATH not set")
            val dbUrl = System.getenv("MUD_DB_URL")
            val scanPackage = System.getenv("MUD_SCAN_PACKAGE") ?: ""

            val socket = org.newsclub.net.unix.AFUNIXSocket.newInstance()
            socket.connect(org.newsclub.net.unix.AFUNIXSocketAddress.of(java.io.File(socketPath)))

            val process = AreaProcess(
                input = socket.getInputStream(),
                output = socket.getOutputStream(),
                areaId = mapOf("namespace" to ns, "name" to name),
                areaPath = path,
                dbUrl = dbUrl,
                scanPackage = scanPackage,
            )
            process.run()
        }
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cd adapters/jvm && ./gradlew :mud-mop-jvm:test --tests "mud.mop.runtime.AreaProcessTest"`
Expected: PASS

**Step 5: Commit**

```bash
git add adapters/jvm/mud-mop-jvm/src/
git commit -m "feat(jvm): implement area child process with message dispatch"
```

---

## Task 9: Launcher — Async Gradle Builder with Build Logs

Implement the Gradle build manager that builds area projects asynchronously
and captures build logs via MOP. Builds must NOT block the launcher's message loop.

**Files:**
- Create: `adapters/jvm/launcher/src/main/kotlin/mud/launcher/GradleBuilder.kt`
- Create: `adapters/jvm/launcher/src/test/kotlin/mud/launcher/GradleBuilderTest.kt`

**Step 1: Write the failing test**

`GradleBuilderTest.kt`:
```kotlin
package mud.launcher

import kotlin.test.Test
import kotlin.test.assertTrue
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlinx.coroutines.test.runTest
import java.io.File

class GradleBuilderTest {
    @Test
    fun `detects gradle project`() {
        val tempDir = File(System.getProperty("java.io.tmpdir"), "mud-gradle-test-${System.nanoTime()}")
        tempDir.mkdirs()
        File(tempDir, "build.gradle.kts").writeText("plugins { kotlin(\"jvm\") }")
        try {
            assertTrue(GradleBuilder.isGradleProject(tempDir.absolutePath))
        } finally {
            tempDir.deleteRecursively()
        }
    }

    @Test
    fun `returns false for non-gradle directory`() {
        val tempDir = File(System.getProperty("java.io.tmpdir"), "mud-gradle-empty-${System.nanoTime()}")
        tempDir.mkdirs()
        try {
            assertFalse(GradleBuilder.isGradleProject(tempDir.absolutePath))
        } finally {
            tempDir.deleteRecursively()
        }
    }

    @Test
    fun `captures build output lines`() = runTest {
        val logs = mutableListOf<String>()
        val builder = GradleBuilder(onLog = { level, msg -> logs.add("[$level] $msg") })

        // Build a non-existent project — should fail but capture output
        val result = builder.build("/tmp/nonexistent-gradle-project-${System.nanoTime()}")
        assertFalse(result.success)
        assertTrue(logs.isNotEmpty())
        assertNotNull(result.error)
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd adapters/jvm && ./gradlew :launcher:test --tests "mud.launcher.GradleBuilderTest"`
Expected: FAIL — class not found

**Step 3: Write implementation**

`GradleBuilder.kt`:
```kotlin
package mud.launcher

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.slf4j.LoggerFactory
import java.io.BufferedReader
import java.io.File

data class BuildResult(
    val success: Boolean,
    val jarPath: String? = null,
    val error: String? = null,
)

class GradleBuilder(
    private val onLog: (level: String, message: String) -> Unit,
) {
    private val logger = LoggerFactory.getLogger(GradleBuilder::class.java)

    companion object {
        fun isGradleProject(path: String): Boolean {
            val dir = File(path)
            return dir.isDirectory && (
                File(dir, "build.gradle.kts").exists() ||
                File(dir, "build.gradle").exists()
            )
        }
    }

    suspend fun build(areaPath: String): BuildResult = withContext(Dispatchers.IO) {
        val dir = File(areaPath)
        if (!dir.isDirectory) {
            val msg = "Area path does not exist: $areaPath"
            onLog("error", msg)
            return@withContext BuildResult(success = false, error = msg)
        }

        // Determine gradle wrapper or system gradle
        val gradleCmd = if (File(dir, "gradlew").exists()) {
            File(dir, "gradlew").absolutePath
        } else {
            "gradle"
        }

        onLog("info", "Building area at $areaPath...")

        try {
            val process = ProcessBuilder(gradleCmd, "shadowJar", "--no-daemon", "-q")
                .directory(dir)
                .redirectErrorStream(true)
                .start()

            // Capture output line-by-line without blocking the coroutine caller
            val reader: BufferedReader = process.inputStream.bufferedReader()
            val outputLines = mutableListOf<String>()

            reader.forEachLine { line ->
                outputLines.add(line)
                onLog("info", line)
            }

            val exitCode = process.waitFor()

            if (exitCode != 0) {
                val errorMsg = "Gradle build failed with exit code $exitCode"
                onLog("error", errorMsg)
                return@withContext BuildResult(success = false, error = errorMsg)
            }

            // Find the built JAR
            val libsDir = File(dir, "build/libs")
            val jar = libsDir.listFiles()
                ?.filter { it.name.endsWith(".jar") }
                ?.maxByOrNull { it.lastModified() }

            if (jar == null) {
                val msg = "Build succeeded but no JAR found in ${libsDir.absolutePath}"
                onLog("warn", msg)
                return@withContext BuildResult(success = false, error = msg)
            }

            onLog("info", "Build complete: ${jar.absolutePath}")
            BuildResult(success = true, jarPath = jar.absolutePath)

        } catch (e: Exception) {
            val msg = "Build process error: ${e.message}"
            onLog("error", msg)
            logger.error("Gradle build failed for {}", areaPath, e)
            BuildResult(success = false, error = msg)
        }
    }
}
```

**Step 4: Run tests to verify they pass**

Run: `cd adapters/jvm && ./gradlew :launcher:test --tests "mud.launcher.GradleBuilderTest"`
Expected: PASS

**Step 5: Commit**

```bash
git add adapters/jvm/launcher/src/
git commit -m "feat(jvm): add async Gradle builder with log capture"
```

---

## Task 10: Launcher — Area Process Manager

Implement the manager that spawns/stops child JVM processes per area, connects
them via per-area Unix sockets, and handles the async build → spawn lifecycle.

**Files:**
- Create: `adapters/jvm/launcher/src/main/kotlin/mud/launcher/AreaProcessManager.kt`
- Create: `adapters/jvm/launcher/src/test/kotlin/mud/launcher/AreaProcessManagerTest.kt`

**Step 1: Write the failing test**

`AreaProcessManagerTest.kt`:
```kotlin
package mud.launcher

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class AreaProcessManagerTest {
    @Test
    fun `tracks area states through lifecycle`() {
        val manager = AreaProcessManager(
            onSendToDriver = {},
            onLog = { _, _ -> },
        )

        val areaId = mapOf("namespace" to "test", "name" to "village")
        val areaKey = "test/village"

        assertFalse(manager.hasArea(areaKey))

        manager.markBuilding(areaKey, areaId)
        assertTrue(manager.hasArea(areaKey))
        assertEquals(AreaState.BUILDING, manager.getState(areaKey))

        manager.markRunning(areaKey)
        assertEquals(AreaState.RUNNING, manager.getState(areaKey))

        manager.remove(areaKey)
        assertFalse(manager.hasArea(areaKey))
    }

    @Test
    fun `unload during build marks as pending_unload`() {
        val manager = AreaProcessManager(
            onSendToDriver = {},
            onLog = { _, _ -> },
        )

        val areaKey = "test/village"
        manager.markBuilding(areaKey, mapOf("namespace" to "test", "name" to "village"))
        manager.requestUnload(areaKey)
        assertEquals(AreaState.PENDING_UNLOAD, manager.getState(areaKey))
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd adapters/jvm && ./gradlew :launcher:test --tests "mud.launcher.AreaProcessManagerTest"`
Expected: FAIL — class not found

**Step 3: Write implementation**

`AreaProcessManager.kt`:
```kotlin
package mud.launcher

import kotlinx.coroutines.*
import mud.mop.codec.MopCodec
import org.slf4j.LoggerFactory
import java.io.File
import java.net.ServerSocket
import java.util.concurrent.ConcurrentHashMap

enum class AreaState {
    BUILDING,
    STARTING,
    RUNNING,
    PENDING_UNLOAD,
}

data class AreaEntry(
    val areaId: Map<String, String>,
    var state: AreaState,
    var process: Process? = null,
    var socketPath: String? = null,
    var outputStream: java.io.OutputStream? = null,
    var inputStream: java.io.InputStream? = null,
    var readJob: Job? = null,
    var stderrJob: Job? = null,
)

class AreaProcessManager(
    private val onSendToDriver: (Map<String, Any?>) -> Unit,
    private val onLog: (level: String, message: String) -> Unit,
) {
    private val logger = LoggerFactory.getLogger(AreaProcessManager::class.java)
    private val areas = ConcurrentHashMap<String, AreaEntry>()
    private val scope = CoroutineScope(Dispatchers.IO + SupervisorJob())
    private val gradleBuilder = GradleBuilder { level, msg -> onLog(level, msg) }

    fun hasArea(key: String): Boolean = key in areas
    fun getState(key: String): AreaState? = areas[key]?.state

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
        entry.process?.destroyForcibly()
        entry.socketPath?.let { File(it).delete() }
    }

    fun requestUnload(key: String) {
        val entry = areas[key] ?: return
        when (entry.state) {
            AreaState.BUILDING -> entry.state = AreaState.PENDING_UNLOAD
            AreaState.RUNNING, AreaState.STARTING -> {
                remove(key)
            }
            AreaState.PENDING_UNLOAD -> {} // already pending
        }
    }

    /**
     * Handle LoadArea: trigger async Gradle build, then spawn child JVM.
     * Does NOT block the caller — build runs on a coroutine.
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

        // Create per-area Unix socket
        val socketPath = "/tmp/mud-area-${areaId["namespace"]}-${areaId["name"]}.sock"
        File(socketPath).delete()

        entry.state = AreaState.STARTING
        entry.socketPath = socketPath

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

            // Capture stdout in background (before MOP connects)
            entry.stderrJob = scope.launch {
                process.inputStream.bufferedReader().forEachLine { line ->
                    onLog("info", "[$key stdout] $line")
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
```

**Step 4: Run tests to verify they pass**

Run: `cd adapters/jvm && ./gradlew :launcher:test --tests "mud.launcher.AreaProcessManagerTest"`
Expected: PASS

**Step 5: Commit**

```bash
git add adapters/jvm/launcher/src/
git commit -m "feat(jvm): add area process manager with async build lifecycle"
```

---

## Task 11: Launcher — MOP Router & Main Entry Point

Implement the launcher's main loop: connect to the driver, route messages by
area ID, and send area templates on startup.

**Files:**
- Create: `adapters/jvm/launcher/src/main/kotlin/mud/launcher/MopRouter.kt`
- Create: `adapters/jvm/launcher/src/main/kotlin/mud/launcher/Main.kt`
- Create: `adapters/jvm/launcher/src/test/kotlin/mud/launcher/MopRouterTest.kt`

**Step 1: Write the failing test**

`MopRouterTest.kt`:
```kotlin
package mud.launcher

import kotlin.test.Test
import kotlin.test.assertEquals

class MopRouterTest {
    @Test
    fun `extracts area key from load_area message`() {
        val msg = mapOf(
            "type" to "load_area",
            "area_id" to mapOf("namespace" to "alice", "name" to "tavern"),
            "path" to "/data/world/alice/tavern",
            "db_url" to null,
        )
        assertEquals("alice/tavern", MopRouter.extractAreaKey(msg))
    }

    @Test
    fun `extracts area key from session_input via session registry`() {
        val router = MopRouter()
        router.registerSession(42L, "alice/tavern")
        assertEquals("alice/tavern", router.sessionAreaKey(42L))
    }

    @Test
    fun `returns null for unknown session`() {
        val router = MopRouter()
        assertEquals(null, router.sessionAreaKey(999L))
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cd adapters/jvm && ./gradlew :launcher:test --tests "mud.launcher.MopRouterTest"`
Expected: FAIL — class not found

**Step 3: Write MopRouter**

`MopRouter.kt`:
```kotlin
package mud.launcher

import org.slf4j.LoggerFactory
import java.util.concurrent.ConcurrentHashMap

class MopRouter {
    private val logger = LoggerFactory.getLogger(MopRouter::class.java)
    private val sessionToArea = ConcurrentHashMap<Long, String>()

    companion object {
        fun extractAreaKey(msg: Map<String, Any?>): String? {
            @Suppress("UNCHECKED_CAST")
            val areaId = msg["area_id"] as? Map<String, Any?> ?: return null
            val ns = areaId["namespace"] as? String ?: return null
            val name = areaId["name"] as? String ?: return null
            return "$ns/$name"
        }
    }

    fun registerSession(sessionId: Long, areaKey: String) {
        sessionToArea[sessionId] = areaKey
    }

    fun unregisterSession(sessionId: Long) {
        sessionToArea.remove(sessionId)
    }

    fun sessionAreaKey(sessionId: Long): String? = sessionToArea[sessionId]
}
```

**Step 4: Write Main.kt**

`Main.kt`:
```kotlin
package mud.launcher

import mud.mop.client.MopClient
import mud.mop.codec.MopCodec
import org.slf4j.LoggerFactory
import java.io.File
import kotlinx.coroutines.*

private val logger = LoggerFactory.getLogger("mud.launcher.Main")

fun main(args: Array<String>) {
    val socketPath = parseSocketPath(args)

    logger.info("JVM Adapter Launcher starting, connecting to {}", socketPath)

    val client = MopClient.connect(
        socketPath = socketPath,
        adapterName = "mud-adapter-jvm",
        language = "kotlin",
        version = "0.1.0",
    )

    val router = MopRouter()
    val processManager = AreaProcessManager(
        onSendToDriver = { msg -> client.sendMessage(msg) },
        onLog = { level, message ->
            client.sendMessage(mapOf(
                "type" to "log",
                "level" to level,
                "message" to message,
                "area" to null,
            ))
        },
    )

    // Send handshake
    client.sendHandshake()
    logger.info("Handshake sent")

    // Send area templates in background
    client.onMessage = { msg -> dispatchDriverMessage(msg, router, processManager, client) }

    // Start read loop on background thread
    val readThread = Thread({ client.readLoop() }, "mop-read-loop")
    readThread.isDaemon = true
    readThread.start()

    // Send area templates (after read loop is running so responses can be received)
    runBlocking {
        sendAreaTemplates(client)
    }

    // Wait for read loop to finish (connection closed)
    readThread.join()

    processManager.shutdown()
    logger.info("Launcher shut down")
}

private fun dispatchDriverMessage(
    msg: Map<String, Any?>,
    router: MopRouter,
    processManager: AreaProcessManager,
    client: MopClient,
) {
    when (msg["type"]) {
        "load_area", "reload_area" -> {
            val areaKey = MopRouter.extractAreaKey(msg) ?: return
            @Suppress("UNCHECKED_CAST")
            val areaId = msg["area_id"] as Map<String, String>
            val path = msg["path"] as? String ?: return
            val dbUrl = msg["db_url"] as? String
            processManager.loadArea(areaId, path, dbUrl)
        }

        "unload_area" -> {
            val areaKey = MopRouter.extractAreaKey(msg) ?: return
            processManager.requestUnload(areaKey)
        }

        "session_start" -> {
            val sessionId = (msg["session_id"] as Number).toLong()
            // TODO: determine which area the session belongs to
            // For now, route based on a default or first loaded area
        }

        "session_input" -> {
            val sessionId = (msg["session_id"] as Number).toLong()
            val areaKey = router.sessionAreaKey(sessionId) ?: return
            processManager.routeToArea(areaKey, msg)
        }

        "session_end" -> {
            val sessionId = (msg["session_id"] as Number).toLong()
            val areaKey = router.sessionAreaKey(sessionId) ?: return
            processManager.routeToArea(areaKey, msg)
            router.unregisterSession(sessionId)
        }

        "ping" -> {
            client.sendMessage(mapOf("type" to "pong", "seq" to msg["seq"]))
        }

        "configure" -> {
            logger.info("Received configure message")
            // Store stdlib DB URL if needed
        }

        "get_web_data", "check_builder_access" -> {
            // Route to appropriate area
            val areaKey = msg["area_key"] as? String
            if (areaKey != null) {
                processManager.routeToArea(areaKey, msg)
            }
        }

        else -> {
            logger.warn("Unhandled driver message type: {}", msg["type"])
        }
    }
}

private suspend fun sendAreaTemplates(client: MopClient) {
    val templateDir = findTemplateDir() ?: run {
        logger.warn("No template directory found, skipping template registration")
        return
    }

    // Load base template files
    val baseDir = File(templateDir, "base")
    val overlaysDir = File(templateDir, "overlays")

    if (!baseDir.isDirectory) {
        logger.warn("No base template directory at {}", baseDir)
        return
    }

    // Collect overlay names
    val overlays = overlaysDir.listFiles()
        ?.filter { it.isDirectory }
        ?.map { it.name }
        ?: emptyList()

    if (overlays.isEmpty()) {
        // Send a single template from base
        val files = collectFiles(baseDir)
        sendTemplate(client, "kotlin:default", files)
        return
    }

    // Send one template per overlay (base + overlay merged)
    for (overlay in overlays) {
        val baseFiles = collectFiles(baseDir)
        val overlayFiles = collectFiles(File(overlaysDir, overlay))
        val merged = baseFiles + overlayFiles // overlay files override base
        sendTemplate(client, "kotlin:$overlay", merged)
    }
}

private suspend fun sendTemplate(client: MopClient, name: String, files: Map<String, String>) {
    try {
        client.sendDriverRequest("set_area_template", mapOf(
            "name" to name,
            "files" to files,
        ))
        logger.info("Sent area template '{}' ({} files)", name, files.size)
    } catch (e: Exception) {
        logger.error("Failed to send area template '{}'", name, e)
    }
}

private fun collectFiles(dir: File): Map<String, String> {
    if (!dir.isDirectory) return emptyMap()
    val files = mutableMapOf<String, String>()
    dir.walkTopDown()
        .filter { it.isFile }
        .forEach { file ->
            val rel = file.relativeTo(dir).path
            files[rel] = file.readText()
        }
    return files
}

private fun findTemplateDir(): File? {
    // Look relative to the launcher JAR or working directory
    val candidates = listOf(
        File("stdlib/templates/area"),
        File("adapters/jvm/stdlib/templates/area"),
        File(System.getProperty("mud.template.dir", "")),
    )
    return candidates.firstOrNull { it.isDirectory }
}

private fun parseSocketPath(args: Array<String>): String {
    val idx = args.indexOf("--socket")
    if (idx >= 0 && idx + 1 < args.size) return args[idx + 1]
    throw IllegalArgumentException("Usage: launcher --socket <path>")
}
```

**Step 5: Run tests to verify they pass**

Run: `cd adapters/jvm && ./gradlew :launcher:test --tests "mud.launcher.MopRouterTest"`
Expected: PASS

**Step 6: Commit**

```bash
git add adapters/jvm/launcher/src/
git commit -m "feat(jvm): implement launcher main loop and MOP router"
```

---

## Task 12: Area Template — Base + Overlays

Create the area template files in the stdlib, with a shared base and per-framework
overlays.

**Files:**
- Create: `adapters/jvm/stdlib/templates/area/base/mud.yaml`
- Create: `adapters/jvm/stdlib/templates/area/base/settings.gradle.kts`
- Create: `adapters/jvm/stdlib/templates/area/base/src/main/kotlin/MudArea.kt`
- Create: `adapters/jvm/stdlib/templates/area/base/src/main/kotlin/rooms/Entrance.kt`
- Create: `adapters/jvm/stdlib/templates/area/base/web/templates/index.html`
- Create: `adapters/jvm/stdlib/templates/area/base/db/migrations/.gitkeep`
- Create: `adapters/jvm/stdlib/templates/area/base/agents.md`
- Create: `adapters/jvm/stdlib/templates/area/overlays/ktor/build.gradle.kts`
- Create: `adapters/jvm/stdlib/templates/area/overlays/ktor/mud.yaml`
- Create: `adapters/jvm/stdlib/templates/area/overlays/spring-boot/build.gradle.kts`
- Create: `adapters/jvm/stdlib/templates/area/overlays/spring-boot/mud.yaml`
- Create: `adapters/jvm/stdlib/templates/area/overlays/quarkus/build.gradle.kts`
- Create: `adapters/jvm/stdlib/templates/area/overlays/quarkus/mud.yaml`

**Step 1: Create base template files**

`base/mud.yaml`:
```yaml
framework: none
web_mode: template
entry_class: MudArea
```

`base/settings.gradle.kts`:
```kotlin
rootProject.name = "{{area_name}}"
```

`base/src/main/kotlin/MudArea.kt`:
```kotlin
import mud.stdlib.annotations.*
import mud.stdlib.world.Area

@MudArea(webMode = WebMode.TEMPLATE)
class MudArea : Area() {

    @WebData
    fun templateData(): Map<String, Any> = mapOf(
        "area_name" to name,
        "namespace" to namespace,
        "room_count" to rooms.size,
        "item_count" to items.size,
        "npc_count" to npcs.size,
    )
}
```

`base/src/main/kotlin/rooms/Entrance.kt`:
```kotlin
import mud.stdlib.annotations.MudRoom
import mud.stdlib.world.Room

@MudRoom
class Entrance : Room() {
    override val name = "The Entrance"
    override val description = """
        You stand at the entrance of {{area_name}}.
        Stone walls rise around you, cool and damp to the touch.
        A passage leads north into the darkness.
    """.trimIndent()

    init {
        // exit("north", "rooms.hall")
    }
}
```

`base/web/templates/index.html`:
```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>{{ area_name }}</title>
</head>
<body>
  <h1>Welcome to {{ area_name }}</h1>
  <p>Part of the {{ namespace }} world.</p>
  <p>This area has {{ room_count }} rooms, {{ item_count }} items, and {{ npc_count }} NPCs.</p>
</body>
</html>
```

`base/agents.md`:
```markdown
# {{area_name}} — Area Development Guide

## Project Structure

This is a MUD area built with the JVM adapter. It uses annotations for
game object discovery and Gradle for building.

## Annotations

- `@MudRoom` — Marks a class as a room. Must extend `Room()`.
- `@MudNPC` — Marks a class as an NPC. Must extend `NPC()`.
- `@MudItem` — Marks a class as an item. Must extend `Item()`.
- `@MudDaemon` — Marks a class as a background daemon. Must extend `Daemon()`.
- `@MudArea` — Marks the area entry point (one per area). Must extend `Area()`.
- `@WebData` — Marks a method that returns `Map<String, Any>` for Tera templates.

## Base Classes

- `Room` — Has `name`, `description`, `exit(direction, to)`, `onEnter(player)`
- `Item` — Has `name`, `description`, `portable`, `onUse(player, target)`
- `NPC` — Has `name`, `description`, `location`, `onTalk(player)`
- `Daemon` — Has `name`, `tick()` called periodically
- `Area` — Has `rooms`, `items`, `npcs`, `daemons`, `name`, `namespace`

## Configuration (mud.yaml)

```yaml
framework: none | ktor | spring-boot | quarkus
web_mode: template | spa | static
entry_class: MudArea
```

## Web Modes

### Template Mode (default)
- Place HTML files in `web/templates/`
- Uses Tera template engine (Jinja2-like syntax)
- Data comes from `@WebData` method: `{{ variable_name }}`
- The driver renders templates server-side

### SPA Mode
- Place frontend source in `web/src/` with `package.json`
- Set `web_mode: spa` in `mud.yaml`
- Driver builds with Vite and serves from `dist/`
- Define API routes using your framework (Ktor/Spring Boot/Quarkus)

### Static Mode
- Place files in `web/`
- Served as-is by the driver

## Database

- Place Flyway migrations in `db/migrations/`
- Named `V1__description.sql`, `V2__next.sql`, etc.
- Migrations run automatically on area load
- Connection URL provided via `MUD_DB_URL` environment variable

## Building

Run `./gradlew build` or `./gradlew shadowJar` for a fat JAR.
The adapter builds automatically on area load and git push.
```

**Step 2: Create framework overlays**

`overlays/ktor/mud.yaml`:
```yaml
framework: ktor
web_mode: template
entry_class: MudArea
```

`overlays/ktor/build.gradle.kts`:
```kotlin
plugins {
    kotlin("jvm") version "2.1.0"
    id("com.github.johnrengelman.shadow") version "8.1.1"
}

repositories {
    mavenCentral()
}

dependencies {
    implementation("mud:mud-mop-jvm:0.1.0")
    implementation("mud:mud-stdlib:0.1.0")

    // Ktor
    implementation("io.ktor:ktor-server-core:3.0.3")
    implementation("io.ktor:ktor-server-netty:3.0.3")
    implementation("io.ktor:ktor-server-content-negotiation:3.0.3")
    implementation("io.ktor:ktor-serialization-jackson:3.0.3")
    implementation("io.ktor:ktor-server-websockets:3.0.3")

    // Database
    implementation("org.flywaydb:flyway-core:10.22.0")
    implementation("org.flywaydb:flyway-database-postgresql:10.22.0")
    implementation("org.postgresql:postgresql:42.7.4")

    // Logging
    implementation("ch.qos.logback:logback-classic:1.5.12")
}
```

`overlays/spring-boot/mud.yaml`:
```yaml
framework: spring-boot
web_mode: template
entry_class: MudArea
```

`overlays/spring-boot/build.gradle.kts`:
```kotlin
plugins {
    kotlin("jvm") version "2.1.0"
    kotlin("plugin.spring") version "2.1.0"
    id("org.springframework.boot") version "3.4.1"
    id("io.spring.dependency-management") version "1.1.7"
}

repositories {
    mavenCentral()
}

dependencies {
    implementation("mud:mud-mop-jvm:0.1.0")
    implementation("mud:mud-stdlib:0.1.0")

    // Spring Boot WebFlux
    implementation("org.springframework.boot:spring-boot-starter-webflux")
    implementation("com.fasterxml.jackson.module:jackson-module-kotlin")

    // Database
    implementation("org.flywaydb:flyway-core")
    implementation("org.flywaydb:flyway-database-postgresql")
    implementation("org.postgresql:postgresql")
}
```

`overlays/quarkus/mud.yaml`:
```yaml
framework: quarkus
web_mode: template
entry_class: MudArea
```

`overlays/quarkus/build.gradle.kts`:
```kotlin
plugins {
    kotlin("jvm") version "2.1.0"
    kotlin("plugin.allopen") version "2.1.0"
    id("io.quarkus") version "3.17.5"
}

repositories {
    mavenCentral()
    mavenLocal()
}

dependencies {
    implementation("mud:mud-mop-jvm:0.1.0")
    implementation("mud:mud-stdlib:0.1.0")

    // Quarkus
    implementation(enforcedPlatform("io.quarkus.platform:quarkus-bom:3.17.5"))
    implementation("io.quarkus:quarkus-kotlin")
    implementation("io.quarkus:quarkus-resteasy-reactive")
    implementation("io.quarkus:quarkus-resteasy-reactive-jackson")
    implementation("io.quarkus:quarkus-websockets")

    // Database
    implementation("io.quarkus:quarkus-flyway")
    implementation("io.quarkus:quarkus-jdbc-postgresql")
}

allOpen {
    annotation("jakarta.ws.rs.Path")
    annotation("jakarta.enterprise.context.ApplicationScoped")
}
```

**Step 3: Commit**

```bash
git add adapters/jvm/stdlib/templates/
git commit -m "feat(jvm): add area templates with base + framework overlays"
```

---

## Task 13: Driver-Side — Named Template Support

Modify the Rust driver to support multiple named templates from different adapters,
and pass a template name when creating repos.

**Files:**
- Modify: `crates/mud-driver/src/server.rs` — change `area_template` from `Option<HashMap>` to `HashMap<String, HashMap<String, String>>`, update `handle_set_area_template` to accept a `name` field, update `handle_repo_create` to accept a `template` field
- Modify: `crates/mud-driver/src/config.rs` — add `default_template` to `AdaptersConfig`
- Modify: `crates/mud-driver/src/git/repo_manager.rs` — no changes needed (already takes `Option<&HashMap>`)

**Step 1: Write the failing test**

Add to existing config tests in `crates/mud-driver/src/config.rs`:
```rust
#[test]
fn parses_default_template() {
    let yaml = r#"
adapters:
  default_template: "kotlin:ktor"
  ruby:
    enabled: true
"#;
    let cfg = Config::from_yaml(yaml).unwrap();
    assert_eq!(cfg.adapters.default_template.as_deref(), Some("kotlin:ktor"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mud-driver parses_default_template`
Expected: FAIL — field `default_template` does not exist

**Step 3: Update config.rs**

Add `default_template` to `AdaptersConfig`:
```rust
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AdaptersConfig {
    pub ruby: Option<RubyAdapterConfig>,
    pub jvm: Option<JvmAdapterConfig>,
    pub default_template: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct JvmAdapterConfig {
    pub enabled: bool,
    pub command: String,
    pub adapter_path: String,
}

impl Default for JvmAdapterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            command: "java".into(),
            adapter_path: "adapters/jvm/launcher/build/libs/launcher.jar".into(),
        }
    }
}
```

**Step 4: Update server.rs — template storage**

Change `area_template` field from:
```rust
area_template: Option<HashMap<String, String>>,
```
to:
```rust
area_templates: HashMap<String, HashMap<String, String>>,
```

Update initialization:
```rust
area_templates: HashMap::new(),
```

**Step 5: Update handle_set_area_template**

```rust
fn handle_set_area_template(
    &mut self,
    request_id: u64,
    params: Value,
) -> DriverMessage {
    let (name, files) = match &params {
        Value::Map(m) => {
            let name = match m.get("name") {
                Some(Value::String(s)) => s.clone(),
                _ => "default".to_string(),
            };
            match m.get("files") {
                Some(Value::Map(files_map)) => {
                    let mut template = HashMap::new();
                    for (path, content) in files_map {
                        if let Value::String(content_str) = content {
                            template.insert(path.clone(), content_str.clone());
                        }
                    }
                    (name, template)
                }
                _ => {
                    return DriverMessage::RequestError {
                        request_id,
                        error: "missing 'files' map parameter".into(),
                    };
                }
            }
        }
        _ => {
            return DriverMessage::RequestError {
                request_id,
                error: "params must be a map".into(),
            };
        }
    };

    let count = files.len();
    info!(name = %name, count, "Area template set");
    self.area_templates.insert(name, files);

    DriverMessage::RequestResponse {
        request_id,
        result: Value::Bool(true),
    }
}
```

**Step 6: Update handle_repo_create to accept template name**

In `handle_repo_create`, after extracting `seed`:
```rust
let template_name = match &params {
    Value::Map(m) => m
        .get("template")
        .and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .or_else(|| self.config.adapters.default_template.clone()),
    _ => self.config.adapters.default_template.clone(),
};

let template = template_name
    .as_ref()
    .and_then(|name| self.area_templates.get(name))
    .or_else(|| self.area_templates.values().next());
```

Replace `self.area_template.as_ref()` with `template` in the `create_repo` call.

**Step 7: Update the account creation code similarly**

In `handle_create_account`, update the `create_repo` call to use
`self.config.adapters.default_template` for template lookup.

**Step 8: Run tests**

Run: `cargo test -p mud-driver`
Expected: All tests pass

**Step 9: Commit**

```bash
git add crates/mud-driver/src/
git commit -m "feat(driver): support multiple named area templates"
```

---

## Task 14: Driver-Side — JVM Adapter Spawning

Add JVM adapter spawning to the adapter manager, mirroring the Ruby pattern.

**Files:**
- Modify: `crates/mud-driver/src/runtime/adapter_manager.rs` — add JVM spawn logic
- Modify: `crates/mud-driver/src/config.rs` — already done in Task 13

**Step 1: Update adapter_manager.rs**

In the `start` method, after the Ruby adapter spawn block:
```rust
if let Some(ref jvm) = config.adapters.jvm {
    if jvm.enabled {
        let socket_str = self.socket_path.to_string_lossy().to_string();
        let world_path = config.world.resolved_path();
        self.spawn_adapter(&jvm.command, &jvm.adapter_path, &socket_str, &world_path)?;
        info!("spawned JVM adapter process");
    }
}
```

Note: The `spawn_adapter` method already handles generic adapter spawning — it
passes `--socket <path>` and sets `MUD_WORLD_PATH`. The JVM launcher accepts
the same CLI pattern. If the `command` is "java", the invocation becomes:
`java -jar <adapter_path> --socket <socket_path>`.

However, `spawn_adapter` currently runs `Command::new(command).arg(adapter_path)`,
which works for `ruby adapters/ruby/bin/mud-adapter` but for Java needs
`java -jar adapters/jvm/launcher/build/libs/launcher.jar`. Update `spawn_adapter`
to handle the `-jar` flag when the command is "java":

```rust
fn spawn_adapter(
    &mut self,
    command: &str,
    adapter_path: &str,
    socket_path: &str,
    world_path: &std::path::Path,
) -> Result<()> {
    let mut cmd = Command::new(command);

    // For Java, pass -jar flag before the adapter path
    if command.contains("java") {
        cmd.arg("-jar");
    }

    let child = cmd
        .arg(adapter_path)
        .arg("--socket")
        .arg(socket_path)
        .env("MUD_WORLD_PATH", world_path)
        .kill_on_drop(true)
        .spawn()
        .with_context(|| {
            format!("spawning adapter: {command} {adapter_path} --socket {socket_path}")
        })?;

    self.processes.push(child);
    Ok(())
}
```

**Step 2: Run tests**

Run: `cargo test -p mud-driver`
Expected: All tests pass

**Step 3: Commit**

```bash
git add crates/mud-driver/src/
git commit -m "feat(driver): add JVM adapter spawning to adapter manager"
```

---

## Task 15: Driver-Side — Per-Area API Proxying

Add per-area API route proxying so that JVM areas in SPA mode can serve
`/project/<ns>/<area>/api/*` endpoints through their framework web server.

**Files:**
- Modify: `crates/mud-driver/src/web/project.rs` — add API route proxying
- Modify: `crates/mud-driver/src/server.rs` — track per-area web socket paths

This task requires the adapter to notify the driver of each area's API socket
path via a new driver request action `"register_area_web"`.

**Step 1: Add register_area_web handler to server.rs**

In `handle_driver_request`, add a new action:
```rust
"register_area_web" => {
    let area_key = get_string_param(&params, "area_key")
        .unwrap_or_default();
    let socket_path = get_string_param(&params, "socket_path")
        .unwrap_or_default();
    self.area_web_sockets.insert(area_key, socket_path);
    DriverMessage::RequestResponse {
        request_id,
        result: Value::Bool(true),
    }
}
```

Add field to Server:
```rust
area_web_sockets: HashMap<String, String>,
```

**Step 2: Add API proxy route in project.rs**

In the project router, before the static file fallback, check if the path
starts with `api/` and proxy to the area's registered web socket:

```rust
// If path starts with "api/" and area has a registered web socket, proxy
if sub_path.starts_with("api/") || sub_path == "api" {
    let area_key = format!("{}/{}", ns, area_name);
    if let Some(socket_path) = state.area_web_sockets.get(&area_key) {
        return proxy_to_unix_socket(socket_path, req).await;
    }
}
```

**Step 3: Run tests**

Run: `cargo test -p mud-driver`
Expected: All tests pass

**Step 4: Commit**

```bash
git add crates/mud-driver/src/
git commit -m "feat(driver): add per-area API proxy routing for SPA mode"
```

---

## Task 16: Integration — Config & Justfile

Add JVM adapter configuration and development recipes.

**Files:**
- Modify: `config/server.yml` (or template) — add JVM adapter section
- Modify: `justfile` — add JVM build and test recipes

**Step 1: Update server.yml template**

Add to the adapters section:
```yaml
adapters:
  default_template: null
  ruby:
    enabled: true
    command: "ruby"
    adapter_path: "adapters/ruby/bin/mud-adapter"
  jvm:
    enabled: false
    command: "java"
    adapter_path: "adapters/jvm/launcher/build/libs/launcher.jar"
```

**Step 2: Add justfile recipes**

```just
# Build JVM adapter
build-jvm:
    cd adapters/jvm && ./gradlew build

# Test JVM adapter
test-jvm:
    cd adapters/jvm && ./gradlew test

# Build JVM launcher fat JAR
build-jvm-jar:
    cd adapters/jvm && ./gradlew :launcher:shadowJar

# Clean JVM build artifacts
clean-jvm:
    cd adapters/jvm && ./gradlew clean
```

**Step 3: Run build to verify**

Run: `just build-jvm`
Expected: BUILD SUCCESSFUL

**Step 4: Commit**

```bash
git add config/ justfile
git commit -m "feat: add JVM adapter config and justfile recipes"
```

---

## Task 17: End-to-End Smoke Test

Create a basic integration test that verifies the launcher connects to a mock
driver socket, sends handshake + templates, and handles a ping/pong exchange.

**Files:**
- Create: `adapters/jvm/launcher/src/test/kotlin/mud/launcher/LauncherIntegrationTest.kt`

**Step 1: Write the test**

`LauncherIntegrationTest.kt`:
```kotlin
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
```

**Step 2: Run test**

Run: `cd adapters/jvm && ./gradlew :launcher:test --tests "mud.launcher.LauncherIntegrationTest"`
Expected: PASS

**Step 3: Commit**

```bash
git add adapters/jvm/launcher/src/test/
git commit -m "test(jvm): add launcher integration test with mock driver"
```

---

## Execution Notes

### Build log handling
- `GradleBuilder` (Task 9) captures build output line-by-line and forwards via the `onLog` callback
- The launcher (Task 11) wires `onLog` to send MOP `Log` messages to the driver
- Build runs on a coroutine (`Dispatchers.IO`) and never blocks the launcher's MOP message loop
- If `UnloadArea` arrives during a build, the area is marked `PENDING_UNLOAD` and cleaned up when the build completes

### Non-blocking build flow
```
Driver sends LoadArea
  → Launcher receives, calls processManager.loadArea()
    → Coroutine launched: gradleBuilder.build() runs async
      → Build output lines → onLog callback → MOP Log message → driver
    → On success: spawnChild() starts the area JVM process
    → On failure: sends AreaError to driver, cleans up
  → Launcher message loop continues immediately (not blocked)
```

### Template selection flow
```
User creates area (portal / CLI / in-game)
  → repo_create request with optional "template" field
  → Driver looks up template by name in area_templates HashMap
  → Falls back to config.adapters.default_template
  → Falls back to first registered template
  → Seeds repo with selected template files
```

### Task dependencies
- Tasks 1-8 are the core JVM adapter (can be built independently)
- Tasks 9-11 are the launcher (depend on tasks 2-3)
- Task 12 is the template (independent)
- Tasks 13-15 are driver-side changes (independent of JVM code)
- Task 16 ties it together
- Task 17 verifies the integration
