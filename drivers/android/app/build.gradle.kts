import org.gradle.api.tasks.testing.logging.TestExceptionFormat

plugins {
    id("com.android.application")
}

android {
    namespace = "com.lahfir.agentmobile.driver"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.lahfir.agentmobile.driver"
        minSdk = 30
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
    }

    sourceSets {
        getByName("test") {
            resources.srcDirs(
                "../../../crates/core/tests/spec",
                "../../../crates/core/tests/fixtures",
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

dependencyLocking {
    lockAllConfigurations()
}

dependencies {
    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20250517")
}

tasks.withType<Test>().configureEach {
    testLogging {
        exceptionFormat = TestExceptionFormat.FULL
        showExceptions = true
        showCauses = true
        showStackTraces = true
    }
}
