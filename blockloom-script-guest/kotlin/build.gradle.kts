plugins {
    kotlin("multiplatform") version "2.4.21"
}

repositories {
    mavenCentral()
}

kotlin {
    @OptIn(org.jetbrains.kotlin.gradle.ExperimentalWasmDsl::class)
    wasmWasi {
        nodejs()
        binaries.executable()
    }
}
