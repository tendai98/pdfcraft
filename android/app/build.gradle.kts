plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// Release signing comes only from the environment (CI secrets); no key is ever committed. Without
// these variables a release build is left unsigned.
val keystorePath: String? = System.getenv("ANDROID_KEYSTORE_PATH")
val keystorePassword: String? = System.getenv("ANDROID_KEYSTORE_PASSWORD")
val keyAlias: String? = System.getenv("ANDROID_KEY_ALIAS")

android {
    namespace = "app.pdfcraft.android"
    compileSdk = 35

    defaultConfig {
        // Placeholder identity until the owner chooses one (docs/android/README.md).
        applicationId = "app.pdfcraft.android"
        minSdk = 26
        targetSdk = 35
        versionCode = 1
        versionName = "0.7.1"
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    signingConfigs {
        if (keystorePath != null && keystorePassword != null && keyAlias != null) {
            create("release") {
                storeFile = file(keystorePath)
                storePassword = keystorePassword
                this.keyAlias = keyAlias
                keyPassword = System.getenv("ANDROID_KEY_PASSWORD") ?: keystorePassword
            }
        }
    }

    buildTypes {
        getByName("release") {
            isMinifyEnabled = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("release")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    // Bytecode for Java 17 (AGP 8's floor); any JDK from 17 up builds it.
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}
