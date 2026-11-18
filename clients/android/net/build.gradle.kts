plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

// The version (vork docs/storage/versioning.md): the workspace's, a release's when
// this commit is tagged with it, else a build towards it; the commit count is
// the versionCode, so each build installs over the one before.
// clients/android/ in the vork repository.
val repo = rootProject.projectDir.parentFile.parentFile
fun git(vararg args: String): String = try {
    ProcessBuilder(listOf("git", "-C", repo.path) + args).redirectErrorStream(true).start().inputStream.bufferedReader().readText().trim()
} catch (e: Exception) {
    ""
}
val workspaceVersion = File(repo, "Cargo.toml").readLines().first { it.startsWith("version = ") }.substringAfter('"').substringBefore('"')
val commits = git("rev-list", "--count", "HEAD").toIntOrNull() ?: 1
val released = git("describe", "--exact-match", "--tags", "HEAD") == "v$workspaceVersion"

android {
    namespace = "org.vork.net"
    compileSdk = 34

    defaultConfig {
        applicationId = "org.vork.net"
        minSdk = 26
        targetSdk = 34
        versionCode = commits
        versionName = if (released) workspaceVersion else "$workspaceVersion-dev.$commits"
        // The node is built for 64-bit ARM phones (and x86_64 emulators).
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    buildFeatures { compose = true }

    buildTypes {
        // Signed with the debug key for now: for testers, not a store.
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    // The node runs as a program from the native library folder (Android
    // lets an app execute its native libraries, not files it writes): it
    // must be extracted there, not left in the APK.
    packaging { jniLibs { useLegacyPackaging = true } }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions { jvmTarget = "17" }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(platform(libs.androidx.compose.bom))
    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material.icons)
    // Scanning another device's pairing code inside the app: ZXing (open source,
    // Apache-2.0, decodes on the phone; no Play services, nothing leaves it).
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
}
