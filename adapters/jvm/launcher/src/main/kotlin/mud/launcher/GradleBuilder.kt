package mud.launcher

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.slf4j.LoggerFactory
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
            // Fall back to shared Gradle wrapper (set by adapter deployment)
            System.getenv("MUD_GRADLEW") ?: "gradle"
        }

        onLog("info", "Building area at $areaPath...")

        try {
            val process = ProcessBuilder(gradleCmd, "shadowJar", "--no-daemon", "-q")
                .directory(dir)
                .redirectErrorStream(true)
                .start()

            // Capture output line-by-line without blocking the coroutine caller
            val reader = process.inputStream.bufferedReader()
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
