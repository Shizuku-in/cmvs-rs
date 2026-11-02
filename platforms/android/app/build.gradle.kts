import java.util.Properties

plugins {
    id("com.android.application")
}

val local = Properties().apply {
    val file = rootProject.file("local.properties")
    if (file.exists()) file.inputStream().use { load(it) }
}
val rustAbis = providers.gradleProperty("rustAbis").orElse("x86_64").get().split(",")
require(rustAbis.all { it in setOf("x86_64", "arm64-v8a") }) { "rustAbis supports x86_64 and arm64-v8a" }
val ndkRevision = providers.gradleProperty("ndkVersion").orElse("30.0.16248370").get()
val sdkPath = local.getProperty("sdk.dir") ?: System.getenv("ANDROID_HOME") ?: System.getenv("ANDROID_SDK_ROOT")
val cargoExecutable = providers.gradleProperty("cargoExecutable").orElse("cargo").get()
val cargoNdkDirectory = local.getProperty("cargo.ndk.dir")?.let { rootProject.file(it) }
abstract class BuildRust : Exec() {
    @get:OutputDirectory
    abstract val outputDirectory: DirectoryProperty
}

android {
    namespace = "org.cmvs.player"
    compileSdk = 35
    ndkVersion = ndkRevision
    defaultConfig {
        applicationId = "org.cmvs.player"
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0-preview"
        ndk { abiFilters += rustAbis }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
        }
    }
}

val buildRust = tasks.register<BuildRust>("buildRust") {
    outputDirectory.set(layout.buildDirectory.dir("generated/rustJniLibs"))
    workingDir = rootProject.projectDir.parentFile.parentFile
    doFirst {
        require(sdkPath != null) { "Set sdk.dir in local.properties or ANDROID_HOME" }
        val ndkPath = File(sdkPath, "ndk/$ndkRevision")
        require(ndkPath.isDirectory) { "Install NDK $ndkRevision or set -PndkVersion" }
        environment("ANDROID_NDK_HOME", ndkPath.absolutePath)
        if (cargoNdkDirectory != null) {
            environment("PATH", cargoNdkDirectory.absolutePath + File.pathSeparator + System.getenv("PATH"))
        }
    }
    val arguments = mutableListOf(cargoExecutable, "ndk")
    rustAbis.forEach { arguments += listOf("-t", it) }
    arguments += listOf("--platform", "29", "-o", outputDirectory.get().asFile.absolutePath,
        "build", "--locked", "--release", "-p", "cmvs-android")
    commandLine(arguments)
    // Always invoke Cargo so changes in shared workspace crates are observed;
    // Cargo itself handles incremental builds.
    outputs.upToDateWhen { false }
}
androidComponents.onVariants { variant ->
    variant.sources.jniLibs?.addGeneratedSourceDirectory(buildRust, BuildRust::outputDirectory)
}
