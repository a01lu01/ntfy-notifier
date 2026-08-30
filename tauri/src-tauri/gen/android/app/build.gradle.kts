import com.android.build.api.instrumentation.FramesComputationMode
import com.android.build.api.instrumentation.InstrumentationScope
import java.util.Properties

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("rust")
}

val jacksonApi24CompatVersion = "2.15.3"
configurations.configureEach {
    incoming.afterResolve {
        val resolvedJacksonVersions = resolutionResult.allComponents
            .mapNotNull { it.moduleVersion }
            .filter { it.group == "com.fasterxml.jackson.core" }
            .associate { it.name to it.version }
        check(resolvedJacksonVersions.values.all { it == jacksonApi24CompatVersion }) {
            "Jackson API 24 compatibility was validated only for $jacksonApi24CompatVersion, " +
                "but this configuration resolved $resolvedJacksonVersions; review the ASM transform"
        }
    }
}

val tauriProperties = Properties().apply {
    val propFile = file("tauri.properties")
    if (propFile.exists()) {
        propFile.inputStream().use { load(it) }
    }
}

android {
    compileSdk = 36
    namespace = "com.why.ntfy_notifier"
    defaultConfig {
        manifestPlaceholders["usesCleartextTraffic"] = "false"
        applicationId = "com.why.ntfy_notifier"
        minSdk = 24
        targetSdk = 36
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        versionCode = tauriProperties.getProperty("tauri.android.versionCode", "1").toInt()
        versionName = tauriProperties.getProperty("tauri.android.versionName", "1.0")
    }
    buildTypes {
        getByName("debug") {
            manifestPlaceholders["usesCleartextTraffic"] = "true"
            isDebuggable = true
            isJniDebuggable = true
            isMinifyEnabled = false
        }
        getByName("release") {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("debug")
        }
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
    buildFeatures {
        buildConfig = true
    }
    testOptions {
        animationsDisabled = true
    }
}

androidComponents {
    onVariants(selector().all()) { variant ->
        variant.instrumentation.transformClassesWith(
            JacksonApi24CompatVisitorFactory::class.java,
            InstrumentationScope.ALL,
        ) {}
        variant.instrumentation.setAsmFramesComputationMode(FramesComputationMode.COPY_FRAMES)
    }
}

rust {
    rootDirRel = "../../../"
}

dependencies {
    implementation("androidx.webkit:webkit:1.14.0")
    implementation("androidx.appcompat:appcompat:1.7.1")
    implementation("androidx.activity:activity-ktx:1.10.1")
    implementation("com.google.android.material:material:1.12.0")
    implementation("androidx.lifecycle:lifecycle-process:2.10.0")
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation(
        "com.fasterxml.jackson.core:jackson-databind:$jacksonApi24CompatVersion"
    )
    androidTestImplementation("androidx.test.ext:junit:1.1.4")
    androidTestImplementation("androidx.test:runner:1.5.0")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.5.0")
}

apply(from = "tauri.build.gradle.kts")
