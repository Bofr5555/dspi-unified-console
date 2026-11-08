// :ui-common — theme, shared components ported from macOS CustomSlider.swift,
// ChannelPalette.swift, Components.swift and the Linux qt/qml/components/ set,
// the route table, the app's one preference store (AppSettings over
// DataStore, decision F3) and the shared app state. Owned by package HOME;
// other UI packages request additions. No dependency on :bridge (pure UI +
// presentation logic).
plugins {
    id("dspi.android-compose")
}

android {
    namespace = "se.bofr.dspi.ui"
    // -- WP AN20 -- Robolectric reads the merged manifest and resources (as :feature-settings, AN9)
    testOptions { unitTests.isIncludeAndroidResources = true }
}

// -- WP AN20 -- the Android runtime Robolectric runs on, resolved (checksum-pinned) so it stays offline (as :app)
val robolectricRuntime: Configuration by configurations.creating

dependencies {
    implementation(libs.kotlinx.coroutines.core)
    implementation(libs.androidx.datastore.preferences)
    testImplementation(libs.kotlinx.coroutines.test)
    // -- WP AN20 -- semantics tests of the shared controls, with the Accessibility Test Framework checks
    testImplementation(platform(libs.compose.bom))
    testImplementation(libs.compose.ui.test.junit4)
    testImplementation(libs.compose.ui.test.junit4.accessibility)
    testImplementation(libs.robolectric)
    debugImplementation(platform(libs.compose.bom))
    debugImplementation(libs.compose.ui.test.manifest)
    robolectricRuntime(libs.robolectric.android.all)
    // -- WP AN20 -- the version :app resolves (and verification-metadata.xml pins), as :feature-settings
    constraints { testImplementation("androidx.tracing:tracing:1.2.0") }
}

// -- WP AN20 -- as :app: Robolectric offline
tasks.withType<Test>().configureEach {
    val runtime = robolectricRuntime
    inputs.files(runtime)
    systemProperty("robolectric.offline", "true")
    doFirst { systemProperty("robolectric.dependency.dir", runtime.singleFile.parentFile.path) }
}

// -- WP AN20 -- AtfChecks, shared with the feature modules' tests as an extra test source directory
android.sourceSets["test"].kotlin.srcDir("src/a11yTest/kotlin")
