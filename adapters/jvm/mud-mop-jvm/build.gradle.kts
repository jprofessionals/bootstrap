plugins {
    kotlin("jvm")
    `maven-publish`
}

publishing {
    publications {
        create<MavenPublication>("maven") {
            from(components["java"])
            groupId = "mud"
            artifactId = "mud-mop-jvm"
        }
    }
}

dependencies {
    implementation("org.msgpack:msgpack-core:0.9.8")
    implementation("com.kohlschutter.junixsocket:junixsocket-core:2.10.1")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    implementation("org.slf4j:slf4j-api:2.0.16")
    implementation("ch.qos.logback:logback-classic:1.5.12")
    implementation("io.github.classgraph:classgraph:4.8.179")
    implementation("org.flywaydb:flyway-core:10.22.0")
    implementation("org.flywaydb:flyway-database-postgresql:10.22.0")
    implementation("org.postgresql:postgresql:42.7.4")

    implementation(project(":stdlib"))

    // Ktor is compileOnly — only available at runtime when the area JAR includes it
    compileOnly("io.ktor:ktor-server-core:3.0.3")
    compileOnly("io.ktor:ktor-server-netty:3.0.3")
    compileOnly("io.ktor:ktor-server-content-negotiation:3.0.3")
    compileOnly("io.ktor:ktor-serialization-jackson:3.0.3")

    testImplementation(kotlin("test"))
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.9.0")
}
