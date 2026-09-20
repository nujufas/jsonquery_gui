import java.util.Properties

plugins {
    id("com.android.application")
}

// ---------------------------------------------------------------------------
// Identity & version
// ---------------------------------------------------------------------------

/** The version the whole workspace is released as (root Cargo.toml). */
val workspaceVersion: String = run {
    val cargoToml = rootProject.file("../Cargo.toml").readText()
    val section = cargoToml.substringAfter("[workspace.package]")
    Regex("""(?m)^version\s*=\s*"([^"]+)"""").find(section)?.groupValues?.get(1)
        ?: error("no [workspace.package] version in the root Cargo.toml")
}

val versionCodeFromFile: Int = Properties()
    .apply { rootProject.file("version.properties").inputStream().use(::load) }
    .getProperty("versionCode")
    .toInt()

// ---------------------------------------------------------------------------
// Signing: env vars (CI) win over android/signing/keystore.properties (local)
// ---------------------------------------------------------------------------

val keystoreProperties = Properties().apply {
    val file = rootProject.file("signing/keystore.properties")
    if (file.exists()) file.inputStream().use(::load)
}

fun signingValue(env: String, key: String): String? =
    System.getenv(env)?.takeIf { it.isNotBlank() } ?: keystoreProperties.getProperty(key)

val releaseKeystore: File? = signingValue("ANDROID_KEYSTORE_FILE", "storeFile")
    ?.let { rootProject.file(it) }

// ---------------------------------------------------------------------------
// The Rust library
// ---------------------------------------------------------------------------

/** ABIs to build the Rust library for, e.g. `-PrustAbis=x86_64` for an emulator. */
val rustAbis: List<String> = (providers.gradleProperty("rustAbis").orNull
    ?: "arm64-v8a,armeabi-v7a,x86_64").split(",").map(String::trim)

/** Cargo profile from android/Cargo.toml: `release` (default) or `ci` (quicker to build). */
val rustProfile: String = providers.gradleProperty("rustProfile").orNull ?: "release"

val rustLibsDir = layout.buildDirectory.dir("rust/jniLibs")

val buildRust = tasks.register<Exec>("buildRust") {
    group = "build"
    description = "Compiles android/rust for ${rustAbis.joinToString()} with cargo-ndk."
    workingDir = rootProject.projectDir
    val minSdk = 24
    commandLine(
        buildList {
            addAll(listOf("cargo", "ndk", "--platform", minSdk.toString()))
            addAll(listOf("-o", rustLibsDir.get().asFile.absolutePath))
            rustAbis.forEach { addAll(listOf("-t", it)) }
            addAll(listOf("build", "--profile", rustProfile, "-p", "jsonquery_android"))
        }
    )
    inputs.dir(rootProject.file("rust/src"))
    inputs.file(rootProject.file("rust/Cargo.toml"))
    inputs.file(rootProject.file("Cargo.toml"))
    inputs.file(rootProject.file("Cargo.lock"))
    inputs.dir(rootProject.file("../crates"))
    inputs.property("abis", rustAbis)
    inputs.property("profile", rustProfile)
    outputs.dir(rustLibsDir)

    // Start from nothing: cargo-ndk only ever adds files, so libraries of an
    // earlier build (another profile, another ABI) would ride along.
    doFirst { delete(rustLibsDir) }
    // cargo-ndk copies every shared library the build produced, including
    // *host* ones cargo builds for build scripts (`slug`, via jmespath): x86-64
    // Linux code that would ship in every ABI folder, unloadable and not
    // 16 KB-aligned. Ship only our own library.
    doLast {
        fileTree(rustLibsDir) { exclude("**/libjsonquery_android.so") }.files.forEach { it.delete() }
    }
}

tasks.named("preBuild") { dependsOn(buildRust) }

// ---------------------------------------------------------------------------
// The app
// ---------------------------------------------------------------------------

android {
    namespace = "io.github.nujufas.jsonquery"
    compileSdk = 36
    ndkVersion = "29.0.14206865"

    defaultConfig {
        // Permanent once the app is on Google Play; see android/README.md.
        applicationId = "io.github.nujufas.jsonquery"
        minSdk = 24
        // Google Play requires new uploads to target Android 16 (API 36).
        targetSdk = 36
        versionCode = versionCodeFromFile
        versionName = workspaceVersion
    }

    signingConfigs {
        create("release") {
            if (releaseKeystore != null) {
                storeFile = releaseKeystore
                storePassword = signingValue("ANDROID_KEYSTORE_PASSWORD", "storePassword")
                keyAlias = signingValue("ANDROID_KEY_ALIAS", "keyAlias")
                keyPassword = signingValue("ANDROID_KEY_PASSWORD", "keyPassword")
            }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
            // Unsigned when no key is configured: fine for a local check, not for Play.
            if (releaseKeystore != null) signingConfig = signingConfigs.getByName("release")
            // Ships the native symbol table so Play Console can symbolicate crashes.
            ndk.debugSymbolLevel = "SYMBOL_TABLE"
        }
        debug {
            // Installs next to the release app instead of replacing it.
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
    }

    sourceSets.getByName("main").jniLibs.directories.add(rustLibsDir.get().asFile.path)

    packaging {
        // Uncompressed, page-aligned .so files: smaller download, and what
        // Android's 16 KB page-size requirement wants.
        jniLibs.useLegacyPackaging = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    lint {
        abortOnError = true
        warningsAsErrors = false
    }
}

dependencies {
    // 1.17 is the newest line built against compileSdk 36; 1.19 needs 37.
    // Move both together when compileSdk moves.
    implementation("androidx.core:core-ktx:1.17.0")
}
