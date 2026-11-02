import java.net.URI
import java.security.MessageDigest
import java.util.Properties

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
    alias(libs.plugins.kotlin.serialization)
}

val localProperties = Properties().apply {
    rootProject.file("local.properties").takeIf { it.isFile }?.inputStream()?.use(::load)
}

android {
    namespace = "anomaly.hex.android"
    compileSdk = 37
    ndkVersion = "29.0.14206865"

    defaultConfig {
        applicationId = "anomaly.hex.android"
        minSdk = 33
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += (providers.gradleProperty("hexAbis").orNull ?: "arm64-v8a,x86_64").split(",") }
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    // Local signing key, never in git. Without one, release falls back to the debug key.
    val keystore = localProperties.getProperty("hex.keystore")
    signingConfigs {
        if (keystore != null) {
            create("hex") {
                storeFile = file(keystore)
                storePassword = localProperties.getProperty("hex.keystore.password")
                keyAlias = localProperties.getProperty("hex.key.alias")
                keyPassword = localProperties.getProperty("hex.keystore.password")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            signingConfig = signingConfigs.findByName("hex") ?: signingConfigs.getByName("debug")
        }
        // Release code plus the debug hooks: unshrunk Compose takes many minutes to install on a
        // software emulator, and tests should run the same optimised code the phone gets.
        create("emulator") {
            initWith(getByName("release"))
            isDebuggable = true
            signingConfig = signingConfigs.getByName("debug")
            matchingFallbacks += "release"
        }
    }

    sourceSets.getByName("emulator") {
        kotlin.directories += "src/debug/kotlin"
        res.directories += "src/debug/res"
        manifest.srcFile("src/debug/AndroidManifest.xml")
    }

    buildFeatures {
        compose = true
        aidl = true
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        // Uncompressed and page-aligned: the engine process maps these directly.
        jniLibs.useLegacyPackaging = false
    }

    testOptions.unitTests.isReturnDefaultValues = true
}

// The speech detector is small enough to ship, so a first dictation can't type invented text,
// but it isn't committed: it's fetched at a pinned revision and digest.
abstract class FetchSpeechDetector : DefaultTask() {
    @get:OutputDirectory abstract val output: DirectoryProperty

    @TaskAction fun fetch() {
        val file = output.file("silero-vad-v6.2-F32.gguf").get().asFile
        val digest = "0d871a800c420fdd5d1b17a3aea4296d1c39608f0ff4a9295eb76457eb1c88e6"
        if (!file.isFile) {
            URI("https://huggingface.co/handy-computer/silero-vad-v6.2-gguf/resolve/a9b2d61c20ae4ddf3644edb34969e8185a485448/silero-vad-v6.2-F32.gguf")
                .toURL().openStream().use { input -> file.outputStream().use { input.copyTo(it) } }
        }
        val actual = MessageDigest.getInstance("SHA-256").digest(file.readBytes()).joinToString("") { "%02x".format(it) }
        check(actual == digest) { "Speech detector digest mismatch: $actual" }
    }
}

val fetchSpeechDetector = tasks.register<FetchSpeechDetector>("fetchSpeechDetector") {
    output.set(layout.buildDirectory.dir("generated/speechDetector"))
}

androidComponents.onVariants { variant ->
    variant.sources.assets?.addGeneratedSourceDirectory(fetchSpeechDetector, FetchSpeechDetector::output)
}

dependencies {
    implementation(project(":core"))
    implementation(project(":native-asr"))
    implementation(project(":native-llm"))
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.material3)
    implementation(libs.compose.icons)
    implementation(libs.androidx.core)
    implementation(libs.activity.compose)
    implementation(libs.lifecycle.runtime)
    testImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.core)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.runner)
}
