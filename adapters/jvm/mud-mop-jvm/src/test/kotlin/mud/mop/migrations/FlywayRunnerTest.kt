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
