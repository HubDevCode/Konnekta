# JNA binds the core's C functions by name and reflects on its own classes.
-keep class com.sun.jna.** { *; }
-keep interface org.konnekta.app.core.KonnektaLib { *; }
-dontwarn java.awt.**
-dontwarn com.sun.jna.**
