package fail.golem.testk

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

/** The Compose Multiplatform counter screen, shared by Android and iOS. */
@Composable
fun CounterScreen() {
    var count by remember { mutableStateOf(0) }
    MaterialTheme {
        Column(
            modifier = Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Text("Compose Counter", style = MaterialTheme.typography.headlineMedium)
            // No contentDescription: on iOS, Compose Multiplatform reports a
            // contentDescription as the label in place of the text, which
            // would hide the count's value from text selectors.
            Text("$count")
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Button(
                    onClick = { count++ },
                    modifier = Modifier.semantics { contentDescription = "Increment" },
                ) { Text("+") }
                Button(
                    onClick = { count-- },
                    modifier = Modifier.semantics { contentDescription = "Decrement" },
                ) { Text("-") }
            }
        }
    }
}

/**
 * Android's whole app. iOS starts on a native SwiftUI home screen instead
 * (iosApp/iosApp/ContentView.swift) and opens [CounterScreen] from there.
 */
@Composable
fun App() {
    var showCounter by remember { mutableStateOf(false) }
    if (showCounter) {
        CounterScreen()
    } else {
        MaterialTheme {
            Column(
                modifier = Modifier.fillMaxSize().safeDrawingPadding().padding(16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("Home", style = MaterialTheme.typography.headlineMedium)
                Button(onClick = { showCounter = true }) { Text("Open counter") }
            }
        }
    }
}
