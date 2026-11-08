// :ffi — the real core for Android: libdspi_ffi.so (cargo-ndk) + UniFFI Kotlin
// bindings (generated at build time) + FfiDspiCore (generated adapter,
// checked in). Owned by package FFI.
//
// The Rust steps need cargo/cargo-ndk/the NDK: run Gradle via tools/dev.sh.
plugins {
    id("dspi.android-library")
}

android {
    namespace = "se.bofr.dspi.ffi"
    defaultConfig {
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
        // -- WP AN15 -- keep rules for UniFFI/JNA, applied to the app's R8 run
        consumerProguardFiles("consumer-rules.pro")
    }
    // Filled per UniFFI namespace by the generateUniffiKotlin_* tasks below.
    sourceSets["main"].kotlin.srcDir(layout.buildDirectory.dir("generated/uniffi/dspi_core"))
    sourceSets["main"].kotlin.srcDir(layout.buildDirectory.dir("generated/uniffi/dspi_ffi"))
    // Per build type: debug -> rust/jniLibs-debug (profile from dspi.rustProfile,
    // default debug), release -> rust/jniLibs-release (ALWAYS the cargo release
    // profile, so a Play .aab can never ship an unoptimised .so).
    sourceSets["debug"].jniLibs.srcDir(layout.buildDirectory.dir("rust/jniLibs-debug"))
    sourceSets["release"].jniLibs.srcDir(layout.buildDirectory.dir("rust/jniLibs-release"))
}

dependencies {
    api(project(":core-api"))
    // UniFFI's Kotlin runtime needs JNA (the AAR carries libjnidispatch.so per ABI).
    implementation("${libs.jna.get().module}:${libs.versions.jna.get()}@aar")
    // The jar has the host's libjnidispatch, for FfiLiveDemoTest.
    testImplementation("${libs.jna.get().module}:${libs.versions.jna.get()}")
}

// The Rust side is the monorepo's one workspace (crates/, Cargo.toml at the top).
val workspaceDir = rootProject.layout.projectDirectory.dir("../..")
val rustProfile = providers.gradleProperty("dspi.rustProfile").getOrElse("debug")
val cargoTarget = providers.environmentVariable("CARGO_TARGET_DIR").getOrElse(workspaceDir.dir("target").asFile.path)

fun requireContainer() {
    check(System.getenv("DSPI_IN_CONTAINER") == "1") {
        "The :ffi Rust build runs inside the build container: use tools/dev.sh ./gradlew ..."
    }
}

val rustSources =
    fileTree(workspaceDir) {
        include(
            "Cargo.toml",
            "Cargo.lock",
            "crates/dspi-ffi/**",
            "crates/dspi-core/**",
            "crates/dspi-link/**",
            "crates/dspi-proto/**",
            "crates/dspi-session/**",
            "crates/dspi-transport/**"
        )
    }

val cargoBuildHost by tasks.registering(Exec::class) {
    description = "Build libdspi_ffi and uniffi-bindgen for the host (UniFFI reads the library's metadata)."
    workingDir = workspaceDir.asFile
    commandLine("cargo", "build", "-p", "dspi-ffi", "--features", "cli", "--locked")
    inputs.files(rustSources)
    outputs.files("$cargoTarget/debug/libdspi_ffi.so", "$cargoTarget/debug/uniffi-bindgen")
    doFirst { requireContainer() }
}

// One run per UniFFI namespace: each generates its own file in its own package
// (crates/<crate>/uniffi.toml). ci/compile-bindings.sh checks in CI that these
// are all the namespaces the library has.
val uniffiNamespaces = mapOf("dspi_core" to "crates/dspi-core/uniffi.toml", "dspi_ffi" to "crates/dspi-ffi/uniffi.toml")

val generateUniffiKotlin =
    uniffiNamespaces.map { (namespace, config) ->
        tasks.register<Exec>("generateUniffiKotlin_$namespace") {
            description = "Generate the UniFFI Kotlin bindings of namespace $namespace from the host library."
            dependsOn(cargoBuildHost)
            val out = layout.buildDirectory.dir("generated/uniffi/$namespace")
            workingDir = workspaceDir.asFile
            inputs.file("$cargoTarget/debug/libdspi_ffi.so")
            inputs.file(workspaceDir.file(config))
            outputs.dir(out)
            doFirst {
                requireContainer()
                delete(out)
            }
            commandLine(
                "$cargoTarget/debug/uniffi-bindgen",
                "generate", "--library", "$cargoTarget/debug/libdspi_ffi.so",
                "--crate", namespace, "--config", config,
                "--language", "kotlin", "--no-format",
                "--out-dir", out.get().asFile.path
            )
        }
    }

fun registerCargoNdkBuild(variant: String, profile: String) =
    tasks.register<Exec>("cargoNdkBuild${variant.replaceFirstChar { it.uppercase() }}") {
        description = "Cross-compile libdspi_ffi.so ($profile profile) for the $variant variant with cargo-ndk."
        val out = layout.buildDirectory.dir("rust/jniLibs-$variant")
        workingDir = workspaceDir.asFile
        inputs.files(rustSources)
        inputs.property("profile", profile)
        outputs.dir(out)
        doFirst {
            requireContainer()
            delete(out) // cargo-ndk only adds files; drop stale libraries
        }
        val profileArgs = if (profile == "release") listOf("--release") else emptyList()
        commandLine(
            listOf(
                "cargo", "ndk",
                "-t", "arm64-v8a", "-t", "armeabi-v7a", "-t", "x86_64",
                "-P", libs.versions.minSdk.get(),
                "-o", out.get().asFile.path,
                "build", "-p", "dspi-ffi", "--locked"
            ) + profileArgs
        )
    }

// dspi.rustProfile (gradle.properties) only picks the DEBUG variant's profile;
// the release variant is always built with --release.
val cargoNdkBuildDebug = registerCargoNdkBuild("debug", rustProfile)
val cargoNdkBuildRelease = registerCargoNdkBuild("release", "release")

tasks.named("preBuild") { dependsOn(generateUniffiKotlin) }
tasks.matching { it.name == "preDebugBuild" }.configureEach { dependsOn(cargoNdkBuildDebug) }
tasks.matching { it.name == "preReleaseBuild" }.configureEach { dependsOn(cargoNdkBuildRelease) }

// FfiLiveDemoTest calls the real library on the JVM, against the host build.
tasks.withType<Test>().configureEach {
    dependsOn(cargoBuildHost)
    systemProperty("jna.library.path", "$cargoTarget/debug")
    systemProperty("java.library.path", "$cargoTarget/debug")
}
