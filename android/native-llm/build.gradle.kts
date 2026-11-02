plugins {
    alias(libs.plugins.android.library)
}

android {
    namespace = "anomaly.hex.android.nativellm"
    compileSdk = 37
    ndkVersion = "29.0.14206865"

    defaultConfig {
        minSdk = 33
        ndk { abiFilters += (providers.gradleProperty("hexAbis").orNull ?: "arm64-v8a,x86_64").split(",") }
        externalNativeBuild {
            cmake {
                arguments += listOf("-DANDROID_STL=c++_shared", "-DCMAKE_BUILD_TYPE=Release")
                targets += "hexllm"
            }
        }
    }

    externalNativeBuild {
        cmake {
            path = file("src/main/cpp/CMakeLists.txt")
            version = "3.31.6"
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildTypes {
        // The inference loops are unusably slow without optimisation, even under debug.
        debug { externalNativeBuild { cmake { arguments += "-DCMAKE_BUILD_TYPE=Release" } } }
    }
}
