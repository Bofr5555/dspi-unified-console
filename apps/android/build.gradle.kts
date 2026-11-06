// Root project: no code. Module conventions live in buildSrc
// (dspi.jvm-library, dspi.android-library, dspi.android-compose,
// dspi.android-app, dspi.ktlint).

// No tmpfs for tests (the JVM's default temp dir is /tmp): TemporaryFolder and
// createTempDirectory land in each module's build directory.
subprojects {
    tasks.withType<Test>().configureEach {
        val dir = layout.buildDirectory.dir("tmp/test-java-io")
        systemProperty("java.io.tmpdir", dir.get().asFile.path)
        doFirst { dir.get().asFile.mkdirs() }
    }
}
