plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "org.konnekta.app"
    compileSdk = 37

    defaultConfig {
        applicationId = "org.konnekta.app"
        minSdk = 28
        targetSdk = 36
        versionCode = 7
        versionName = "2.5.7"
        // The Rust core (libkonnekta_ffi.so) is built by scripts/build-android.sh.
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildFeatures { compose = true }

    // One APK per ABI as well as the universal one: smaller downloads for phones.
    splits {
        abi {
            isEnable = true
            reset()
            include("arm64-v8a", "x86_64")
            isUniversalApk = true
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    // Release key from the environment (CI secrets, see docs/RILASCIO.md);
    // without it the release APK is signed with the debug key so it still installs.
    val keystore = System.getenv("KONNEKTA_KEYSTORE")?.let(::file)?.takeIf { it.exists() }
    signingConfigs {
        if (keystore != null) create("release") {
            storeFile = keystore
            storePassword = System.getenv("KONNEKTA_KEYSTORE_PASSWORD")
            keyAlias = System.getenv("KONNEKTA_KEY_ALIAS")
            keyPassword = System.getenv("KONNEKTA_KEY_PASSWORD")
        }
    }

    buildTypes {
        release {
            // R8 drops the unused icons and code (the debug APK is ~120 MB).
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }

    testOptions { unitTests.isReturnDefaultValues = true }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-extended:1.7.8")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.core:core-ktx:1.19.1")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    implementation("androidx.biometric:biometric:1.1.0")
    // Scan and QR modes of the photo request, recognised on the phone (bundled models, offline).
    implementation("com.google.mlkit:barcode-scanning:17.3.0")
    implementation("com.google.mlkit:text-recognition:16.0.1")
    // JNA calls the C ABI of the Rust core (crates/konnekta-ffi/include/konnekta.h).
    implementation("net.java.dev.jna:jna:5.19.1@aar")
    testImplementation("junit:junit:4.13.2")
}
