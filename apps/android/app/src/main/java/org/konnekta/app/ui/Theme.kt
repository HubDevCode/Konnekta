package org.konnekta.app.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color

/*
 * Konnekta palette. The desktop app uses the same values as CSS variables
 * (apps/desktop/ui/styles.css): change both together.
 */

@Immutable
data class Status(val ok: Color, val warn: Color, val danger: Color, val border: Color, val muted: Color)

private val LightStatus = Status(
    ok = Color(0xFF2E7D4F), warn = Color(0xFFA15C00), danger = Color(0xFFBA1A1A),
    border = Color(0xFFDDE1EC), muted = Color(0xFF5D6378),
)
private val DarkStatus = Status(
    ok = Color(0xFF7BD39B), warn = Color(0xFFF2B95C), danger = Color(0xFFFFB4AB),
    border = Color(0xFF343846), muted = Color(0xFFA0A5B8),
)

private val Light = lightColorScheme(
    primary = Color(0xFF475D92),
    onPrimary = Color(0xFFFFFFFF),
    primaryContainer = Color(0xFFDAE2FF),
    onPrimaryContainer = Color(0xFF001946),
    secondary = Color(0xFF1E8FA8),
    onSecondary = Color(0xFFFFFFFF),
    secondaryContainer = Color(0xFFCDEFF7),
    onSecondaryContainer = Color(0xFF002830),
    background = Color(0xFFF7F8FC),
    onBackground = Color(0xFF1B1E2B),
    surface = Color(0xFFF7F8FC),
    onSurface = Color(0xFF1B1E2B),
    surfaceVariant = Color(0xFFE6E8F2),
    onSurfaceVariant = Color(0xFF5D6378),
    surfaceContainerLowest = Color(0xFFFFFFFF),
    surfaceContainerLow = Color(0xFFFFFFFF),
    surfaceContainer = Color(0xFFF0F2F8),
    surfaceContainerHigh = Color(0xFFE9ECF4),
    surfaceContainerHighest = Color(0xFFE2E5EF),
    outline = Color(0xFFC3C8D6),
    outlineVariant = Color(0xFFDDE1EC),
    error = Color(0xFFBA1A1A),
)

private val Dark = darkColorScheme(
    primary = Color(0xFFB1C5FF),
    onPrimary = Color(0xFF172E60),
    primaryContainer = Color(0xFF2F4578),
    onPrimaryContainer = Color(0xFFDAE2FF),
    secondary = Color(0xFF7FD3E8),
    onSecondary = Color(0xFF003641),
    secondaryContainer = Color(0xFF1D4E5A),
    onSecondaryContainer = Color(0xFFCDEFF7),
    background = Color(0xFF121318),
    onBackground = Color(0xFFE3E4EC),
    surface = Color(0xFF121318),
    onSurface = Color(0xFFE3E4EC),
    surfaceVariant = Color(0xFF2A2D38),
    onSurfaceVariant = Color(0xFFA0A5B8),
    surfaceContainerLowest = Color(0xFF0D0E12),
    surfaceContainerLow = Color(0xFF1B1D24),
    surfaceContainer = Color(0xFF1F212A),
    surfaceContainerHigh = Color(0xFF252832),
    surfaceContainerHighest = Color(0xFF2F323D),
    outline = Color(0xFF4A4F60),
    outlineVariant = Color(0xFF343846),
    error = Color(0xFFFFB4AB),
)

val LocalStatus = staticCompositionLocalOf { LightStatus }

@Composable
fun KonnektaTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val scheme = if (dark) Dark else Light
    androidx.compose.runtime.CompositionLocalProvider(LocalStatus provides if (dark) DarkStatus else LightStatus) {
        MaterialTheme(colorScheme = scheme) {
            // Text outside a Surface/Scaffold (onboarding, feature screens,
            // lists) takes the theme's colour instead of Compose's black.
            androidx.compose.runtime.CompositionLocalProvider(
                androidx.compose.material3.LocalContentColor provides scheme.onBackground,
                content = content,
            )
        }
    }
}
