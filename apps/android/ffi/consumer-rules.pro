# :ffi consumer rules (WP AN15): what R8 must keep for UniFFI's Kotlin bindings
# (build/generated/uniffi/**, package se.bofr.dspi.ffi.uniffi) to reach
# libdspi_ffi through JNA. Applied to every app that depends on :ffi.
# Only the parts JNA touches by reflection or from native code are kept; the
# rest of the generated file (records, converters, objects) is shrunk and
# renamed like any other code.

# JNA itself: libjnidispatch looks up its classes, fields and methods by name
# over JNI (Native, Pointer, Structure, CallbackReference, ...).
-keep class com.sun.jna.** { *; }
# JNA's desktop-only paths (AWT window handles); not on Android.
-dontwarn java.awt.**

# Library interfaces (UniffiLib, IntegrityCheckingUniffiLib): JNA proxies them,
# and each method name is the exported C symbol it binds to.
-keep interface * extends com.sun.jna.Library { *; }

# Structures (RustBuffer, UniffiRustCallStatus, ForeignBytes, the callback
# vtables, UniffiForeignFuture*): @Structure.FieldOrder names the fields, JNA
# reads and writes them by name, and creates instances (ByValue, ByReference)
# through their constructors.
-keepattributes RuntimeVisibleAnnotations,AnnotationDefault
-keep class * extends com.sun.jna.Structure { <fields>; <init>(...); }

# Callbacks (the Rust-future continuation, the callback-interface vtable
# entries: HostTransport, DeviceEvents, LinkListener, RtaListener, ...): JNA
# finds the interface that extends Callback and its one method by reflection,
# on the interface and on the Kotlin objects that implement it.
-keep interface * extends com.sun.jna.Callback { *; }
-keep class * implements com.sun.jna.Callback { *; }

# Types JNA instantiates for arguments and return values.
-keep class * extends com.sun.jna.IntegerType { <init>(...); }
-keep class * extends com.sun.jna.PointerType { <init>(...); }
