plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
}

// The version (ADR-0029): the node crate's (vork), a release's when
// this commit is tagged with it, else a build towards it; the commit count is
// the versionCode, so each build installs over the one before.
// clients/android/ in the vork repository.
val repo = rootProject.projectDir.parentFile.parentFile
fun git(vararg args: String): String = try {
    ProcessBuilder(listOf("git", "-C", repo.path) + args).redirectErrorStream(true).start().inputStream.bufferedReader().readText().trim()
} catch (e: Exception) {
    ""
}
val workspaceVersion = File(repo, "Cargo.toml").readLines().dropWhile { it.trim() != "[package]" }.first { it.startsWith("version = ") }.substringAfter('"').substringBefore('"')
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

    // The window is the node's web dashboard (src/web/index.html), the same
    // page the desktop app and a server's browser show.
    sourceSets["main"].assets.srcDir(File(repo, "src/web"))

    buildTypes {
        // Signed with the debug key for now: for testers, not a store.
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
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
    implementation(libs.androidx.activity.ktx)
    // Scanning another device's pairing code inside the app: ZXing (open source,
    // Apache-2.0, decodes on the phone; no Play services, nothing leaves it).
    implementation("com.journeyapps:zxing-android-embedded:4.3.0")
}
