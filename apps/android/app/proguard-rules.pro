# :app R8 rules (WP AN15). The release build is minified and resource-shrunk
# (app/build.gradle.kts). Library rules travel with their libraries: UniFFI and
# JNA in :ffi (ffi/consumer-rules.pro); AndroidX, Compose and coroutines ship
# their own. The app uses no kotlinx.serialization, and no reflection beyond
# what those rules cover (manifest components are kept by AAPT's rules).

# Readable stack traces: keep line numbers, hide the source file names. Play
# retraces crash reports with the mapping.txt that the .aab carries
# (BUNDLE-METADATA/com.android.tools.build.obfuscation/proguard.map).
-keepattributes SourceFile,LineNumberTable
-renamesourcefileattribute SourceFile
