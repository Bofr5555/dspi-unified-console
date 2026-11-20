// :app — wiring only (decision D9): NavHost over the feature entry points,
// the real stack (FfiDspiCore -> DspiBridgeImpl -> AndroidUsbSession), the
// explicit demo mode (FakeDspiCore.demo()), and the DataStore LinkStore.
// Owned by package APP (integration); packages do not edit it except through
// the route table in :ui-common.
import java.util.Properties

plugins {
    id("dspi.android-app")
}

// Release (Google Play) signing: the UPLOAD key, never committed. Default
// location keystore/upload.properties (+ the keystore file it names), both
// gitignored; created by tools/upload-key.sh. -Pdspi.uploadKey=<path> points
// at another properties file (tools/ci.sh release uses a throwaway key in a
// temp dir). Keys: storeFile (relative to the properties file), storePassword,
// keyAlias, keyPassword. See docs/RELEASE.md.
val uploadKeyProps: File =
    providers.gradleProperty("dspi.uploadKey").map { file(it) }
        .getOrElse(rootProject.file("keystore/upload.properties"))
val uploadKey: Properties? =
    uploadKeyProps.takeIf { it.isFile }?.let { f -> Properties().apply { f.inputStream().use { load(it) } } }

// The sideload APK (owner 2026-11-09): -Pdspi.sideload=true builds the same app
// as a separate one, se.bofr.dspi_console_android_apk named "uConsole for DSPi (APK)",
// so it installs next to the Play copy instead of colliding with it. Task names
// stay the same; without the property the build is the Play one. -- WP RN1 --
// Its release APK is the release file itself, uConsole-for-DSPi-<versionName>.apk
// (in app/build/outputs/apk/release/).
val sideload: Boolean = providers.gradleProperty("dspi.sideload").map { it.toBoolean() }.getOrElse(false)

android {
    namespace = "se.bofr.dspi.app"
    defaultConfig {
        applicationId = "se.bofr.dspi_console_android" + if (sideload) "_apk" else ""
        // The launcher label, which the About page shows as its title too. The Play
        // build keeps "DSPi Console" until its next release renames it (owner 2026-11-09)
        resValue("string", "app_name", if (sideload) "uConsole for DSPi (APK)" else "DSPi Console")
        // The ONE place for the app version (runtime reads PackageInfo).
        // Every upload to Play needs a higher versionCode than any before it.
        versionCode = 14
        versionName = "0.14.0"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64") }
    }
    // The container's home (and with it ~/.android/debug.keystore) is thrown
    // away after every run, so debug builds would get a new signature each
    // time and could not update an installed copy. Use a persistent,
    // gitignored project keystore when it exists (tools/debug-keystore.sh).
    val projectDebugKeystore = rootProject.file("keystore/debug.keystore")
    if (projectDebugKeystore.exists()) {
        signingConfigs.getByName("debug") {
            storeFile = projectDebugKeystore
            storePassword = "android"
            keyAlias = "androiddebugkey"
            keyPassword = "android"
        }
    }
    if (uploadKey != null) {
        signingConfigs.create("upload") {
            storeFile = uploadKeyProps.parentFile.resolve(uploadKey.getProperty("storeFile") ?: "upload.jks")
            storePassword = uploadKey.getProperty("storePassword")
            keyAlias = uploadKey.getProperty("keyAlias")
            keyPassword = uploadKey.getProperty("keyPassword")
        }
    }
    buildTypes {
        release {
            // -- WP AN15 -- R8: shrink, optimise and obfuscate the code, and drop
            // unused resources. UniFFI's Kotlin bindings reach libdspi_ffi through
            // JNA (reflection over Structure fields, Library and Callback
            // interfaces); their keep rules come with :ffi (ffi/consumer-rules.pro).
            // :ffi always links the Rust release profile here.
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("upload")
        }
    }
    // JVM tests touch android.util.Log / android.os only through code paths
    // they don't assert on; the stubs return defaults instead of throwing.
    testOptions {
        unitTests.isReturnDefaultValues = true
        // AN27: Robolectric reads the merged manifest and resources
        unitTests.isIncludeAndroidResources = true
    }
}

// -- WP RN1 -- the sideload release APK under the release's file name
if (sideload) {
    androidComponents {
        onVariants(selector().withBuildType("release")) { variant ->
            variant.outputs.forEach { output ->
                (output as com.android.build.api.variant.impl.VariantOutputImpl).outputFileName
                    .set(output.versionName.map { "uConsole-for-DSPi-$it.apk" })
            }
        }
    }
}

// AN27: the Android runtime Robolectric runs on. Left to itself it downloads this at
// test time, past dependency verification, so Gradle resolves it (checksum-pinned)
// and Robolectric stays offline.
val robolectricRuntime: Configuration by configurations.creating

dependencies {
    implementation(project(":bridge"))
    implementation(project(":ffi"))
    implementation(project(":usb"))
    implementation(project(":link"))
    implementation(project(":ui-common"))
    implementation(project(":feature-home"))
    implementation(project(":feature-graph"))
    implementation(project(":feature-matrix"))
    implementation(project(":feature-tools"))
    implementation(project(":feature-presets"))
    implementation(project(":feature-settings"))
    implementation(project(":feature-analysis"))
    implementation(project(":feature-firmware"))
    implementation(project(":feature-onboarding")) // -- WP AN19 --

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.navigation.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.androidx.datastore.preferences)

    // Demo mode runs on the real core (AN28); the Kotlin fake is for tests only
    testImplementation(project(":fake-core"))
    // AN28: the real core on the JVM, as :ffi's FfiLiveDemoTest (the host's libjnidispatch)
    testImplementation("${libs.jna.get().module}:${libs.versions.jna.get()}")
    testImplementation(libs.kotlinx.coroutines.test)
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)
    testImplementation(libs.robolectric)
    // Declares the Activity createComposeRule() starts; debug builds only
    debugImplementation(platform(libs.compose.bom))
    debugImplementation(libs.compose.ui.test.manifest)
    robolectricRuntime(libs.robolectric.android.all)
}

tasks.withType<Test>().configureEach {
    val runtime = robolectricRuntime
    inputs.files(runtime)
    systemProperty("robolectric.offline", "true")
    doFirst { systemProperty("robolectric.dependency.dir", runtime.singleFile.parentFile.path) }
    // AN28: demo mode on the JVM needs the host build of libdspi_ffi
    dependsOn(":ffi:cargoBuildHost")
    val workspaceTarget = rootProject.layout.projectDirectory.dir("../../target").asFile.path
    val cargoTarget = providers.environmentVariable("CARGO_TARGET_DIR").getOrElse(workspaceTarget)
    systemProperty("jna.library.path", "$cargoTarget/debug")
    systemProperty("java.library.path", "$cargoTarget/debug")
    // -- WP AN34 -- the graph overlay's following test runs crates/dspi-link's fake hub (as :feature-analysis' AN17 test)
    dependsOn(":link:cargoBuildFakeHub")
    systemProperty("dspi.fakehub", "$cargoTarget/debug/dspi-link-fakehub")
    // -- WP AN42 -- UndoBarShotsWpAn42Test writes its screenshots here when given -Pdspi.shotsDir (as UIM1's)
    providers.gradleProperty("dspi.shotsDir").orNull?.let { systemProperty("dspi.shotsDir", it) }
}

// Fail early and clearly instead of producing an unsigned / wrongly signed release.
val checkUploadKey by tasks.registering {
    description = "Fail the release build when the upload key (keystore/upload.properties) is missing."
    val props = uploadKeyProps
    val storeFile = uploadKey?.let { props.parentFile.resolve(it.getProperty("storeFile") ?: "upload.jks") }
    val missing =
        listOf("storePassword", "keyAlias", "keyPassword").filter { uploadKey?.getProperty(it).isNullOrEmpty() }
    doLast {
        if (!props.isFile) {
            throw GradleException(
                "Release signing: upload key properties not found at $props.\n" +
                    "Create the upload key with tools/upload-key.sh (see docs/RELEASE.md), " +
                    "or pass -Pdspi.uploadKey=<path/to/upload.properties>."
            )
        }
        if (missing.isNotEmpty()) throw GradleException("Release signing: $props lacks ${missing.joinToString()}.")
        if (storeFile == null || !storeFile.isFile) {
            throw GradleException("Release signing: keystore $storeFile (storeFile in $props) does not exist.")
        }
    }
}
tasks.matching { it.name == "preReleaseBuild" }.configureEach { dependsOn(checkUploadKey) }
