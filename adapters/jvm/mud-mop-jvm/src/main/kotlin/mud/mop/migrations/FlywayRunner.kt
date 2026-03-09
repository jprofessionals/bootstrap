package mud.mop.migrations

import org.flywaydb.core.Flyway
import org.slf4j.LoggerFactory
import java.io.File
import java.net.URI

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

        val (jdbcUrl, user, password) = toJdbc(dbUrl)

        val flyway = Flyway.configure()
            .dataSource(jdbcUrl, user, password)
            .locations("filesystem:${migDir.absolutePath}")
            .load()

        val result = flyway.migrate()
        logger.info("Ran {} migration(s) for area at {}", result.migrationsExecuted, areaPath)
        return result.migrationsExecuted
    }

    /**
     * Convert a postgres:// or postgresql:// URL to JDBC format.
     * Input:  postgres://user:pass@host:port/db
     * Output: Triple(jdbc:postgresql://host:port/db, user, pass)
     */
    internal fun toJdbc(url: String): Triple<String, String?, String?> {
        if (url.startsWith("jdbc:")) return Triple(url, null, null)

        // Normalize scheme to parse with java.net.URI
        val normalized = url.replaceFirst(Regex("^postgres(ql)?://"), "http://")
        val uri = URI(normalized)
        val jdbcUrl = "jdbc:postgresql://${uri.host}:${uri.port}${uri.path}"
        val userInfo = uri.userInfo?.split(":", limit = 2)
        val user = userInfo?.getOrNull(0)
        val password = userInfo?.getOrNull(1)
        return Triple(jdbcUrl, user, password)
    }
}
