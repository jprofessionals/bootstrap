plugins {
    kotlin("jvm") version "2.1.20"
    id("com.gradleup.shadow") version "8.3.6"
}

repositories {
    mavenLocal()
    mavenCentral()
}

dependencies {
    implementation("mud:mud-mop-jvm:0.1.0")
    implementation("mud:mud-stdlib:0.1.0")

    // Database (optional — remove if not using migrations)
    implementation("org.flywaydb:flyway-core:10.22.0")
    implementation("org.flywaydb:flyway-database-postgresql:10.22.0")
    implementation("org.postgresql:postgresql:42.7.4")

    // Logging
    implementation("ch.qos.logback:logback-classic:1.5.12")
}

tasks.shadowJar {
    manifest { attributes("Main-Class" to "mud.mop.runtime.AreaProcess") }
    archiveClassifier.set("")
    mergeServiceFiles()
}
