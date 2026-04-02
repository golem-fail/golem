package com.golem.testb

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    TestScreen()
                }
            }
        }
    }
}

@Composable
fun TestScreen() {
    var counter by remember { mutableStateOf(0) }
    var status by remember { mutableStateOf("Ready") }
    var toggleOn by remember { mutableStateOf(false) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        Text(
            "GOLEM Test B",
            fontSize = 28.sp,
            modifier = Modifier.semantics { contentDescription = "app-b-title" }
        )

        Text(
            status,
            modifier = Modifier.semantics { contentDescription = "status-label" }
        )

        Text(
            "Shared Data",
            modifier = Modifier.semantics { contentDescription = "shared-data-display" }
        )

        Button(
            onClick = { status = "Refreshed" },
            modifier = Modifier.semantics { contentDescription = "refresh-button" }
        ) {
            Text("Refresh")
        }

        Divider()

        // Counter for accessibility_id testing
        Text(
            "$counter",
            fontSize = 24.sp,
            modifier = Modifier.semantics { contentDescription = "counter-b" }
        )

        Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            Button(
                onClick = { counter++ },
                modifier = Modifier.semantics { contentDescription = "increment-b" }
            ) {
                Text("Increment")
            }

            Button(
                onClick = { counter-- },
                modifier = Modifier.semantics { contentDescription = "decrement-b" }
            ) {
                Text("Decrement")
            }
        }

        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Text("Test Toggle")
            Switch(
                checked = toggleOn,
                onCheckedChange = { toggleOn = it },
                modifier = Modifier.semantics { contentDescription = "toggle-b" }
            )
        }
    }
}
