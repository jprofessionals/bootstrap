plugins {
    kotlin("jvm")
    `maven-publish`
}

publishing {
    publications {
        create<MavenPublication>("maven") {
            from(components["java"])
            groupId = "mud"
            artifactId = "mud-stdlib"
        }
    }
}

dependencies {
    testImplementation(kotlin("test"))
}
