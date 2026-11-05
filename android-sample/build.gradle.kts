// AGP 9 は Kotlin を内蔵する（kotlin-android プラグインは不要）。内蔵の Kotlin のバージョンは、
// ここで Kotlin Gradle プラグインを明示して固定する（依存の更新は Dependabot が提案する）。
buildscript {
    repositories { mavenCentral() }
    dependencies { classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.4.21") }
}

plugins {
    id("com.android.application") version "9.5.0" apply false
}
