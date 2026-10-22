# JNI names are an ABI, including callbacks owned by shared BLE pumps.
-keep class rs.reticulum.prns.expo.PrnsBluetoothNative { *; }
-keep class rs.reticulum.prns.expo.PrnsBluetoothLifecycle { *; }
-keep class rs.reticulum.prns.expo.PrnsLanNative { *; }
-keep class rs.reticulum.prns.expo.PrnsLanSessionNative { *; }

# UniFFI JNA interfaces and structure fields are discovered by reflection.
-keep class rs.reticulum.prns.host.bindings.** { *; }
-keep class com.sun.jna.** { *; }
-keep class * extends com.sun.jna.Structure { public *; }
-dontwarn java.awt.**
