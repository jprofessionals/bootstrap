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

        // Build a non-existent project -- should fail but capture output
        val result = builder.build("/tmp/nonexistent-gradle-project-${System.nanoTime()}")
        assertFalse(result.success)
        assertTrue(logs.isNotEmpty())
        assertNotNull(result.error)
    }
}
