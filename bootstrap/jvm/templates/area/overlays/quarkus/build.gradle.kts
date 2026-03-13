plugins {
    kotlin("jvm") version "2.1.20"
    kotlin("plugin.allopen") version "2.1.20"
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
